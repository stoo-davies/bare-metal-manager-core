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

//! Mark-and-sweep garbage collection for cached artifacts.
//!
//! When an OS definition is deleted, its cached artifacts are no longer
//! referenced but remain in storage. This module periodically scans storage
//! and deletes any object whose key is not referenced by the `cached_url` of
//! any current artifact.
//!
//! Safety rails (see `run_once`):
//!   - If the API returns zero OS definitions we abort — a transient API
//!     outage must not be interpreted as "nothing is referenced, delete all."
//!   - Only keys matching the cache key format (see `keys::is_valid_cache_key`)
//!     are eligible for deletion, so stray files in the bucket are ignored.
//!   - Objects younger than `grace_period` are skipped to avoid racing with
//!     an in-progress upload or a freshly-created OS not yet polled.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use rpc::forge::OperatingSystem;
use tracing::{info, warn};

use crate::api_client::ApiClient;
use crate::config::RuntimeConfig;
use crate::error::ImageCacheError;
use crate::keys::{is_valid_cache_key, key_from_cached_url};
use crate::storage::{StorageBackend, StoredObject};

pub async fn run_once<S: StorageBackend>(
    api: &ApiClient,
    storage: &S,
    config: &RuntimeConfig,
) -> Result<(), ImageCacheError> {
    info!(dry_run = config.gc.dry_run, "Starting GC cycle");

    let os_ids = api.discover_os_ids(config.tenant_filter.clone()).await?;
    if os_ids.is_empty() {
        warn!("GC: API returned zero OS definitions, aborting sweep");
        return Ok(());
    }
    let os_defs = api.get_os_definitions(os_ids).await?;
    let live_keys = build_live_set(&os_defs);

    let objects = storage.list_keys().await?;
    let total_objects = objects.len();

    let now = SystemTime::now();
    let orphans = compute_orphans(&live_keys, &objects, config.gc.grace_period, now);

    let mut deleted = 0u32;
    let mut errors = 0u32;
    for key in &orphans {
        if config.gc.dry_run {
            info!(key, "GC dry-run: would delete orphan");
            continue;
        }
        match storage.delete_object(key).await {
            Ok(()) => {
                info!(key, "GC: deleted orphan");
                deleted += 1;
            }
            Err(e) => {
                warn!(key, error = %e, "GC: failed to delete orphan");
                errors += 1;
            }
        }
    }

    info!(
        total_objects,
        live_keys = live_keys.len(),
        orphans = orphans.len(),
        deleted,
        errors,
        dry_run = config.gc.dry_run,
        "GC cycle complete"
    );
    Ok(())
}

/// Collects the set of storage keys referenced by any artifact's `cached_url`.
fn build_live_set(os_defs: &[OperatingSystem]) -> HashSet<String> {
    let mut live = HashSet::new();
    for os in os_defs {
        for art in &os.ipxe_template_artifacts {
            if let Some(url) = &art.cached_url
                && !url.is_empty()
            {
                live.insert(key_from_cached_url(url).to_string());
            }
        }
    }
    live
}

/// Returns the keys that should be deleted: stored objects that are not in
/// `live`, match the cache key format, and are older than `grace_period`.
fn compute_orphans(
    live: &HashSet<String>,
    stored: &[StoredObject],
    grace_period: Duration,
    now: SystemTime,
) -> Vec<String> {
    stored
        .iter()
        .filter(|obj| !live.contains(&obj.key))
        .filter(|obj| is_valid_cache_key(&obj.key))
        .filter(|obj| {
            now.duration_since(obj.last_modified)
                .map(|age| age >= grace_period)
                .unwrap_or(false)
        })
        .map(|obj| obj.key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn obj(key: &str, age: Duration, now: SystemTime) -> StoredObject {
        StoredObject {
            key: key.to_string(),
            last_modified: now - age,
        }
    }

    #[test]
    fn orphan_is_deleted() {
        let now = SystemTime::now();
        let live = HashSet::new();
        let key = format!("{}.iso", sha('a'));
        let stored = vec![obj(&key, Duration::from_secs(3600 * 48), now)];
        let orphans = compute_orphans(&live, &stored, Duration::from_secs(86400), now);
        assert_eq!(orphans, vec![key]);
    }

    #[test]
    fn live_key_is_kept() {
        let now = SystemTime::now();
        let key = format!("{}.iso", sha('a'));
        let live: HashSet<String> = [key.clone()].into_iter().collect();
        let stored = vec![obj(&key, Duration::from_secs(3600 * 48), now)];
        let orphans = compute_orphans(&live, &stored, Duration::from_secs(86400), now);
        assert!(orphans.is_empty());
    }

    #[test]
    fn young_orphan_is_kept_during_grace_period() {
        let now = SystemTime::now();
        let live = HashSet::new();
        let key = format!("{}.iso", sha('a'));
        let stored = vec![obj(&key, Duration::from_secs(3600), now)];
        let orphans = compute_orphans(&live, &stored, Duration::from_secs(86400), now);
        assert!(orphans.is_empty());
    }

    #[test]
    fn orphan_at_grace_boundary_is_deleted() {
        let now = SystemTime::now();
        let live = HashSet::new();
        let key = format!("{}.iso", sha('a'));
        let stored = vec![obj(&key, Duration::from_secs(86400), now)];
        let orphans = compute_orphans(&live, &stored, Duration::from_secs(86400), now);
        assert_eq!(orphans.len(), 1);
    }

    #[test]
    fn non_cache_key_files_are_ignored() {
        let now = SystemTime::now();
        let live = HashSet::new();
        let stored = vec![
            obj("readme.txt", Duration::from_secs(86400 * 30), now),
            obj("someone-else-file", Duration::from_secs(86400 * 30), now),
        ];
        let orphans = compute_orphans(&live, &stored, Duration::from_secs(86400), now);
        assert!(
            orphans.is_empty(),
            "GC must not touch objects that don't match our key format"
        );
    }

    #[test]
    fn mixed_scenario() {
        let now = SystemTime::now();
        let kept_live = format!("{}.iso", sha('a'));
        let deleted_orphan = format!("{}.qcow2", sha('b'));
        let too_young = format!("{}.img", sha('c'));
        let foreign_file = "not-ours.txt".to_string();

        let live: HashSet<String> = [kept_live.clone()].into_iter().collect();
        let stored = vec![
            obj(&kept_live, Duration::from_secs(86400 * 10), now),
            obj(&deleted_orphan, Duration::from_secs(86400 * 10), now),
            obj(&too_young, Duration::from_secs(3600), now),
            obj(&foreign_file, Duration::from_secs(86400 * 30), now),
        ];
        let orphans = compute_orphans(&live, &stored, Duration::from_secs(86400), now);
        assert_eq!(orphans, vec![deleted_orphan]);
    }

    #[test]
    fn build_live_set_handles_shared_keys() {
        use rpc::forge::IpxeTemplateArtifact;
        let key = format!("{}.iso", sha('a'));
        let url = format!("https://cache.example.com/{key}");
        let art = |name: &str| IpxeTemplateArtifact {
            name: name.to_string(),
            cached_url: Some(url.clone()),
            ..Default::default()
        };
        let os_a = OperatingSystem {
            ipxe_template_artifacts: vec![art("kernel")],
            ..Default::default()
        };
        let os_b = OperatingSystem {
            ipxe_template_artifacts: vec![art("kernel")],
            ..Default::default()
        };
        let live = build_live_set(&[os_a, os_b]);
        assert_eq!(live.len(), 1);
        assert!(live.contains(&key));
    }

    #[test]
    fn build_live_set_skips_empty_urls() {
        use rpc::forge::IpxeTemplateArtifact;
        let os = OperatingSystem {
            ipxe_template_artifacts: vec![
                IpxeTemplateArtifact {
                    name: "a".into(),
                    cached_url: None,
                    ..Default::default()
                },
                IpxeTemplateArtifact {
                    name: "b".into(),
                    cached_url: Some(String::new()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(build_live_set(&[os]).is_empty());
    }
}
