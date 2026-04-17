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

use ratatui::widgets::ListState;
use rpc::forge::{
    CreateOperatingSystemRequest, IpxeTemplate, IpxeTemplateArtifact,
    IpxeTemplateArtifactCacheStrategy, IpxeTemplateParameter,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    TemplateSelect,
    Form,
    Review,
    Submitting,
    Result(SubmitResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitResult {
    Success { os_id: String, name: String },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    None,
    Basic,
    Bearer,
}

impl AuthType {
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Basic,
            Self::Basic => Self::Bearer,
            Self::Bearer => Self::None,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::None => Self::Bearer,
            Self::Basic => Self::None,
            Self::Bearer => Self::Basic,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Basic => "Basic",
            Self::Bearer => "Bearer",
        }
    }

    pub fn to_proto(self) -> Option<String> {
        match self {
            Self::None => None,
            Self::Basic => Some("Basic".into()),
            Self::Bearer => Some("Bearer".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStrategyChoice {
    CacheAsNeeded,
    LocalOnly,
    CachedOnly,
    RemoteOnly,
}

impl CacheStrategyChoice {
    pub fn next(self) -> Self {
        match self {
            Self::CacheAsNeeded => Self::LocalOnly,
            Self::LocalOnly => Self::CachedOnly,
            Self::CachedOnly => Self::RemoteOnly,
            Self::RemoteOnly => Self::CacheAsNeeded,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::CacheAsNeeded => Self::RemoteOnly,
            Self::LocalOnly => Self::CacheAsNeeded,
            Self::CachedOnly => Self::LocalOnly,
            Self::RemoteOnly => Self::CachedOnly,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CacheAsNeeded => "CacheAsNeeded",
            Self::LocalOnly => "LocalOnly",
            Self::CachedOnly => "CachedOnly",
            Self::RemoteOnly => "RemoteOnly",
        }
    }

    pub fn to_proto_i32(self) -> i32 {
        match self {
            Self::CacheAsNeeded => IpxeTemplateArtifactCacheStrategy::CacheAsNeeded as i32,
            Self::LocalOnly => IpxeTemplateArtifactCacheStrategy::LocalOnly as i32,
            Self::CachedOnly => IpxeTemplateArtifactCacheStrategy::CachedOnly as i32,
            Self::RemoteOnly => IpxeTemplateArtifactCacheStrategy::RemoteOnly as i32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    NotSet,
    Loading,
    Loaded(usize),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaStatus {
    NotSet,
    Downloading,
    Computed(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ParamField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ArtifactField {
    pub name: String,
    pub url: String,
    pub sha: String,
    pub auth_type: AuthType,
    pub auth_token: String,
    pub cache_strategy: CacheStrategyChoice,
    pub sha_status: ShaStatus,
}

impl ArtifactField {
    pub fn new(name: String) -> Self {
        Self {
            name,
            url: String::new(),
            sha: String::new(),
            auth_type: AuthType::None,
            auth_token: String::new(),
            cache_strategy: CacheStrategyChoice::CacheAsNeeded,
            sha_status: ShaStatus::NotSet,
        }
    }

    /// Number of navigable sub-fields for this artifact.
    pub fn sub_field_count(&self) -> usize {
        // url, sha, auth_type, auth_token (only if auth != None), cache_strategy
        if self.auth_type == AuthType::None {
            4 // url, sha, auth_type, cache_strategy
        } else {
            5 // url, sha, auth_type, auth_token, cache_strategy
        }
    }
}

#[derive(Debug, Clone)]
pub struct OsForm {
    // General section
    pub name: String,
    pub description: String,
    pub tenant_organization_id: String,
    pub is_active: bool,
    pub allow_override: bool,
    pub phone_home_enabled: bool,
    pub user_data_path: String,
    pub user_data_content: Option<String>,
    pub user_data_status: FileStatus,

    // Template parameters section
    pub required_params: Vec<ParamField>,
    pub optional_params: Vec<ParamField>,
    pub reserved_params: Vec<String>,

    // Artifacts section
    pub required_artifacts: Vec<ArtifactField>,
    pub optional_artifacts: Vec<ArtifactField>,
}

impl OsForm {
    pub fn from_template(template: &IpxeTemplate) -> Self {
        let required_params = template
            .required_params
            .iter()
            .map(|name| ParamField {
                name: name.clone(),
                value: String::new(),
            })
            .collect();

        let reserved_params = template.reserved_params.clone();

        let required_artifacts = template
            .required_artifacts
            .iter()
            .map(|name| ArtifactField::new(name.clone()))
            .collect();

        Self {
            name: String::new(),
            description: String::new(),
            tenant_organization_id: String::new(),
            is_active: true,
            allow_override: false,
            phone_home_enabled: false,
            user_data_path: String::new(),
            user_data_content: None,
            user_data_status: FileStatus::NotSet,
            required_params,
            optional_params: Vec::new(),
            reserved_params,
            required_artifacts,
            optional_artifacts: Vec::new(),
        }
    }
}

/// General section field indices (0-based within the section).
pub const GENERAL_FIELD_COUNT: usize = 7;

pub struct App {
    pub phase: Phase,
    pub templates: Vec<IpxeTemplate>,
    pub selected_template: Option<IpxeTemplate>,

    // Phase 1 state
    pub template_filter: String,
    pub template_list_state: ListState,

    // Phase 2 state
    pub form: OsForm,
    pub focused_field: usize,
    pub scroll_offset: usize,

    // Phase 3 state
    pub review_scroll: usize,
    /// Maximum useful scroll offset for the review pane, set during draw.
    pub review_scroll_max: usize,

    // Shared
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub quit_confirm: bool,
    /// Monotonic counter for generating unique optional field names.
    pub next_optional_id: usize,
}

impl App {
    pub fn new(mut templates: Vec<IpxeTemplate>) -> Self {
        templates.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
        let mut list_state = ListState::default();
        if !templates.is_empty() {
            list_state.select(Some(0));
        }

        // Provide a dummy empty form until a template is selected
        let dummy_template = IpxeTemplate {
            name: String::new(),
            template: String::new(),
            required_params: vec![],
            description: String::new(),
            reserved_params: vec![],
            required_artifacts: vec![],
            scope: 0,
            id: None,
        };

        Self {
            phase: Phase::TemplateSelect,
            templates,
            selected_template: None,
            template_filter: String::new(),
            template_list_state: list_state,
            form: OsForm::from_template(&dummy_template),
            focused_field: 0,
            scroll_offset: 0,
            review_scroll: 0,
            review_scroll_max: 0,
            status_message: None,
            should_quit: false,
            quit_confirm: false,
            next_optional_id: 1,
        }
    }

    pub fn filtered_template_indices(&self) -> Vec<usize> {
        if self.template_filter.is_empty() {
            return (0..self.templates.len()).collect();
        }
        let filter = self.template_filter.to_ascii_lowercase();
        self.templates
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.name.to_ascii_lowercase().contains(&filter)
                    || t.description.to_ascii_lowercase().contains(&filter)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn select_template(&mut self, template_index: usize) {
        let template = self.templates[template_index].clone();
        self.form = OsForm::from_template(&template);
        self.selected_template = Some(template);
        self.focused_field = 0;
        self.scroll_offset = 0;
        self.phase = Phase::Form;
    }

    /// Total number of navigable fields in the form.
    pub fn total_field_count(&self) -> usize {
        let param_count = self.form.required_params.len() + self.form.optional_params.len();
        let artifact_field_count: usize = self
            .form
            .required_artifacts
            .iter()
            .chain(self.form.optional_artifacts.iter())
            .map(|a| a.sub_field_count())
            .sum();
        GENERAL_FIELD_COUNT + param_count + artifact_field_count
    }

    pub fn has_form_data(&self) -> bool {
        !self.form.name.is_empty()
            || !self.form.description.is_empty()
            || !self.form.tenant_organization_id.is_empty()
            || !self.form.user_data_path.is_empty()
            || self
                .form
                .required_params
                .iter()
                .any(|p| !p.value.is_empty())
            || !self.form.optional_params.is_empty()
            || self
                .form
                .required_artifacts
                .iter()
                .any(|a| !a.url.is_empty() || !a.sha.is_empty())
            || !self.form.optional_artifacts.is_empty()
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.form.name.is_empty() {
            errors.push("Name is required".into());
        }
        if self.form.tenant_organization_id.is_empty() {
            errors.push("Tenant Organization ID is required".into());
        }
        for p in &self.form.required_params {
            if p.value.is_empty() {
                errors.push(format!("Parameter '{}' is required", p.name));
            }
        }
        for a in &self.form.required_artifacts {
            if a.url.is_empty() {
                errors.push(format!("Artifact '{}' URL is required", a.name));
            }
        }
        for a in self
            .form
            .required_artifacts
            .iter()
            .chain(self.form.optional_artifacts.iter())
        {
            if a.auth_type != AuthType::None && a.auth_token.is_empty() {
                errors.push(format!(
                    "Artifact '{}' has auth type {} but no token",
                    a.name,
                    a.auth_type.as_str()
                ));
            }
        }
        errors
    }

    pub fn build_request(&self) -> CreateOperatingSystemRequest {
        let template = self.selected_template.as_ref().expect("template selected");

        let parameters: Vec<IpxeTemplateParameter> = self
            .form
            .required_params
            .iter()
            .chain(self.form.optional_params.iter())
            .filter(|p| !p.value.is_empty())
            .map(|p| IpxeTemplateParameter {
                name: p.name.clone(),
                value: p.value.clone(),
            })
            .collect();

        let artifacts: Vec<IpxeTemplateArtifact> = self
            .form
            .required_artifacts
            .iter()
            .chain(self.form.optional_artifacts.iter())
            .filter(|a| !a.url.is_empty())
            .map(|a| IpxeTemplateArtifact {
                name: a.name.clone(),
                url: a.url.clone(),
                sha: if a.sha.is_empty() {
                    None
                } else {
                    Some(a.sha.clone())
                },
                auth_type: a.auth_type.to_proto(),
                auth_token: if a.auth_token.is_empty() {
                    None
                } else {
                    Some(a.auth_token.clone())
                },
                cache_strategy: a.cache_strategy.to_proto_i32(),
                cached_url: None,
            })
            .collect();

        CreateOperatingSystemRequest {
            name: self.form.name.clone(),
            description: if self.form.description.is_empty() {
                None
            } else {
                Some(self.form.description.clone())
            },
            tenant_organization_id: self.form.tenant_organization_id.clone(),
            is_active: self.form.is_active,
            allow_override: self.form.allow_override,
            phone_home_enabled: self.form.phone_home_enabled,
            user_data: self.form.user_data_content.clone(),
            id: None,
            ipxe_script: None,
            ipxe_template_id: template.id,
            ipxe_template_parameters: parameters,
            ipxe_template_artifacts: artifacts,
        }
    }

    pub fn has_spinner(&self) -> bool {
        matches!(self.phase, Phase::Submitting)
            || self.form.user_data_status == FileStatus::Loading
            || self
                .form
                .required_artifacts
                .iter()
                .chain(self.form.optional_artifacts.iter())
                .any(|a| a.sha_status == ShaStatus::Downloading)
    }
}
