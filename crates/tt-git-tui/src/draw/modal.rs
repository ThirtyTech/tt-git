//! Dialogs: prompts, confirmations, the command palette and pickers, the
//! keys popup and the error popup.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use tt_git_core::{DiscardScope, SearchIn, StashScope, ignores_case, message_regex};

use super::centered;
use super::filter::draw_filter;
use crate::app::App;
use crate::modal::{Modal, PALETTE_ROWS, PaletteItem, PromptAction, worktree_rel_path};
use crate::tui_theme::Theme;

pub fn draw_modal(modal: &Modal, frame: &mut Frame, area: Rect, theme: &Theme) {
    if let Modal::Palette {
        title,
        input,
        items,
        selected,
        lines,
    } = modal
    {
        return draw_palette(frame, area, theme, title, input, items, *selected, *lines);
    }
    if let Modal::Filter(panel) = modal {
        return draw_filter(panel, frame, area, theme);
    }
    let discard_title = match modal {
        Modal::DiscardAll { folder: Some(f), .. } => format!("Discard all in {f}"),
        _ => "Discard all".into(),
    };
    // The stash dialog's row of choices ends in a file name.
    let wide = matches!(
        modal,
        Modal::Prompt {
            action: PromptAction::Stash { .. } | PromptAction::Agent { .. } | PromptAction::FilterMessage { .. },
            ..
        } | Modal::DiscardAll { .. }
    );
    let width = if wide { 72 } else { 60 }.min(area.width);
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let (title, body, hint): (&str, Vec<Line>, String) = match modal {
        Modal::Palette { .. } | Modal::Filter(_) => unreachable!("handled above"),
        Modal::Prompt { title, input, action } => {
            let input_line = Line::from(vec![Span::raw(input.as_str()), Span::styled("█", theme.help_key)]);
            let tab = |label: String, on: bool| {
                if on {
                    Span::styled(label, theme.help_key.add_modifier(Modifier::UNDERLINED))
                } else {
                    Span::styled(label, theme.muted)
                }
            };
            match action {
                PromptAction::Stash { scope, file, folder } => {
                    let mut tabs = Vec::new();
                    if let Some(f) = folder {
                        let picked = matches!(scope, StashScope::Paths(p) if p.len() == 1 && p[0] == *f);
                        tabs.extend([tab(f.clone(), picked), Span::styled(" · ", theme.muted)]);
                    }
                    tabs.extend([
                        tab("Everything".into(), *scope == StashScope::All),
                        Span::styled(" · ", theme.muted),
                        tab("Staged".into(), *scope == StashScope::Staged),
                        Span::styled(" · ", theme.muted),
                        tab("Unstaged".into(), *scope == StashScope::Unstaged),
                        Span::styled(" · ", theme.muted),
                        tab("Keep staged".into(), *scope == StashScope::KeepStaged),
                    ]);
                    if let Some(paths) = file {
                        let name = paths[0].rsplit('/').next().unwrap_or(&paths[0]).to_string();
                        tabs.push(Span::styled(" · ", theme.muted));
                        tabs.push(tab(name, matches!(scope, StashScope::Paths(p) if p == paths)));
                    }
                    let note = match scope {
                        StashScope::All => "staged, unstaged and untracked changes".to_string(),
                        StashScope::Staged => "only what's staged".to_string(),
                        StashScope::Unstaged => "unstaged and untracked changes; staged ones stay".to_string(),
                        StashScope::KeepStaged => "everything, leaving staged changes in place".to_string(),
                        StashScope::Paths(paths) => format!("only {}", paths.join(", ")),
                    };
                    let body = vec![
                        Line::from(tabs),
                        input_line,
                        Line::styled(format!("{note} · message optional"), theme.muted),
                    ];
                    (title, body, "enter stash · tab what to stash · esc cancel".into())
                }
                PromptAction::CreateBranch { worktree, .. } => {
                    let note = if *worktree {
                        let name = input.trim();
                        let name = if name.is_empty() { "<name>" } else { name };
                        format!("checked out in {}", worktree_rel_path(name))
                    } else {
                        "switches to it here".to_string()
                    };
                    let body = vec![
                        Line::from(vec![
                            tab("Branch".into(), !worktree),
                            Span::styled(" · ", theme.muted),
                            tab("Worktree".into(), *worktree),
                        ]),
                        input_line,
                        Line::styled(note, theme.muted),
                    ];
                    (title, body, "enter confirm · tab branch/worktree · esc cancel".into())
                }
                PromptAction::CreateTag { annotated, .. } => {
                    let note = if *annotated {
                        "asks for a message next"
                    } else {
                        "just a name for the commit"
                    };
                    let body = vec![
                        Line::from(vec![
                            tab("Lightweight".into(), !annotated),
                            Span::styled(" · ", theme.muted),
                            tab("Annotated".into(), *annotated),
                        ]),
                        input_line,
                        Line::styled(note, theme.muted),
                    ];
                    let hint = if *annotated {
                        "enter next · tab lightweight/annotated · esc cancel"
                    } else {
                        "enter create · tab lightweight/annotated · esc cancel"
                    };
                    (title, body, hint.into())
                }
                PromptAction::Agent { actions, selected, .. } => {
                    let mut tabs = vec![tab("Custom message".into(), selected.is_none())];
                    for (i, a) in actions.iter().enumerate() {
                        tabs.push(Span::styled(" · ", theme.muted));
                        tabs.push(tab(a.name.clone(), *selected == Some(i)));
                    }
                    let picked = selected.and_then(|i| actions.get(i));
                    let note = match picked {
                        Some(a) => {
                            let what = a.description.clone().unwrap_or_else(|| a.name.clone());
                            format!("{what} · anything typed is added to it")
                        }
                        None => "type what you want the agent to do".into(),
                    };
                    let body = vec![Line::from(tabs), input_line, Line::styled(note, theme.muted)];
                    (title, body, "enter start · tab ready-made actions · esc cancel".into())
                }
                PromptAction::FilterMessage { presets, filter } => {
                    let typed = input.trim();
                    let search_in = filter.search_in;
                    let mut choices = vec![Span::styled("Search in: ", theme.muted)];
                    for choice in SearchIn::ALL {
                        if choices.len() > 1 {
                            choices.push(Span::styled(" · ", theme.muted));
                        }
                        let label = match choice {
                            SearchIn::Subject => "Subject",
                            SearchIn::Message => "Whole message",
                            SearchIn::Diff => "Diff",
                        };
                        choices.push(tab(label.into(), choice == search_in));
                    }
                    let mut tabs = Vec::new();
                    for p in presets {
                        if !tabs.is_empty() {
                            tabs.push(Span::styled(" · ", theme.muted));
                        }
                        tabs.push(tab(p.clone(), p == typed));
                    }
                    let what = match search_in {
                        SearchIn::Subject => "regex on the subject",
                        SearchIn::Message => "regex on the whole message, ^ and $ at each line",
                        SearchIn::Diff => "extended regex on lines added or removed",
                    };
                    let case = if ignores_case(typed) {
                        "ignores case"
                    } else {
                        "matches case"
                    };
                    let checked = match search_in {
                        SearchIn::Diff => Ok(()),
                        _ => message_regex(typed).map(drop),
                    };
                    let note = match checked {
                        _ if typed.is_empty() => Line::styled("empty shows every commit again", theme.muted),
                        Ok(()) => Line::styled(format!("{what} · {case}"), theme.muted),
                        Err(e) => {
                            let why = e.to_string();
                            // The last line of the error says what's wrong, after a
                            // drawing of where.
                            let last = why.lines().last().unwrap_or_default().trim();
                            let last = last.strip_prefix("error: ").unwrap_or(last);
                            Line::styled(format!("invalid regex: {last}"), theme.error)
                        }
                    };
                    let mut body = vec![Line::from(choices)];
                    if !tabs.is_empty() {
                        // A dotted rule, so presets wrapped onto a second line
                        // don't read as part of the input.
                        body.extend([Line::from(tabs), Line::styled("┄".repeat(inner_width), theme.border)]);
                    }
                    body.extend([input_line, note]);
                    let hint = if presets.is_empty() {
                        "enter filter · ctrl+t search in · esc cancel"
                    } else {
                        "enter filter · tab presets · ctrl+t search in · esc cancel"
                    };
                    (title, body, hint.into())
                }
                _ => (title, vec![input_line], "enter confirm · esc cancel".into()),
            }
        }
        Modal::Confirm {
            title, message, alt, ..
        } => (
            title,
            message.lines().map(Line::from).collect(),
            match alt.as_deref() {
                Some((key, hint, _)) => format!("y confirm · {key} {hint} · n/esc cancel"),
                None => "y confirm · n/esc cancel".into(),
            },
        ),
        Modal::DiscardAll { scope, counts, folder } => {
            let tab = |label: &'static str, s: DiscardScope| {
                if s == *scope {
                    Span::styled(label, theme.help_key.add_modifier(Modifier::UNDERLINED))
                } else {
                    Span::styled(label, theme.muted)
                }
            };
            let sep = || Span::styled(" · ", theme.muted);
            let what = match scope {
                DiscardScope::Everything => "every file back to HEAD, untracked files deleted",
                DiscardScope::Unstaged => "files back to what's staged; staged changes and untracked files stay",
                DiscardScope::UnstagedAndUntracked => {
                    "files back to what's staged, untracked files deleted; staged changes stay"
                }
                DiscardScope::Untracked => "untracked files deleted; changes to tracked files stay",
            };
            let lost = counts.lost(*scope);
            let mut body = vec![
                Line::from(vec![
                    tab("Everything", DiscardScope::Everything),
                    sep(),
                    tab("Unstaged", DiscardScope::Unstaged),
                    sep(),
                    tab("Unstaged + untracked", DiscardScope::UnstagedAndUntracked),
                    sep(),
                    tab("Untracked", DiscardScope::Untracked),
                ]),
                Line::styled(what, theme.muted),
                Line::raw(""),
            ];
            if lost.is_empty() {
                body.push(Line::raw("Nothing to discard here."));
            } else {
                body.push(Line::raw(format!("Lost: {}.", lost.join(", "))));
            }
            let mut kept = counts.kept(*scope);
            if let Some(f) = folder {
                kept.push(format!("everything outside {f}"));
            }
            body.push(Line::raw(format!("Kept: {}.", kept.join(", "))));
            let hint = if lost.is_empty() {
                "tab what to discard · n/esc cancel"
            } else {
                body.push(Line::styled("This cannot be undone.", theme.error));
                "y discard · tab what to discard · n/esc cancel"
            };
            (&discard_title, body, hint.into())
        }
    };
    let body_rows: usize = body.iter().map(|l| l.width().div_ceil(inner_width).max(1)).sum();
    let height = (body_rows as u16 + 4).min(area.height);
    let popup = crate::draw::centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(format!(" {title} "), theme.title_focused));
    let mut lines = body;
    lines.push(Line::raw(""));
    lines.push(Line::styled(hint, theme.muted));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), popup);
}

#[allow(clippy::too_many_arguments)]
fn draw_palette(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    input: &str,
    items: &[PaletteItem],
    selected: usize,
    line_count: usize,
) {
    // (label, hint) per row. A line number is the one thing to do, in
    // place of the commands.
    let (matches, selected) = match Modal::palette_line(input, line_count) {
        Some(line) => (vec![(format!("Go to line {line}"), format!("of {line_count}"))], 0),
        None => (
            Modal::palette_matches(items, input)
                .into_iter()
                .map(|i| (i.label.clone(), i.hint.clone()))
                .collect(),
            selected,
        ),
    };
    let rows = matches.len().clamp(1, PALETTE_ROWS);
    let width = 70.min(area.width);
    let height = (rows as u16 + 4).min(area.height);
    let popup = Rect {
        y: area.y + area.height.saturating_sub(height) / 4,
        ..crate::draw::centered(area, width, height)
    };
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(format!(" {title} "), theme.title_focused));
    let inner_width = popup.width.saturating_sub(2) as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("> ", theme.help_key),
            Span::raw(input.to_string()),
            Span::styled("█", theme.help_key),
        ]),
        Line::raw(""),
    ];
    let start = selected.saturating_sub(PALETTE_ROWS - 1);
    if matches.is_empty() {
        let empty = if items.is_empty() && line_count > 0 {
            "type a line number"
        } else {
            "no matches"
        };
        lines.push(Line::styled(empty, theme.muted));
    }
    for (i, (label, hint)) in matches.into_iter().enumerate().skip(start).take(PALETTE_ROWS) {
        let hint_w = hint.chars().count();
        let label_w = inner_width.saturating_sub(hint_w + 1);
        let line = Line::from(vec![
            Span::raw(crate::widgets::fit(&label, label_w)),
            Span::raw(" "),
            Span::styled(hint, theme.muted),
        ]);
        lines.push(if i == selected {
            line.patch_style(theme.selected)
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The keys popup, narrowed to what's typed.
pub(super) fn draw_help_popup(app: &App, frame: &mut Frame, area: Rect, theme: &Theme) {
    let search = if app.help_query().is_empty() {
        Line::from(vec![
            Span::styled("> ", theme.help_key),
            Span::styled("type to search", theme.muted),
        ])
    } else {
        Line::from(vec![
            Span::styled("> ", theme.help_key),
            Span::raw(app.help_query().to_string()),
            Span::styled("█", theme.help_key),
        ])
    };
    let mut lines = vec![search];
    let rows = app.help_rows();
    if rows.is_empty() {
        lines.push(Line::styled("no matching keys", theme.muted));
    }
    for (keys, label) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("{keys:>16}  "), theme.help_key),
            Span::styled(label, theme.help_text),
        ]));
    }

    // Size for the unfiltered list so the popup holds still while typing.
    let popup = centered(area, 50, app.all_help_row_count() as u16 + 3);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border_focused)
        .title(Span::styled(" Keybindings ", theme.title_focused))
        .title_top(Line::styled(concat!(" tt-git ", env!("CARGO_PKG_VERSION"), " "), theme.muted).right_aligned());
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The error popup.
pub(super) fn draw_error_popup(frame: &mut Frame, area: Rect, msg: &str, theme: &Theme) {
    let width = 70.min(area.width);
    let inner_width = width.saturating_sub(2).max(1) as usize;
    let text_lines: usize = msg
        .lines()
        .map(|l| l.chars().count().div_ceil(inner_width).max(1))
        .sum();
    let popup = centered(area, width, text_lines as u16 + 2);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.error)
        .title(Span::styled(" Error ", theme.error));
    frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }).block(block), popup);
}
