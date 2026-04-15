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

use super::app::App;

/// Build a pretty-printed JSON representation of the request for review.
///
/// We construct this manually because `CreateOperatingSystemRequest` doesn't
/// derive `serde::Serialize`.
pub fn build_review_json(app: &App) -> String {
    let form = &app.form;
    let template = app.selected_template.as_ref().expect("template selected");

    let mut obj = serde_json::Map::new();

    obj.insert("name".into(), serde_json::Value::String(form.name.clone()));

    if !form.description.is_empty() {
        obj.insert(
            "description".into(),
            serde_json::Value::String(form.description.clone()),
        );
    }

    obj.insert(
        "tenant_organization_id".into(),
        serde_json::Value::String(form.tenant_organization_id.clone()),
    );

    obj.insert("is_active".into(), serde_json::Value::Bool(form.is_active));
    obj.insert(
        "allow_override".into(),
        serde_json::Value::Bool(form.allow_override),
    );
    obj.insert(
        "phone_home_enabled".into(),
        serde_json::Value::Bool(form.phone_home_enabled),
    );

    if let Some(ref content) = form.user_data_content {
        // Truncate for display if very long
        let display = if content.len() > 500 {
            format!("{}... ({} bytes total)", &content[..500], content.len())
        } else {
            content.clone()
        };
        obj.insert("user_data".into(), serde_json::Value::String(display));
    }

    if let Some(ref id) = template.id {
        obj.insert(
            "ipxe_template_id".into(),
            serde_json::Value::String(id.to_string()),
        );
    }

    obj.insert(
        "ipxe_template_name".into(),
        serde_json::Value::String(template.name.clone()),
    );

    // Parameters
    let params: Vec<serde_json::Value> = form
        .required_params
        .iter()
        .chain(form.optional_params.iter())
        .filter(|p| !p.value.is_empty())
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "value": p.value,
            })
        })
        .collect();
    if !params.is_empty() {
        obj.insert(
            "ipxe_template_parameters".into(),
            serde_json::Value::Array(params),
        );
    }

    // Artifacts
    let artifacts: Vec<serde_json::Value> = form
        .required_artifacts
        .iter()
        .chain(form.optional_artifacts.iter())
        .filter(|a| !a.url.is_empty())
        .map(|a| {
            let mut aobj = serde_json::Map::new();
            aobj.insert("name".into(), serde_json::Value::String(a.name.clone()));
            aobj.insert("url".into(), serde_json::Value::String(a.url.clone()));
            if !a.sha.is_empty() {
                aobj.insert("sha".into(), serde_json::Value::String(a.sha.clone()));
            }
            aobj.insert(
                "auth_type".into(),
                serde_json::Value::String(a.auth_type.as_str().into()),
            );
            if !a.auth_token.is_empty() {
                aobj.insert("auth_token".into(), serde_json::Value::String("***".into()));
            }
            aobj.insert(
                "cache_strategy".into(),
                serde_json::Value::String(a.cache_strategy.as_str().into()),
            );
            serde_json::Value::Object(aobj)
        })
        .collect();
    if !artifacts.is_empty() {
        obj.insert(
            "ipxe_template_artifacts".into(),
            serde_json::Value::Array(artifacts),
        );
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .unwrap_or_else(|_| "Error building JSON preview".into())
}
