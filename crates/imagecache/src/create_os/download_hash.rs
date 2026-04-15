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

use sha2::{Digest, Sha256};

const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024 * 1024; // 50 GiB

/// Download a URL and compute its SHA256 hash.
///
/// Streams the response body through a hasher without writing to disk.
/// Returns the hex-encoded SHA256 digest.
pub async fn download_and_compute_sha256(
    url: &str,
    auth_type: Option<&str>,
    auth_token: Option<&str>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(300))
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let mut request = client.get(url);
    request = apply_auth(request, auth_type, auth_token);

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("HTTP error: {e}"))?;

    let mut hasher = Sha256::new();
    let mut bytes_read: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Download error: {e}"))?
    {
        bytes_read += chunk.len() as u64;
        if bytes_read > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "Download exceeds size limit ({} bytes > {} bytes)",
                bytes_read, MAX_DOWNLOAD_BYTES
            ));
        }
        hasher.update(&chunk);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn apply_auth(
    request: reqwest::RequestBuilder,
    auth_type: Option<&str>,
    auth_token: Option<&str>,
) -> reqwest::RequestBuilder {
    match (auth_type, auth_token) {
        (Some("Bearer"), Some(token)) => request.bearer_auth(token),
        (Some("Basic"), Some(token)) => request.header("Authorization", format!("Basic {token}")),
        _ => request,
    }
}
