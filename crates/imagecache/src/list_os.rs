/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Lists every operating system definition along with each of its iPXE
//! template artifacts (URL, cached URL, SHA, cache strategy, auth).
//!
//! Required environment variables:
//!   CARBIDE_API_INTERNAL_URL  (or defaults to https://carbide-api.forge-system.svc.cluster.local:1079)
//!   FORGE_ROOT_CAFILE_PATH
//!   FORGE_CLIENT_CERT_PATH
//!   FORGE_CLIENT_KEY_PATH
//!
//! Optional environment variables:
//!   CARBIDE_TENANT_ORGANIZATION_ID  (restrict listing to a single tenant)

use std::collections::HashSet;
use std::env;

use forge_tls::client_config::ClientCert;
use rpc::forge::{
    IpxeTemplateArtifact, IpxeTemplateArtifactCacheStrategy, OperatingSystem,
    OperatingSystemSearchFilter, OperatingSystemType, OperatingSystemsByIdsRequest, TenantState,
};
use rpc::forge_api_client::ForgeApiClient;
use rpc::forge_tls_client::{ApiConfig, ForgeClientConfig};

fn env_required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let internal_api_url = env::var("CARBIDE_API_INTERNAL_URL")
        .unwrap_or_else(|_| "https://carbide-api.forge-system.svc.cluster.local:1079".to_string());
    let tenant_filter = env::var("CARBIDE_TENANT_ORGANIZATION_ID").ok();

    let client_cert = Some(ClientCert {
        cert_path: env_required("FORGE_CLIENT_CERT_PATH"),
        key_path: env_required("FORGE_CLIENT_KEY_PATH"),
    });
    let client_config = ForgeClientConfig::new(env_required("FORGE_ROOT_CAFILE_PATH"), client_cert);

    let api = ForgeApiClient::new(&ApiConfig::new(&internal_api_url, &client_config));

    let os_ids = api
        .find_operating_system_ids(OperatingSystemSearchFilter {
            tenant_organization_id: tenant_filter.clone(),
        })
        .await?;

    if os_ids.ids.is_empty() {
        if let Some(t) = &tenant_filter {
            println!("No operating system definitions found for tenant {t}.");
        } else {
            println!("No operating system definitions found.");
        }
        return Ok(());
    }

    let os_defs = api
        .find_operating_systems_by_ids(OperatingSystemsByIdsRequest { ids: os_ids.ids })
        .await?;

    let mut operating_systems = os_defs.operating_systems;
    operating_systems.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let total_os = operating_systems.len();
    let mut total_artifacts = 0usize;
    let mut total_cached = 0usize;
    let mut unique_cached: HashSet<&str> = HashSet::new();

    println!(
        "Found {total_os} operating system definition{}.",
        if total_os == 1 { "" } else { "s" }
    );

    for (idx, os) in operating_systems.iter().enumerate() {
        print_separator();
        print_os_header(idx + 1, total_os, os);
        print_os_fields(os);
        let (arts, cached) = print_artifacts(&os.ipxe_template_artifacts);
        total_artifacts += arts;
        total_cached += cached;
        for art in &os.ipxe_template_artifacts {
            if let Some(url) = art.cached_url.as_deref().filter(|u| !u.is_empty()) {
                unique_cached.insert(url);
            }
        }
    }

    print_separator();
    println!(
        "Total: {total_os} OS definition{}, {total_artifacts} artifact{} ({total_cached} cached, {} unique).",
        if total_os == 1 { "" } else { "s" },
        if total_artifacts == 1 { "" } else { "s" },
        unique_cached.len(),
    );

    Ok(())
}

fn print_separator() {
    println!("{}", "─".repeat(78));
}

fn print_os_header(index: usize, total: usize, os: &OperatingSystem) {
    println!("[{index}/{total}] {}", os.name);
}

fn print_os_fields(os: &OperatingSystem) {
    let id_str = os
        .id
        .as_ref()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "<unset>".to_string());
    println!("  ID:               {id_str}");
    println!("  Type:             {}", os_type_label(os.r#type));
    println!("  Tenant:           {}", os.tenant_organization_id);
    println!("  Status:           {}", status_label(os.status));
    println!(
        "  Active:           {}   Allow override: {}   Phone home: {}",
        os.is_active, os.allow_override, os.phone_home_enabled
    );
    println!("  Created:          {}", os.created);
    println!("  Updated:          {}", os.updated);

    if let Some(desc) = os.description.as_ref().filter(|s| !s.is_empty()) {
        println!("  Description:      {desc}");
    }
    if let Some(template_id) = os.ipxe_template_id.as_ref() {
        println!("  Template ID:      {template_id}");
    }
    if let Some(hash) = os
        .ipxe_template_definition_hash
        .as_ref()
        .filter(|s| !s.is_empty())
    {
        println!("  Template hash:    {hash}");
    }
    if !os.ipxe_template_parameters.is_empty() {
        println!("  Parameters:");
        for p in &os.ipxe_template_parameters {
            let value = if p.value.is_empty() {
                "<empty>"
            } else {
                &p.value
            };
            println!("    - {} = {value}", p.name);
        }
    }
    if let Some(user_data) = os.user_data.as_ref().filter(|s| !s.is_empty()) {
        let lines = user_data.lines().count();
        let bytes = user_data.len();
        println!("  user_data:        {lines} line(s), {bytes} byte(s)");
    }
}

fn print_artifacts(artifacts: &[IpxeTemplateArtifact]) -> (usize, usize) {
    if artifacts.is_empty() {
        println!("  Artifacts:        (none)");
        return (0, 0);
    }

    let cached_count = artifacts
        .iter()
        .filter(|a| a.cached_url.as_ref().is_some_and(|u| !u.is_empty()))
        .count();
    println!("  Artifacts ({}): {cached_count} cached", artifacts.len());

    for (i, art) in artifacts.iter().enumerate() {
        println!("    [{}] {}", i + 1, art.name);
        println!("        URL:          {}", art.url);
        match art.cached_url.as_deref() {
            Some(u) if !u.is_empty() => println!("        Cached URL:   {u}"),
            _ => println!("        Cached URL:   <not cached>"),
        }
        match art.sha.as_deref() {
            Some(s) if !s.is_empty() => println!("        SHA256:       {s}"),
            _ => println!("        SHA256:       <none>"),
        }
        println!(
            "        Cache mode:   {}",
            cache_strategy_label(art.cache_strategy)
        );
        let auth = art.auth_type.as_deref().unwrap_or("None");
        let token_present = art.auth_token.as_ref().is_some_and(|t| !t.is_empty());
        if auth.eq_ignore_ascii_case("None") || auth.is_empty() {
            println!("        Auth:         None");
        } else {
            println!(
                "        Auth:         {auth} (token {})",
                if token_present { "set" } else { "missing" }
            );
        }
    }

    (artifacts.len(), cached_count)
}

fn os_type_label(value: i32) -> &'static str {
    match OperatingSystemType::try_from(value) {
        Ok(OperatingSystemType::OsTypeUnspecified) => "UNSPECIFIED",
        Ok(OperatingSystemType::OsTypeIpxe) => "IPXE (inline script)",
        Ok(OperatingSystemType::OsTypeTemplatedIpxe) => "TEMPLATED_IPXE",
        Err(_) => "UNKNOWN",
    }
}

fn status_label(value: i32) -> &'static str {
    match TenantState::try_from(value) {
        Ok(TenantState::Provisioning) => "PROVISIONING",
        Ok(TenantState::Ready) => "READY",
        Ok(TenantState::Configuring) => "CONFIGURING",
        Ok(TenantState::Terminating) => "TERMINATING",
        Ok(TenantState::Terminated) => "TERMINATED",
        Ok(_) => "OTHER",
        Err(_) => "UNKNOWN",
    }
}

fn cache_strategy_label(value: i32) -> &'static str {
    match IpxeTemplateArtifactCacheStrategy::try_from(value) {
        Ok(IpxeTemplateArtifactCacheStrategy::CacheAsNeeded) => "CACHE_AS_NEEDED",
        Ok(IpxeTemplateArtifactCacheStrategy::LocalOnly) => "LOCAL_ONLY",
        Ok(IpxeTemplateArtifactCacheStrategy::CachedOnly) => "CACHED_ONLY",
        Ok(IpxeTemplateArtifactCacheStrategy::RemoteOnly) => "REMOTE_ONLY",
        Err(_) => "UNKNOWN",
    }
}
