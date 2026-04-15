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

use super::app::{App, GENERAL_FIELD_COUNT};

/// Identifies which field the focus index maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldLocation {
    /// General section field by index (0=name, 1=description, 2=tenant_org_id,
    /// 3=is_active, 4=allow_override, 5=phone_home_enabled, 6=user_data_path).
    General(usize),
    /// A required parameter at the given index.
    RequiredParam(usize),
    /// An optional (user-added) parameter at the given index.
    OptionalParam(usize),
    /// A sub-field within an artifact.
    /// (is_required, artifact_index, sub_field_index)
    /// Sub-fields: 0=url, 1=sha, 2=auth_type, 3=auth_token or cache_strategy, 4=cache_strategy
    Artifact {
        required: bool,
        artifact_index: usize,
        sub_field: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Bool,
    EnumAuthType,
    EnumCacheStrategy,
    FilePath,
}

#[allow(dead_code)] // label and required are used for informational display in ui.rs
pub struct FieldDescriptor {
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
    pub location: FieldLocation,
}

/// Resolve a linear focus index to a field location.
pub fn resolve_field(app: &App, index: usize) -> FieldLocation {
    let mut remaining = index;

    // General section
    if remaining < GENERAL_FIELD_COUNT {
        return FieldLocation::General(remaining);
    }
    remaining -= GENERAL_FIELD_COUNT;

    // Required params
    let req_param_count = app.form.required_params.len();
    if remaining < req_param_count {
        return FieldLocation::RequiredParam(remaining);
    }
    remaining -= req_param_count;

    // Optional params
    let opt_param_count = app.form.optional_params.len();
    if remaining < opt_param_count {
        return FieldLocation::OptionalParam(remaining);
    }
    remaining -= opt_param_count;

    // Required artifacts
    for (i, artifact) in app.form.required_artifacts.iter().enumerate() {
        let sub_count = artifact.sub_field_count();
        if remaining < sub_count {
            return FieldLocation::Artifact {
                required: true,
                artifact_index: i,
                sub_field: remaining,
            };
        }
        remaining -= sub_count;
    }

    // Optional artifacts
    for (i, artifact) in app.form.optional_artifacts.iter().enumerate() {
        let sub_count = artifact.sub_field_count();
        if remaining < sub_count {
            return FieldLocation::Artifact {
                required: false,
                artifact_index: i,
                sub_field: remaining,
            };
        }
        remaining -= sub_count;
    }

    // Fallback (shouldn't happen)
    FieldLocation::General(0)
}

/// Get a descriptor for the field at the given focus index.
pub fn field_descriptor(app: &App, index: usize) -> FieldDescriptor {
    let location = resolve_field(app, index);
    match &location {
        FieldLocation::General(i) => {
            let (label, field_type, required) = match i {
                0 => ("Name", FieldType::Text, true),
                1 => ("Description", FieldType::Text, false),
                2 => ("Tenant Org ID", FieldType::Text, true),
                3 => ("Active", FieldType::Bool, false),
                4 => ("Allow Override", FieldType::Bool, false),
                5 => ("Phone Home", FieldType::Bool, false),
                6 => ("User Data File", FieldType::FilePath, false),
                _ => ("Unknown", FieldType::Text, false),
            };
            FieldDescriptor {
                label: label.to_string(),
                field_type,
                required,
                location,
            }
        }
        FieldLocation::RequiredParam(i) => FieldDescriptor {
            label: app.form.required_params[*i].name.clone(),
            field_type: FieldType::Text,
            required: true,
            location,
        },
        FieldLocation::OptionalParam(i) => FieldDescriptor {
            label: app.form.optional_params[*i].name.clone(),
            field_type: FieldType::Text,
            required: false,
            location,
        },
        FieldLocation::Artifact {
            required,
            artifact_index,
            sub_field,
        } => {
            let artifact = if *required {
                &app.form.required_artifacts[*artifact_index]
            } else {
                &app.form.optional_artifacts[*artifact_index]
            };
            let (sub_label, field_type) = artifact_sub_field_info(artifact, *sub_field);
            FieldDescriptor {
                label: format!("{} > {}", artifact.name, sub_label),
                field_type,
                required: *required && *sub_field == 0, // only URL is required
                location,
            }
        }
    }
}

use super::app::{ArtifactField, AuthType};

fn artifact_sub_field_info(
    artifact: &ArtifactField,
    sub_field: usize,
) -> (&'static str, FieldType) {
    // Sub-fields: 0=url, 1=sha, 2=auth_type, then:
    //   if auth_type != None: 3=auth_token, 4=cache_strategy
    //   if auth_type == None: 3=cache_strategy
    match sub_field {
        0 => ("URL", FieldType::Text),
        1 => ("SHA256", FieldType::Text),
        2 => ("Auth Type", FieldType::EnumAuthType),
        3 => {
            if artifact.auth_type != AuthType::None {
                ("Auth Token", FieldType::Text)
            } else {
                ("Cache Strategy", FieldType::EnumCacheStrategy)
            }
        }
        4 => ("Cache Strategy", FieldType::EnumCacheStrategy),
        _ => ("Unknown", FieldType::Text),
    }
}
