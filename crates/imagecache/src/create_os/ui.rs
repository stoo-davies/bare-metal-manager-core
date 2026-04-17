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

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, Paragraph, Wrap};

use super::app::{App, AuthType, FileStatus, Phase, ShaStatus, SubmitResult};
use super::form::{self, FieldLocation, FieldType};
use super::{review, template_select};

pub fn draw(frame: &mut Frame, app: &mut App) {
    match &app.phase {
        Phase::TemplateSelect => draw_template_select(frame, app),
        Phase::Form => draw_form(frame, app),
        Phase::Review => draw_review(frame, app),
        Phase::Submitting => draw_submitting(frame),
        Phase::Result(result) => draw_result(frame, result.clone()),
    }
}

// --- Phase 1: Template selection ---

fn draw_template_select(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // filter input
            Constraint::Min(5),    // template list
            Constraint::Length(3), // help bar
        ])
        .split(frame.area());

    // Filter input
    let filter_text = format!("Filter: {}_", app.template_filter);
    let filter = Paragraph::new(filter_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select iPXE Template "),
    );
    frame.render_widget(filter, chunks[0]);

    // Template list
    let filtered = app.filtered_template_indices();
    let items = template_select::template_list_items(&app.templates, &filtered);
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(format!(
            " Templates ({}/{}) ",
            filtered.len(),
            app.templates.len()
        )))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::REVERSED),
        );
    frame.render_stateful_widget(list, chunks[1], &mut app.template_list_state);

    // Help bar
    let help = Paragraph::new(Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Yellow)),
        Span::raw(" Navigate  "),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" Select  "),
        Span::styled("Type", Style::default().fg(Color::Yellow)),
        Span::raw(" to filter  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" Quit"),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(help, chunks[2]);
}

// --- Phase 2: Form ---

fn draw_form(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(5),    // form fields
            Constraint::Length(3), // status/help bar
        ])
        .split(frame.area());

    // Title
    let template_name = app
        .selected_template
        .as_ref()
        .map(|t| t.name.as_str())
        .unwrap_or("?");
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " OS Definition ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("(template: {template_name})")),
    ]));
    frame.render_widget(title, chunks[0]);

    // Form fields — the inner area is 2 rows smaller than chunks[1] due to the border
    let (form_lines, focused_line) = build_form_lines(app);
    let inner_height = chunks[1].height.saturating_sub(2) as usize; // border top + bottom

    // Auto-scroll so the focused field is always visible
    if inner_height > 0 {
        if focused_line < app.scroll_offset {
            app.scroll_offset = focused_line;
        } else if focused_line >= app.scroll_offset + inner_height {
            app.scroll_offset = focused_line - inner_height + 1;
        }
    }

    let form_widget = Paragraph::new(form_lines)
        .block(Block::default().borders(Borders::ALL))
        .scroll((app.scroll_offset as u16, 0));
    frame.render_widget(form_widget, chunks[1]);

    // Status/help bar
    let help_line = build_form_help(app);
    let help = Paragraph::new(help_line).block(Block::default().borders(Borders::ALL));
    frame.render_widget(help, chunks[2]);
}

/// Build the form lines and return (lines, focused_line_index).
fn build_form_lines(app: &App) -> (Vec<Line<'static>>, usize) {
    let mut lines = Vec::new();
    let focused = app.focused_field;
    let mut field_idx: usize = 0;
    let mut focused_line: usize = 0;

    // --- General section ---
    lines.push(section_header("General"));

    let general_fields: Vec<(&str, String, bool, FieldType)> = vec![
        ("Name", app.form.name.clone(), true, FieldType::Text),
        (
            "Description",
            app.form.description.clone(),
            false,
            FieldType::Text,
        ),
        (
            "Tenant Org ID",
            app.form.tenant_organization_id.clone(),
            true,
            FieldType::Text,
        ),
        (
            "Active",
            if app.form.is_active { "true" } else { "false" }.into(),
            false,
            FieldType::Bool,
        ),
        (
            "Allow Override",
            if app.form.allow_override {
                "true"
            } else {
                "false"
            }
            .into(),
            false,
            FieldType::Bool,
        ),
        (
            "Phone Home",
            if app.form.phone_home_enabled {
                "true"
            } else {
                "false"
            }
            .into(),
            false,
            FieldType::Bool,
        ),
        (
            "User Data File",
            format_user_data_field(&app.form.user_data_path, &app.form.user_data_status),
            false,
            FieldType::FilePath,
        ),
    ];

    for (label, value, required, field_type) in general_fields {
        if field_idx == focused {
            focused_line = lines.len();
        }
        lines.push(form_field_line(
            label,
            &value,
            required,
            field_idx == focused,
            field_type,
        ));
        field_idx += 1;
    }

    // --- Template Parameters section ---
    if !app.form.required_params.is_empty()
        || !app.form.optional_params.is_empty()
        || !app.form.reserved_params.is_empty()
    {
        lines.push(Line::raw(""));
        lines.push(section_header("Template Parameters"));

        for p in &app.form.required_params {
            if field_idx == focused {
                focused_line = lines.len();
            }
            lines.push(form_field_line(
                &p.name,
                &p.value,
                true,
                field_idx == focused,
                FieldType::Text,
            ));
            field_idx += 1;
        }

        for p in &app.form.optional_params {
            if field_idx == focused {
                focused_line = lines.len();
            }
            lines.push(form_field_line(
                &p.name,
                &p.value,
                false,
                field_idx == focused,
                FieldType::Text,
            ));
            field_idx += 1;
        }

        // Reserved params (display only, not navigable)
        for name in &app.form.reserved_params {
            lines.push(Line::from(vec![
                Span::styled(format!("  {name}"), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    " (reserved — set by system)",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    // --- Artifacts section ---
    if !app.form.required_artifacts.is_empty() || !app.form.optional_artifacts.is_empty() {
        lines.push(Line::raw(""));
        lines.push(section_header("Artifacts"));

        for a in &app.form.required_artifacts {
            let (new_idx, fl) = render_artifact_lines(&mut lines, a, true, field_idx, focused);
            field_idx = new_idx;
            if let Some(l) = fl {
                focused_line = l;
            }
        }
        for a in &app.form.optional_artifacts {
            let (new_idx, fl) = render_artifact_lines(&mut lines, a, false, field_idx, focused);
            field_idx = new_idx;
            if let Some(l) = fl {
                focused_line = l;
            }
        }
    }

    // Ensure field_idx is used (suppress warning)
    let _ = field_idx;

    (lines, focused_line)
}

/// Returns (new_field_idx, Option<focused_line_number>).
fn render_artifact_lines(
    lines: &mut Vec<Line<'static>>,
    artifact: &super::app::ArtifactField,
    required: bool,
    mut field_idx: usize,
    focused: usize,
) -> (usize, Option<usize>) {
    let mut focused_line = None;

    let req_marker = if required { " [REQUIRED]" } else { "" };
    lines.push(Line::from(vec![Span::styled(
        format!("  ── {} ──{}", artifact.name, req_marker),
        Style::default()
            .fg(if required { Color::Yellow } else { Color::Cyan })
            .add_modifier(Modifier::BOLD),
    )]));

    // URL
    if field_idx == focused {
        focused_line = Some(lines.len());
    }
    lines.push(form_field_line(
        "    URL",
        &artifact.url,
        required,
        field_idx == focused,
        FieldType::Text,
    ));
    field_idx += 1;

    // SHA
    let sha_display = match &artifact.sha_status {
        ShaStatus::NotSet => artifact.sha.clone(),
        ShaStatus::Downloading => format!("{} (downloading...)", artifact.sha),
        ShaStatus::Computed(h) => h.clone(),
        ShaStatus::Error(e) => format!("{} (error: {})", artifact.sha, e),
    };
    if field_idx == focused {
        focused_line = Some(lines.len());
    }
    lines.push(form_field_line(
        "    SHA256",
        &sha_display,
        false,
        field_idx == focused,
        FieldType::Text,
    ));
    field_idx += 1;

    // Auth type
    if field_idx == focused {
        focused_line = Some(lines.len());
    }
    lines.push(form_field_line(
        "    Auth Type",
        &format!("< {} >", artifact.auth_type.as_str()),
        false,
        field_idx == focused,
        FieldType::EnumAuthType,
    ));
    field_idx += 1;

    // Auth token (only if auth != None)
    if artifact.auth_type != AuthType::None {
        let masked = if artifact.auth_token.is_empty() {
            String::new()
        } else {
            "***".into()
        };
        if field_idx == focused {
            focused_line = Some(lines.len());
        }
        lines.push(form_field_line(
            "    Auth Token",
            &masked,
            false,
            field_idx == focused,
            FieldType::Text,
        ));
        field_idx += 1;
    }

    // Cache strategy
    if field_idx == focused {
        focused_line = Some(lines.len());
    }
    lines.push(form_field_line(
        "    Cache Strategy",
        &format!("< {} >", artifact.cache_strategy.as_str()),
        false,
        field_idx == focused,
        FieldType::EnumCacheStrategy,
    ));
    field_idx += 1;

    (field_idx, focused_line)
}

fn section_header(title: &str) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!("━━ {title} ━━"),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )])
}

fn form_field_line(
    label: &str,
    value: &str,
    required: bool,
    is_focused: bool,
    _field_type: FieldType,
) -> Line<'static> {
    let req = if required { " *" } else { "  " };
    let label_style = if is_focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let value_style = if is_focused {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default().fg(Color::White)
    };
    let cursor = if is_focused { "_" } else { "" };

    Line::from(vec![
        Span::styled(req.to_string(), Style::default().fg(Color::Red)),
        Span::styled(format!("{label}: "), label_style),
        Span::styled(format!("{value}{cursor}"), value_style),
    ])
}

fn format_user_data_field(path: &str, status: &FileStatus) -> String {
    let status_str = match status {
        FileStatus::NotSet => "",
        FileStatus::Loading => " (loading...)",
        FileStatus::Loaded(n) => return format!("{path} (loaded, {n} bytes)"),
        FileStatus::Error(e) => return format!("{path} (error: {e})"),
    };
    format!("{path}{status_str}")
}

fn build_form_help(app: &App) -> Line<'static> {
    let desc = form::field_descriptor(app, app.focused_field);

    let mut spans = vec![
        Span::styled("Tab/↑↓", Style::default().fg(Color::Yellow)),
        Span::raw(" Navigate  "),
    ];

    match desc.field_type {
        FieldType::Text => {
            spans.extend([
                Span::styled("Type", Style::default().fg(Color::Yellow)),
                Span::raw(" to edit  "),
                Span::styled("Ctrl+U", Style::default().fg(Color::Yellow)),
                Span::raw(" Clear  "),
            ]);
        }
        FieldType::Bool => {
            spans.extend([
                Span::styled("Space", Style::default().fg(Color::Yellow)),
                Span::raw(" Toggle  "),
            ]);
        }
        FieldType::EnumAuthType | FieldType::EnumCacheStrategy => {
            spans.extend([
                Span::styled("←→", Style::default().fg(Color::Yellow)),
                Span::raw(" Cycle  "),
            ]);
        }
        FieldType::FilePath => {
            spans.extend([
                Span::styled("Type", Style::default().fg(Color::Yellow)),
                Span::raw(" path  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(" Load file  "),
            ]);
        }
    }

    // Context-sensitive hints
    if matches!(
        desc.location,
        FieldLocation::Artifact { sub_field: 0, .. } | FieldLocation::Artifact { sub_field: 1, .. }
    ) {
        spans.extend([
            Span::styled("Ctrl+V", Style::default().fg(Color::Yellow)),
            Span::raw(" Validate/SHA  "),
        ]);
    }

    spans.extend([
        Span::styled("Ctrl+S", Style::default().fg(Color::Yellow)),
        Span::raw(" Review  "),
        Span::styled("Ctrl+A", Style::default().fg(Color::Yellow)),
        Span::raw(" Add  "),
        Span::styled("Ctrl+X", Style::default().fg(Color::Yellow)),
        Span::raw(" Remove  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" Back"),
    ]);

    if let Some(ref msg) = app.status_message {
        spans.push(Span::raw("  │ "));
        spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Magenta),
        ));
    }

    Line::from(spans)
}

// --- Phase 3: Review ---

fn draw_review(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(5),    // json preview
            Constraint::Length(3), // validation + help
        ])
        .split(frame.area());

    // Title
    let title = Paragraph::new(Line::from(vec![Span::styled(
        " Review CreateOperatingSystemRequest ",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )]));
    frame.render_widget(title, chunks[0]);

    // JSON preview
    let json = review::build_review_json(app);
    let line_count = json.lines().count();
    let inner_height = chunks[1].height.saturating_sub(2) as usize; // border top + bottom
    app.review_scroll_max = line_count.saturating_sub(inner_height);
    // Clamp scroll in case terminal was resized
    if app.review_scroll > app.review_scroll_max {
        app.review_scroll = app.review_scroll_max;
    }
    let json_widget = Paragraph::new(json)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" JSON Preview "),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.review_scroll as u16, 0));
    frame.render_widget(json_widget, chunks[1]);

    // Validation + help
    let errors = app.validate();
    let mut help_spans = Vec::new();
    if !errors.is_empty() {
        help_spans.push(Span::styled(
            format!("Errors: {}", errors.join("; ")),
            Style::default().fg(Color::Red),
        ));
        help_spans.push(Span::raw("  "));
    }
    if errors.is_empty() {
        help_spans.extend([
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" Submit  "),
        ]);
    }
    help_spans.extend([
        Span::styled("Ctrl+W", Style::default().fg(Color::Yellow)),
        Span::raw(" Save JSON  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(if errors.is_empty() {
            " Back to edit  "
        } else {
            " Back to fix  "
        }),
        Span::styled("↑↓", Style::default().fg(Color::Yellow)),
        Span::raw(" Scroll"),
    ]);
    if let Some(ref msg) = app.status_message {
        help_spans.push(Span::raw("  │ "));
        help_spans.push(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Magenta),
        ));
    }
    let help_line = Line::from(help_spans);
    let help = Paragraph::new(help_line).block(Block::default().borders(Borders::ALL));
    frame.render_widget(help, chunks[2]);
}

// --- Submitting ---

fn draw_submitting(frame: &mut Frame) {
    let area = centered_rect(40, 5, frame.area());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Creating OS Definition ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = Paragraph::new("Submitting...").style(Style::default().fg(Color::Yellow));
    frame.render_widget(text, inner);
}

// --- Result ---

fn draw_result(frame: &mut Frame, result: SubmitResult) {
    let area = centered_rect(60, 8, frame.area());
    let (title, body, color) = match &result {
        SubmitResult::Success { os_id, name } => (
            " Success ",
            format!(
                "OS created successfully!\n\nName: {name}\nID:   {os_id}\n\nPress Enter or q to exit."
            ),
            Color::Green,
        ),
        SubmitResult::Error(e) => (
            " Error ",
            format!("Failed to create OS:\n\n{e}\n\nPress Esc to go back, Enter/q to quit."),
            Color::Red,
        ),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(color));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = Paragraph::new(body)
        .style(Style::default().fg(color))
        .wrap(Wrap { trim: false });
    frame.render_widget(text, inner);
}

/// Create a centered rectangle of the given width and height within the parent area.
fn centered_rect(width: u16, height: u16, parent: Rect) -> Rect {
    let v_pad = parent.height.saturating_sub(height) / 2;
    let h_pad = parent.width.saturating_sub(width) / 2;
    Rect::new(
        parent.x + h_pad,
        parent.y + v_pad,
        width.min(parent.width),
        height.min(parent.height),
    )
}
