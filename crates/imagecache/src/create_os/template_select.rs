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

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;
use rpc::forge::IpxeTemplate;

/// Build list items for the filtered templates.
pub fn template_list_items(
    templates: &[IpxeTemplate],
    filtered_indices: &[usize],
) -> Vec<ListItem<'static>> {
    filtered_indices
        .iter()
        .map(|&i| {
            let t = &templates[i];
            let name = Span::styled(
                t.name.clone(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            );
            let desc = if t.description.is_empty() {
                Span::raw("")
            } else {
                Span::styled(
                    format!(" — {}", t.description),
                    Style::default().fg(Color::DarkGray),
                )
            };
            ListItem::new(Line::from(vec![name, desc]))
        })
        .collect()
}

/// Wrap-around navigation for a list of the given length.
pub fn wrap_nav(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match current {
        Some(i) => {
            if forward {
                if i + 1 >= len { 0 } else { i + 1 }
            } else if i == 0 {
                len - 1
            } else {
                i - 1
            }
        }
        None => 0,
    })
}
