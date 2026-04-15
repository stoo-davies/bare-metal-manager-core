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

//! Interactive TUI tool for creating Operating System definitions via
//! the `forge.Forge/CreateOperatingSystem` gRPC call.
//!
//! Guides the user through:
//! 1. Selecting an iPXE template
//! 2. Filling in OS definition fields (parameters, artifacts, user_data)
//! 3. Reviewing and submitting the request
//!
//! Required environment variables:
//!   CARBIDE_API_INTERNAL_URL  (defaults to https://carbide-api.forge-system.svc.cluster.local:1079)
//!   FORGE_ROOT_CAFILE_PATH
//!   FORGE_CLIENT_CERT_PATH
//!   FORGE_CLIENT_KEY_PATH

#[path = "create_os/app.rs"]
mod app;
#[path = "create_os/download_hash.rs"]
mod download_hash;
#[path = "create_os/form.rs"]
mod form;
#[path = "create_os/input.rs"]
mod input;
#[path = "create_os/review.rs"]
mod review;
#[path = "create_os/template_select.rs"]
mod template_select;
#[path = "create_os/ui.rs"]
mod ui;

use std::env;
use std::io;
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::event::{Event, EventStream};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use rpc::forge_api_client::ForgeApiClient;
use rpc::forge_tls_client::{ApiConfig, ForgeClientConfig};

use app::App;

fn env_required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let internal_api_url = env::var("CARBIDE_API_INTERNAL_URL")
        .unwrap_or_else(|_| "https://carbide-api.forge-system.svc.cluster.local:1079".into());

    let client_cert = Some(forge_tls::client_config::ClientCert {
        cert_path: env_required("FORGE_CLIENT_CERT_PATH"),
        key_path: env_required("FORGE_CLIENT_KEY_PATH"),
    });
    let client_config = ForgeClientConfig::new(env_required("FORGE_ROOT_CAFILE_PATH"), client_cert);
    let api = ForgeApiClient::new(&ApiConfig::new(&internal_api_url, &client_config));

    // Fetch templates before entering TUI
    eprintln!("Connecting to {}...", internal_api_url);
    let template_list = api
        .list_ipxe_templates()
        .await
        .map_err(|e| format!("Failed to fetch templates: {e}"))?;

    if template_list.templates.is_empty() {
        eprintln!("No iPXE templates found. Nothing to do.");
        return Ok(());
    }
    eprintln!(
        "Loaded {} templates. Launching TUI...",
        template_list.templates.len()
    );

    // Set up terminal
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui(&mut terminal, template_list.templates, api).await;

    // Tear down terminal
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

async fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    templates: Vec<rpc::forge::IpxeTemplate>,
    api: ForgeApiClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new(templates);
    let mut event_stream = EventStream::new();
    let (bg_tx, mut bg_rx) = tokio::sync::mpsc::unbounded_channel();

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            biased;

            Some(bg_result) = bg_rx.recv() => {
                input::handle_background_result(&mut app, bg_result);
            }

            maybe_event = event_stream.next() => {
                if let Some(Ok(event)) = maybe_event {
                    // Check Ctrl+V for SHA download first
                    if let Event::Key(key) = &event
                        && key.kind == crossterm::event::KeyEventKind::Press
                        && input::maybe_trigger_sha_download(&mut app, *key, &bg_tx)
                    {
                        continue;
                    }
                    input::handle_event(&mut app, event, &bg_tx, &api);
                }
            }

            _ = tokio::time::sleep(Duration::from_millis(100)), if app.has_spinner() => {
                // Tick for spinner animation
            }
        }
    }

    Ok(())
}
