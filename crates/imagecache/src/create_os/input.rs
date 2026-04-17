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

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;

use super::app::{
    App, ArtifactField, AuthType, FileStatus, ParamField, Phase, ShaStatus, SubmitResult,
};
use super::form::{self, FieldLocation, FieldType};
use super::{download_hash, template_select};

/// Result from background operations.
pub enum BackgroundResult {
    FileLoaded(Result<String, String>),
    ShaComputed {
        /// Artifact name used to look up the target at result time,
        /// avoiding stale-index bugs if artifacts are added/removed.
        artifact_name: String,
        result: Result<String, String>,
    },
    OsCreated(Result<(String, String), String>),
}

/// Handle a crossterm event. Returns true if the app should redraw.
pub fn handle_event(
    app: &mut App,
    event: Event,
    bg_tx: &mpsc::UnboundedSender<BackgroundResult>,
    api: &rpc::forge_api_client::ForgeApiClient,
) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }

    // Global: Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return true;
    }

    match &app.phase {
        Phase::TemplateSelect => handle_template_select(app, key),
        Phase::Form => handle_form(app, key, bg_tx),
        Phase::Review => handle_review(app, key, bg_tx, api),
        Phase::Submitting => false, // ignore input while submitting
        Phase::Result(_) => handle_result(app, key),
    }
}

/// Handle a background result completing.
pub fn handle_background_result(app: &mut App, result: BackgroundResult) {
    match result {
        BackgroundResult::FileLoaded(res) => match res {
            Ok(content) => {
                let len = content.len();
                app.form.user_data_content = Some(content);
                app.form.user_data_status = FileStatus::Loaded(len);
            }
            Err(e) => {
                app.form.user_data_content = None;
                app.form.user_data_status = FileStatus::Error(e);
            }
        },
        BackgroundResult::ShaComputed {
            artifact_name,
            result,
        } => {
            // Look up artifact by name to avoid stale-index bugs
            let artifact = app
                .form
                .required_artifacts
                .iter_mut()
                .chain(app.form.optional_artifacts.iter_mut())
                .find(|a| a.name == artifact_name);
            if let Some(artifact) = artifact {
                match result {
                    Ok(sha) => {
                        artifact.sha = sha.clone();
                        artifact.sha_status = ShaStatus::Computed(sha);
                    }
                    Err(e) => {
                        artifact.sha_status = ShaStatus::Error(e);
                    }
                }
            }
            // If artifact was removed before result arrived, silently drop it.
        }
        BackgroundResult::OsCreated(res) => match res {
            Ok((os_id, name)) => {
                app.phase = Phase::Result(SubmitResult::Success { os_id, name });
            }
            Err(e) => {
                app.phase = Phase::Result(SubmitResult::Error(e));
            }
        },
    }
}

// --- Phase 1: Template selection ---

fn handle_template_select(app: &mut App, key: KeyEvent) -> bool {
    let filtered = app.filtered_template_indices();

    match key.code {
        KeyCode::Esc => {
            app.should_quit = true;
            true
        }
        KeyCode::Up => {
            let new_sel = template_select::wrap_nav(
                app.template_list_state.selected(),
                filtered.len(),
                false,
            );
            app.template_list_state.select(new_sel);
            true
        }
        KeyCode::Down => {
            let new_sel =
                template_select::wrap_nav(app.template_list_state.selected(), filtered.len(), true);
            app.template_list_state.select(new_sel);
            true
        }
        KeyCode::Enter => {
            if let Some(sel) = app.template_list_state.selected()
                && sel < filtered.len()
            {
                app.select_template(filtered[sel]);
                return true;
            }
            false
        }
        KeyCode::Backspace => {
            app.template_filter.pop();
            // Reset selection
            app.template_list_state
                .select(if app.filtered_template_indices().is_empty() {
                    None
                } else {
                    Some(0)
                });
            true
        }
        KeyCode::Char(c) => {
            app.template_filter.push(c);
            app.template_list_state
                .select(if app.filtered_template_indices().is_empty() {
                    None
                } else {
                    Some(0)
                });
            true
        }
        _ => false,
    }
}

// --- Phase 2: Form ---

fn handle_form(
    app: &mut App,
    key: KeyEvent,
    bg_tx: &mpsc::UnboundedSender<BackgroundResult>,
) -> bool {
    let total = app.total_field_count();
    if total == 0 {
        return false;
    }

    // Navigation
    match key.code {
        KeyCode::Tab | KeyCode::Down => {
            app.focused_field = if app.focused_field + 1 >= total {
                0
            } else {
                app.focused_field + 1
            };
            return true;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.focused_field = if app.focused_field == 0 {
                total.saturating_sub(1)
            } else {
                app.focused_field - 1
            };
            return true;
        }
        KeyCode::Esc => {
            if app.quit_confirm {
                app.phase = Phase::TemplateSelect;
                app.quit_confirm = false;
            } else if app.has_form_data() {
                app.quit_confirm = true;
                app.status_message = Some("Press Esc again to discard and go back".into());
            } else {
                app.phase = Phase::TemplateSelect;
            }
            return true;
        }
        _ => {
            // Clear quit confirm on any other key
            if app.quit_confirm {
                app.quit_confirm = false;
                app.status_message = None;
            }
        }
    }

    // Ctrl+S: advance to review screen
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        app.phase = Phase::Review;
        app.review_scroll = 0;
        app.status_message = None;
        return true;
    }

    // Ctrl+A: add optional param or artifact
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
        return handle_add_optional(app);
    }

    // Ctrl+X: remove optional param or artifact
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('x') {
        return handle_remove_optional(app);
    }

    let desc = form::field_descriptor(app, app.focused_field);

    match desc.field_type {
        FieldType::Text => handle_text_field(app, key, &desc.location),
        FieldType::Bool => handle_bool_field(app, key, &desc.location),
        FieldType::EnumAuthType => handle_auth_type_field(app, key, &desc.location),
        FieldType::EnumCacheStrategy => handle_cache_strategy_field(app, key, &desc.location),
        FieldType::FilePath => handle_file_path_field(app, key, &desc.location, bg_tx),
    }
}

fn handle_text_field(app: &mut App, key: KeyEvent, location: &FieldLocation) -> bool {
    let field = get_text_field_mut(app, location);

    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'u' {
                field.clear();
            } else {
                field.push(c);
            }
            true
        }
        KeyCode::Backspace => {
            field.pop();
            true
        }
        KeyCode::Enter => true,
        _ => false,
    }
}

fn handle_bool_field(app: &mut App, key: KeyEvent, location: &FieldLocation) -> bool {
    if key.code == KeyCode::Char(' ') || key.code == KeyCode::Enter {
        let field = get_bool_field_mut(app, location);
        *field = !*field;
        return true;
    }
    false
}

fn handle_auth_type_field(app: &mut App, key: KeyEvent, location: &FieldLocation) -> bool {
    let FieldLocation::Artifact {
        required,
        artifact_index,
        ..
    } = location
    else {
        return false;
    };

    let artifact = if *required {
        &mut app.form.required_artifacts[*artifact_index]
    } else {
        &mut app.form.optional_artifacts[*artifact_index]
    };

    let old_has_token = artifact.auth_type != AuthType::None;

    let toggled = match key.code {
        KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => {
            artifact.auth_type = artifact.auth_type.next();
            true
        }
        KeyCode::Left => {
            artifact.auth_type = artifact.auth_type.prev();
            true
        }
        _ => false,
    };

    if toggled {
        let new_has_token = artifact.auth_type != AuthType::None;
        if old_has_token != new_has_token {
            // The auth_token field was inserted or removed, changing
            // total_field_count. Clamp focused_field to stay valid.
            let total = app.total_field_count();
            if app.focused_field >= total {
                app.focused_field = total.saturating_sub(1);
            }
        }
    }

    toggled
}

fn handle_cache_strategy_field(app: &mut App, key: KeyEvent, location: &FieldLocation) -> bool {
    let FieldLocation::Artifact {
        required,
        artifact_index,
        ..
    } = location
    else {
        return false;
    };

    let artifact = if *required {
        &mut app.form.required_artifacts[*artifact_index]
    } else {
        &mut app.form.optional_artifacts[*artifact_index]
    };

    match key.code {
        KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => {
            artifact.cache_strategy = artifact.cache_strategy.next();
            true
        }
        KeyCode::Left => {
            artifact.cache_strategy = artifact.cache_strategy.prev();
            true
        }
        _ => false,
    }
}

fn handle_file_path_field(
    app: &mut App,
    key: KeyEvent,
    _location: &FieldLocation,
    bg_tx: &mpsc::UnboundedSender<BackgroundResult>,
) -> bool {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'u' {
                app.form.user_data_path.clear();
            } else {
                app.form.user_data_path.push(c);
            }
            // Path changed — invalidate any previously loaded content
            app.form.user_data_content = None;
            app.form.user_data_status = FileStatus::NotSet;
            true
        }
        KeyCode::Backspace => {
            app.form.user_data_path.pop();
            // Path changed — invalidate any previously loaded content
            app.form.user_data_content = None;
            app.form.user_data_status = FileStatus::NotSet;
            true
        }
        KeyCode::Enter => {
            if !app.form.user_data_path.is_empty() {
                app.form.user_data_status = FileStatus::Loading;
                let path = app.form.user_data_path.clone();
                let tx = bg_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::fs::read_to_string(&path)
                        .await
                        .map_err(|e| format!("{e}"));
                    let _ = tx.send(BackgroundResult::FileLoaded(result));
                });
            }
            true
        }
        _ => false,
    }
}

/// Handle Ctrl+V to trigger SHA download for the currently focused artifact field.
pub fn maybe_trigger_sha_download(
    app: &mut App,
    key: KeyEvent,
    bg_tx: &mpsc::UnboundedSender<BackgroundResult>,
) -> bool {
    if !(key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v')) {
        return false;
    }
    if app.phase != Phase::Form {
        return false;
    }

    let location = form::resolve_field(app, app.focused_field);
    let FieldLocation::Artifact {
        required,
        artifact_index,
        sub_field,
    } = location
    else {
        return false;
    };

    // Only trigger on the SHA sub-field (index 1) or URL sub-field (index 0)
    if sub_field > 1 {
        return false;
    }

    let artifact = if required {
        &mut app.form.required_artifacts[artifact_index]
    } else {
        &mut app.form.optional_artifacts[artifact_index]
    };

    if artifact.url.is_empty() {
        app.status_message = Some("URL is empty — cannot download".into());
        return true;
    }

    artifact.sha_status = ShaStatus::Downloading;
    let name = artifact.name.clone();
    let url = artifact.url.clone();
    let auth_type_str = artifact.auth_type.to_proto();
    let auth_token = if artifact.auth_token.is_empty() {
        None
    } else {
        Some(artifact.auth_token.clone())
    };
    let tx = bg_tx.clone();

    tokio::spawn(async move {
        let result = download_hash::download_and_compute_sha256(
            &url,
            auth_type_str.as_deref(),
            auth_token.as_deref(),
        )
        .await;
        let _ = tx.send(BackgroundResult::ShaComputed {
            artifact_name: name,
            result,
        });
    });

    true
}

fn handle_add_optional(app: &mut App) -> bool {
    let location = form::resolve_field(app, app.focused_field);
    match location {
        FieldLocation::General(_)
        | FieldLocation::RequiredParam(_)
        | FieldLocation::OptionalParam(_) => {
            // Add an optional parameter
            let id = app.next_optional_id;
            app.next_optional_id += 1;
            app.form.optional_params.push(ParamField {
                name: format!("param_{id}"),
                value: String::new(),
            });
            app.status_message = Some("Added optional parameter".into());
            true
        }
        FieldLocation::Artifact { .. } => {
            // Add an optional artifact
            let id = app.next_optional_id;
            app.next_optional_id += 1;
            app.form
                .optional_artifacts
                .push(ArtifactField::new(format!("artifact_{id}")));
            app.status_message = Some("Added optional artifact".into());
            true
        }
    }
}

fn handle_remove_optional(app: &mut App) -> bool {
    let location = form::resolve_field(app, app.focused_field);
    match location {
        FieldLocation::OptionalParam(i) => {
            app.form.optional_params.remove(i);
            // Clamp focus to valid range after removal
            let total = app.total_field_count();
            if total == 0 {
                app.focused_field = 0;
            } else if app.focused_field >= total {
                app.focused_field = total - 1;
            }
            app.status_message = Some("Removed optional parameter".into());
            true
        }
        FieldLocation::Artifact {
            required: false,
            artifact_index,
            ..
        } => {
            app.form.optional_artifacts.remove(artifact_index);
            // Clamp focus to valid range — an artifact has multiple sub-fields,
            // so simple decrement-by-1 is insufficient.
            let total = app.total_field_count();
            if total == 0 {
                app.focused_field = 0;
            } else if app.focused_field >= total {
                app.focused_field = total - 1;
            }
            app.status_message = Some("Removed optional artifact".into());
            true
        }
        _ => {
            app.status_message = Some("Can only remove optional fields".into());
            true
        }
    }
}

// --- Phase 3: Review ---

fn handle_review(
    app: &mut App,
    key: KeyEvent,
    bg_tx: &mpsc::UnboundedSender<BackgroundResult>,
    api: &rpc::forge_api_client::ForgeApiClient,
) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.phase = Phase::Form;
            true
        }
        KeyCode::Up => {
            app.review_scroll = app.review_scroll.saturating_sub(1);
            true
        }
        KeyCode::Down => {
            if app.review_scroll < app.review_scroll_max {
                app.review_scroll += 1;
            }
            true
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let json = super::review::build_review_json(app);
            let filename = format!(
                "os-definition-{}.json",
                app.form
                    .name
                    .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_")
            );
            match std::fs::write(&filename, &json) {
                Ok(()) => {
                    app.status_message = Some(format!("Saved to {filename}"));
                }
                Err(e) => {
                    app.status_message = Some(format!("Save failed: {e}"));
                }
            }
            true
        }
        KeyCode::Enter => {
            let errors = app.validate();
            if !errors.is_empty() {
                app.status_message = Some(format!("Validation failed: {}", errors.join("; ")));
                return true;
            }

            app.phase = Phase::Submitting;
            let request = app.build_request();
            let api = api.clone();
            let tx = bg_tx.clone();
            tokio::spawn(async move {
                let result = api.create_operating_system(request).await;
                let mapped = result
                    .map(|os| {
                        let id = os
                            .id
                            .map(|u| u.to_string())
                            .unwrap_or_else(|| "unknown".into());
                        (id, os.name)
                    })
                    .map_err(|e| format!("gRPC error: {e}"));
                let _ = tx.send(BackgroundResult::OsCreated(mapped));
            });
            true
        }
        _ => false,
    }
}

// --- Phase Result ---

fn handle_result(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter | KeyCode::Char('q') => {
            app.should_quit = true;
            true
        }
        KeyCode::Esc => {
            // Allow going back to form on error
            if matches!(app.phase, Phase::Result(SubmitResult::Error(_))) {
                app.phase = Phase::Form;
            } else {
                app.should_quit = true;
            }
            true
        }
        _ => false,
    }
}

// --- Field access helpers ---

fn get_text_field_mut<'a>(app: &'a mut App, location: &FieldLocation) -> &'a mut String {
    match location {
        FieldLocation::General(0) => &mut app.form.name,
        FieldLocation::General(1) => &mut app.form.description,
        FieldLocation::General(2) => &mut app.form.tenant_organization_id,
        FieldLocation::RequiredParam(i) => &mut app.form.required_params[*i].value,
        FieldLocation::OptionalParam(i) => {
            // For optional params, the "name" field is editable too,
            // but we only have one text field per param (the value).
            // The name is set at creation time via Ctrl+A.
            &mut app.form.optional_params[*i].value
        }
        FieldLocation::Artifact {
            required,
            artifact_index,
            sub_field,
        } => {
            let artifact = if *required {
                &mut app.form.required_artifacts[*artifact_index]
            } else {
                &mut app.form.optional_artifacts[*artifact_index]
            };
            match sub_field {
                0 => &mut artifact.url,
                1 => &mut artifact.sha,
                3 if artifact.auth_type != AuthType::None => &mut artifact.auth_token,
                _ => unreachable!("get_text_field_mut: unexpected artifact sub_field {sub_field}"),
            }
        }
        _ => unreachable!("get_text_field_mut called on non-text field"),
    }
}

fn get_bool_field_mut<'a>(app: &'a mut App, location: &FieldLocation) -> &'a mut bool {
    match location {
        FieldLocation::General(3) => &mut app.form.is_active,
        FieldLocation::General(4) => &mut app.form.allow_override,
        FieldLocation::General(5) => &mut app.form.phone_home_enabled,
        _ => unreachable!("get_bool_field_mut called on non-bool field"),
    }
}
