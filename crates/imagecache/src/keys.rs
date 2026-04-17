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

//! Helpers for the storage key format shared by the cache loop and the GC loop.
//!
//! Cache keys always look like `{sha256-lowercase-hex}[.ext]` — see
//! `cache_loop::s3_key_for_artifact`. The GC loop uses `is_valid_cache_key` as a
//! safety rail so it never deletes objects that don't look like our own.

/// Extracts the storage key from a `cached_url` (the path segment after the last slash).
pub fn key_from_cached_url(cached_url: &str) -> &str {
    cached_url.rsplit('/').next().unwrap_or(cached_url)
}

/// Returns `true` if `key` matches the cache key format: 64 lowercase hex chars,
/// optionally followed by a `.ext` suffix.
pub fn is_valid_cache_key(key: &str) -> bool {
    let stem = match key.find('.') {
        Some(pos) => &key[..pos],
        None => key,
    };
    stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_from_cached_url_strips_host() {
        assert_eq!(
            key_from_cached_url("https://cache.example.com/abc123.iso"),
            "abc123.iso"
        );
    }

    #[test]
    fn key_from_cached_url_no_slash() {
        assert_eq!(key_from_cached_url("abc123.iso"), "abc123.iso");
    }

    #[test]
    fn is_valid_cache_key_accepts_sha_with_ext() {
        let sha = "a".repeat(64);
        assert!(is_valid_cache_key(&format!("{sha}.iso")));
    }

    #[test]
    fn is_valid_cache_key_accepts_sha_no_ext() {
        let sha = "a".repeat(64);
        assert!(is_valid_cache_key(&sha));
    }

    #[test]
    fn is_valid_cache_key_rejects_short_stem() {
        assert!(!is_valid_cache_key("abc.iso"));
    }

    #[test]
    fn is_valid_cache_key_rejects_uppercase() {
        let sha = "A".repeat(64);
        assert!(!is_valid_cache_key(&format!("{sha}.iso")));
    }

    #[test]
    fn is_valid_cache_key_rejects_non_hex() {
        let sha = "z".repeat(64);
        assert!(!is_valid_cache_key(&format!("{sha}.iso")));
    }

    #[test]
    fn is_valid_cache_key_rejects_empty() {
        assert!(!is_valid_cache_key(""));
    }

    #[test]
    fn is_valid_cache_key_rejects_random_file() {
        assert!(!is_valid_cache_key("readme.txt"));
    }
}
