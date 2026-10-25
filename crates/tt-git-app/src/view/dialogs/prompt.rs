//! Prompts, confirms and discard-all: a title, lines of text and the keys.

use tt_git_core::{DiscardScope, SearchIn, StashScope, ignores_case, message_regex};

use super::{DialogLook, DialogView, Line, choice, cursor, separator};
use crate::modal::{Modal, PromptAction, worktree_rel_path};
use crate::view::{StyleRole, TextSpan};

impl Modal {
    /// What a prompt, confirm or discard-all shows in a screen `cols`
    /// columns across; `None` for the palette and the filter panel, which
    /// have views of their own.
    pub fn dialog_view(&self, cols: usize) -> Option<DialogView<'_>> {
        // The stash dialog's row of choices ends in a file name.
        let wide = matches!(
            self,
            Modal::Prompt {
                action: PromptAction::Stash { .. } | PromptAction::Agent { .. } | PromptAction::FilterMessage { .. },
                ..
            } | Modal::DiscardAll { .. }
        );
        let width = if wide { 72 } else { 60 }.min(cols);
        let inner_width = width.saturating_sub(2).max(1);
        let (title, body, hint) = match self {
            Modal::Palette { .. } | Modal::Filter(_) => return None,
            Modal::Prompt { title, input, action } => {
                let (body, hint) = prompt_body(input, action, inner_width);
                (title.clone(), body, hint)
            }
            Modal::Confirm {
                title, message, alt, ..
            } => (
                title.clone(),
                message.lines().map(|l| vec![TextSpan::plain(l)]).collect(),
                match alt.as_deref() {
                    Some((key, hint, _)) => format!("y confirm · {key} {hint} · n/esc cancel"),
                    None => "y confirm · n/esc cancel".into(),
                },
            ),
            Modal::DiscardAll { scope, counts, folder } => {
                let title = match folder {
                    Some(f) => format!("Discard all in {f}"),
                    None => "Discard all".into(),
                };
                let tab = |label: &'static str, s: DiscardScope| choice(label, s == *scope);
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
                    vec![
                        tab("Everything", DiscardScope::Everything),
                        separator(),
                        tab("Unstaged", DiscardScope::Unstaged),
                        separator(),
                        tab("Unstaged + untracked", DiscardScope::UnstagedAndUntracked),
                        separator(),
                        tab("Untracked", DiscardScope::Untracked),
                    ],
                    vec![TextSpan::new(what, StyleRole::Muted)],
                    vec![],
                ];
                if lost.is_empty() {
                    body.push(vec![TextSpan::plain("Nothing to discard here.")]);
                } else {
                    body.push(vec![TextSpan::plain(format!("Lost: {}.", lost.join(", ")))]);
                }
                let mut kept = counts.kept(*scope);
                if let Some(f) = folder {
                    kept.push(format!("everything outside {f}"));
                }
                body.push(vec![TextSpan::plain(format!("Kept: {}.", kept.join(", ")))]);
                let hint = if lost.is_empty() {
                    "tab what to discard · n/esc cancel"
                } else {
                    body.push(vec![TextSpan::new("This cannot be undone.", StyleRole::Error)]);
                    "y discard · tab what to discard · n/esc cancel"
                };
                (title, body, hint.to_string())
            }
        };
        Some(DialogView {
            title,
            look: DialogLook::Normal,
            width,
            body,
            hint: Some(hint),
        })
    }
}

/// A prompt's lines (the choices above the input, the input, a note under
/// it) and its keys.
fn prompt_body<'a>(input: &'a str, action: &'a PromptAction, inner_width: usize) -> (Vec<Line<'a>>, String) {
    let input_line = vec![TextSpan::plain(input), cursor()];
    let muted = |text: String| vec![TextSpan::new(text, StyleRole::Muted)];
    match action {
        PromptAction::Stash { scope, file, folder } => {
            let mut tabs = Vec::new();
            if let Some(f) = folder {
                let picked = matches!(scope, StashScope::Paths(p) if p.len() == 1 && p[0] == *f);
                tabs.extend([choice(f.as_str(), picked), separator()]);
            }
            tabs.extend([
                choice("Everything", *scope == StashScope::All),
                separator(),
                choice("Staged", *scope == StashScope::Staged),
                separator(),
                choice("Unstaged", *scope == StashScope::Unstaged),
                separator(),
                choice("Keep staged", *scope == StashScope::KeepStaged),
            ]);
            if let Some(paths) = file {
                let name = paths[0].rsplit('/').next().unwrap_or(&paths[0]);
                tabs.push(separator());
                tabs.push(choice(name, matches!(scope, StashScope::Paths(p) if p == paths)));
            }
            let note = match scope {
                StashScope::All => "staged, unstaged and untracked changes".to_string(),
                StashScope::Staged => "only what's staged".to_string(),
                StashScope::Unstaged => "unstaged and untracked changes; staged ones stay".to_string(),
                StashScope::KeepStaged => "everything, leaving staged changes in place".to_string(),
                StashScope::Paths(paths) => format!("only {}", paths.join(", ")),
            };
            (
                vec![tabs, input_line, muted(format!("{note} · message optional"))],
                "enter stash · tab what to stash · esc cancel".into(),
            )
        }
        PromptAction::CreateBranch { worktree, .. } => {
            let note = if *worktree {
                let name = input.trim();
                let name = if name.is_empty() { "<name>" } else { name };
                format!("checked out in {}", worktree_rel_path(name))
            } else {
                "switches to it here".to_string()
            };
            let tabs = vec![choice("Branch", !worktree), separator(), choice("Worktree", *worktree)];
            (
                vec![tabs, input_line, muted(note)],
                "enter confirm · tab branch/worktree · esc cancel".into(),
            )
        }
        PromptAction::CreateTag { annotated, .. } => {
            let note = if *annotated {
                "asks for a message next"
            } else {
                "just a name for the commit"
            };
            let tabs = vec![
                choice("Lightweight", !annotated),
                separator(),
                choice("Annotated", *annotated),
            ];
            let hint = if *annotated {
                "enter next · tab lightweight/annotated · esc cancel"
            } else {
                "enter create · tab lightweight/annotated · esc cancel"
            };
            (vec![tabs, input_line, muted(note.into())], hint.into())
        }
        PromptAction::Agent { actions, selected, .. } => {
            let mut tabs = vec![choice("Custom message", selected.is_none())];
            for (i, a) in actions.iter().enumerate() {
                tabs.push(separator());
                tabs.push(choice(a.name.as_str(), *selected == Some(i)));
            }
            let picked = selected.and_then(|i| actions.get(i));
            let note = match picked {
                Some(a) => {
                    let what = a.description.as_deref().unwrap_or(&a.name);
                    format!("{what} · anything typed is added to it")
                }
                None => "type what you want the agent to do".into(),
            };
            (
                vec![tabs, input_line, muted(note)],
                "enter start · tab ready-made actions · esc cancel".into(),
            )
        }
        PromptAction::FilterMessage { presets, filter } => {
            let typed = input.trim();
            let search_in = filter.search_in;
            let mut choices = vec![TextSpan::new("Search in: ", StyleRole::Muted)];
            for option in SearchIn::ALL {
                if choices.len() > 1 {
                    choices.push(separator());
                }
                let label = match option {
                    SearchIn::Subject => "Subject",
                    SearchIn::Message => "Whole message",
                    SearchIn::Diff => "Diff",
                };
                choices.push(choice(label, option == search_in));
            }
            let mut tabs = Vec::new();
            for p in presets {
                if !tabs.is_empty() {
                    tabs.push(separator());
                }
                tabs.push(choice(p.as_str(), p == typed));
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
                _ if typed.is_empty() => muted("empty shows every commit again".into()),
                Ok(()) => muted(format!("{what} · {case}")),
                Err(e) => {
                    let why = e.to_string();
                    // The last line of the error says what's wrong, after a
                    // drawing of where.
                    let last = why.lines().last().unwrap_or_default().trim();
                    let last = last.strip_prefix("error: ").unwrap_or(last);
                    vec![TextSpan::new(format!("invalid regex: {last}"), StyleRole::Error)]
                }
            };
            let mut body = vec![choices];
            if !tabs.is_empty() {
                // A dotted rule, so presets wrapped onto a second line
                // don't read as part of the input.
                body.extend([tabs, vec![TextSpan::new("┄".repeat(inner_width), StyleRole::Border)]]);
            }
            body.extend([input_line, note]);
            let hint = if presets.is_empty() {
                "enter filter · ctrl+t search in · esc cancel"
            } else {
                "enter filter · tab presets · ctrl+t search in · esc cancel"
            };
            (body, hint.into())
        }
        _ => (vec![input_line], "enter confirm · esc cancel".into()),
    }
}

#[cfg(test)]
mod tests {
    use tt_git_core::LogFilter;

    use super::*;
    use crate::modal::{DiscardCounts, GitOp};
    use crate::view::dialogs::line_text;

    fn texts(view: &DialogView) -> Vec<String> {
        view.body.iter().map(|l| line_text(l)).collect()
    }

    /// The labels of the choices in `line` that are picked.
    fn picked(line: &[TextSpan]) -> Vec<String> {
        line.iter()
            .filter(|s| s.over == Some(StyleRole::Underlined))
            .map(|s| s.text.to_string())
            .collect()
    }

    #[test]
    fn a_plain_prompt_is_the_input_and_a_cursor() {
        let modal = Modal::prompt(
            "Rename main",
            "trunk",
            PromptAction::RenameBranch { old: "main".into() },
        );
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(view.title, "Rename main");
        assert_eq!(view.width, 60);
        assert_eq!(texts(&view), ["trunk█"]);
        assert_eq!(view.body[0][1].role, StyleRole::HelpKey, "the cursor");
        assert_eq!(view.hint.as_deref(), Some("enter confirm · esc cancel"));
    }

    #[test]
    fn the_branch_prompt_says_where_a_worktree_goes() {
        let mut modal = Modal::prompt(
            "New branch",
            "",
            PromptAction::CreateBranch {
                start: "HEAD".into(),
                worktree: false,
            },
        );
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(picked(&view.body[0]), ["Branch"]);
        assert_eq!(texts(&view)[2], "switches to it here");
        modal.handle_key("tab".parse().unwrap());
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(picked(&view.body[0]), ["Worktree"]);
        assert_eq!(texts(&view)[2], "checked out in .worktrees/<name>");
        assert_eq!(view.body[2][0].role, StyleRole::Muted);
    }

    #[test]
    fn the_stash_prompt_is_wide_and_lists_the_file() {
        let modal = Modal::prompt(
            "Stash",
            "",
            PromptAction::Stash {
                scope: StashScope::Staged,
                file: Some(vec!["src/a.rs".into()]),
                folder: None,
            },
        );
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(view.width, 72);
        assert_eq!(texts(&view)[0], "Everything · Staged · Unstaged · Keep staged · a.rs");
        assert_eq!(picked(&view.body[0]), ["Staged"]);
        assert_eq!(texts(&view)[2], "only what's staged · message optional");
        assert_eq!(modal.dialog_view(40).unwrap().width, 40, "narrowed to the screen");
    }

    #[test]
    fn the_message_filter_checks_the_regex() {
        let modal = |input: &str, presets: Vec<String>| {
            Modal::prompt(
                "Filter by message",
                input,
                PromptAction::FilterMessage {
                    filter: LogFilter::default(),
                    presets,
                },
            )
        };
        let bad = modal("(", vec![]);
        let view = bad.dialog_view(80).unwrap();
        let note = view.body.last().unwrap();
        assert_eq!(note[0].role, StyleRole::Error);
        assert!(line_text(note).starts_with("invalid regex: "), "{note:?}");

        let presets = modal("fix", vec!["fix".into(), "feat".into()]);
        let view = presets.dialog_view(80).unwrap();
        let lines = texts(&view);
        assert_eq!(lines[0], "Search in: Subject · Whole message · Diff");
        assert_eq!(lines[1], "fix · feat");
        assert_eq!(picked(&view.body[1]), ["fix"]);
        assert_eq!(lines[2], "┄".repeat(70), "a rule across the inside");
        assert_eq!(lines[3], "fix█");
        assert_eq!(lines[4], "regex on the subject · ignores case");
        assert_eq!(
            view.hint.as_deref(),
            Some("enter filter · tab presets · ctrl+t search in · esc cancel")
        );
    }

    #[test]
    fn a_confirm_shows_its_message_and_second_answer() {
        let modal = Modal::confirm("Push", "Push main\nto origin?", GitOp::Push).confirm_or('f', "force", GitOp::Pull);
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(texts(&view), ["Push main", "to origin?"]);
        assert_eq!(view.hint.as_deref(), Some("y confirm · f force · n/esc cancel"));
    }

    #[test]
    fn discard_all_says_what_is_lost_and_kept() {
        let counts = DiscardCounts {
            staged: 1,
            unstaged: 2,
            untracked: 0,
            nested: 0,
        };
        let mut modal = Modal::discard_all(counts, Some("src/".into()));
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(view.title, "Discard all in src/");
        assert_eq!(picked(&view.body[0]), ["Everything"]);
        let lines = texts(&view);
        assert_eq!(lines[3], "Lost: 1 staged file, 2 unstaged files.");
        assert_eq!(lines[4], "Kept: ignored files, everything outside src/.");
        assert_eq!(view.body[5][0].role, StyleRole::Error);
        assert_eq!(
            view.hint.as_deref(),
            Some("y discard · tab what to discard · n/esc cancel")
        );

        // Untracked: nothing to lose here.
        for _ in 0..3 {
            modal.handle_key("tab".parse().unwrap());
        }
        let view = modal.dialog_view(200).unwrap();
        assert_eq!(picked(&view.body[0]), ["Untracked"]);
        assert_eq!(texts(&view)[3], "Nothing to discard here.");
        assert_eq!(view.body.len(), 5, "no warning");
        assert_eq!(view.hint.as_deref(), Some("tab what to discard · n/esc cancel"));
    }

    #[test]
    fn the_palette_and_filter_have_views_of_their_own() {
        assert_eq!(Modal::palette(vec![]).dialog_view(80), None);
    }
}
