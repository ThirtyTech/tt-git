//! Background jobs that read from git and report back as actions.

use tt_git_core::patch::FileDiff;
use tt_git_core::{BaseBranches, BranchEntry, CommitFile, GitError, GitRef, LogFilter, LogScope, RemoteConfig, Repo};

use crate::action::{Action, RefsSnapshot};
use crate::config::BranchesConfig;
use std::path::PathBuf;

use crate::editor::{EditTarget, Snapshot, abs_path, write_snapshot};
use crate::filter::{FilterTab, author_choices, file_choices};
use crate::jobs::Ctx;
use crate::modal::GitOp;
use crate::panes::{BranchFocus, DetailsSubject, GraphScope, LoadToken};
use crate::syntax::{self, LineTokens};
use crate::widgets::{expand_tabs, split_lines};

/// Commits per `GraphBatch`, so the first screenful appears quickly.
const GRAPH_BATCH: usize = 1000;
/// Diff output beyond this many lines is cut off in the details pane.
const MAX_DETAIL_LINES: usize = 5000;

fn message(e: GitError) -> String {
    e.to_string()
}

/// The code in `text`'s diffs, as the details pane splits it into lines,
/// if `highlight`.
fn diff_syntax(text: &str, highlight: bool) -> LineTokens {
    if highlight {
        syntax::diff_text(&split_lines(text))
    } else {
        Vec::new()
    }
}

/// `config` says which branches the merged and unmerged lists leave out.
pub fn load_refs(repo: Repo, config: BranchesConfig) -> Action {
    let result = (|| {
        let head = repo.head()?;
        let RemoteConfig { remotes, base_branch } = repo.remote_config()?;
        let (refs, authors) = repo.refs_with_authors(&remotes)?;
        let checkouts = repo.checkouts(500)?;
        let stashes = repo.stashes().unwrap_or_else(|e| {
            tracing::warn!("listing stashes failed: {e}");
            Vec::new()
        });
        let worktrees = repo.worktrees().unwrap_or_else(|e| {
            tracing::warn!("listing worktrees failed: {e}");
            Vec::new()
        });
        let branches = tt_git_core::branches::unify(&refs, &head, &checkouts, |r| repo.ref_activity(r));
        let base = match load_base(&repo, &refs, &authors, &branches, base_branch, &config) {
            Ok(base) => base,
            Err(e) => {
                tracing::warn!("comparing branches with the base failed: {e}");
                None
            }
        };
        Ok(RefsSnapshot {
            head,
            refs,
            checkouts,
            branches,
            base,
            stashes,
            worktrees,
        })
    })();
    Action::RefsLoaded(result.map(Box::new).map_err(message))
}

/// Splits the branches by whether they're merged into the base branch.
/// `authors` are the refs' authors as [`Repo::refs_with_authors`] read
/// them; `setting` is the picked base branch.
fn load_base(
    repo: &Repo,
    refs: &[GitRef],
    authors: &[(String, String)],
    branches: &[BranchEntry],
    setting: Option<String>,
    config: &BranchesConfig,
) -> Result<Option<BaseBranches>, GitError> {
    use tt_git_core::branches::{
        TAGS_COMPARED, base_scope, compare_with_base, default_base, resolve_base, tags_newest_first,
    };

    let Some(base) = resolve_base(refs, setting.as_deref()) else {
        return Ok(None);
    };
    let detected = default_base(refs).map(|r| r.short_name.clone());
    let scope = base_scope(base);
    let merged = repo.merged_refs(&base.full_name, &scope)?;
    let (unmerged, merged) =
        compare_with_base(branches, base, &merged, &config.unmerged_exclude, &config.safe_branches);
    let tags: Vec<GitRef> = tags_newest_first(refs).into_iter().take(TAGS_COMPARED).collect();
    // All in one git when it can, one each otherwise.
    let pairs: Vec<(&str, &str)> = tags.iter().map(|t| (t.full_name.as_str(), t.target.as_str())).collect();
    let compared = if pairs.is_empty() {
        None
    } else {
        repo.ahead_behind_many(&base.full_name, &pairs)
    }
    .unwrap_or_default();
    let tags = tags
        .into_iter()
        .filter_map(|tag| {
            if let Some(distance) = compared.get(&tag.full_name) {
                return Some((tag.full_name, *distance));
            }
            match repo.ahead_behind(&base.full_name, &tag.target) {
                Ok(distance) => Some((tag.full_name, distance)),
                Err(e) => {
                    tracing::warn!("comparing {} with the base failed: {e}", tag.short_name);
                    None
                }
            }
        })
        .collect();
    Ok(Some(BaseBranches {
        base: base.short_name.clone(),
        base_ref: base.full_name.clone(),
        chosen: setting.is_some_and(|s| s == base.short_name || s == base.full_name),
        detected,
        unmerged,
        merged,
        authors: repo.ref_authors_of(authors, &scope)?,
        tags,
    }))
}

/// How many files have changes in each of the worktrees at `paths`.
pub fn count_worktree_changes(repo: Repo, paths: Vec<PathBuf>) -> Action {
    let counts = paths
        .into_iter()
        .map(|path| {
            let count = repo.worktree_changes(&path).map_err(|e| {
                let msg = e.stderr().map(str::to_string).unwrap_or_else(|| e.to_string());
                tracing::warn!("counting changes in {} failed: {msg}", path.display());
                msg
            });
            (path, count)
        })
        .collect();
    Action::WorktreeChanges(counts)
}

/// How much disk space the worktree at `path` takes, leaving out the
/// worktrees in `nested` (`.worktrees/x` in the main one) so they count once.
pub fn measure_worktree_size(path: PathBuf, nested: Vec<PathBuf>) -> Action {
    let size = tt_git_core::repo::disk_usage(&path, &nested).map_err(|e| {
        tracing::warn!("measuring {} failed: {e}", path.display());
        e.to_string()
    });
    Action::WorktreeSize(path, size)
}

/// Everyone with commits, for the filter panel's author list.
pub fn load_filter_authors(repo: Repo) -> Action {
    Action::FilterChoicesLoaded {
        tab: FilterTab::Author,
        choices: repo.authors().map(author_choices).map_err(message),
    }
}

/// Tracked files and their directories, for the filter panel's file list.
pub fn load_filter_files(repo: Repo) -> Action {
    Action::FilterChoicesLoaded {
        tab: FilterTab::File,
        choices: repo.tracked_files().map(file_choices).map_err(message),
    }
}

pub fn load_status(repo: Repo) -> Action {
    Action::StatusLoaded(repo.status().map_err(message))
}

/// Where a "load more" picks up: past the `skip` commits shown, which a
/// load that walked from `tips` listed.
#[derive(Debug, Clone)]
pub struct Continue {
    pub skip: usize,
    pub tips: Vec<u8>,
}

/// Streams the graph's commits in batches, ending with a `done` batch; or,
/// with a `focus`, loads just that branch's history in one go. A `filter`
/// (ignored with a `focus`) also loads in one go, since parents it leaves
/// out are only known at the end. Streaming stops early (killing git) once
/// `load` is superseded, since its batches would be dropped. Without a
/// `focus`, the load first records on `load` what it walks from.
///
/// With `from` (whole graph, no filter), streams only the commits after
/// those already shown if it still walks from the same tips; otherwise it
/// sends [`Action::GraphRestarted`] and loads them all.
#[allow(clippy::too_many_arguments)]
pub fn load_graph(
    repo: Repo,
    scope: GraphScope,
    focus: Option<BranchFocus>,
    filter: LogFilter,
    load: LoadToken,
    limit: usize,
    from: Option<Continue>,
    ctx: &Ctx,
) -> Action {
    let generation = load.generation();
    if let Some(focus) = focus {
        return match repo.branch_history(&focus.tip, &focus.base_ref, limit) {
            Ok(history) => Action::BranchHistoryLoaded { generation, history },
            Err(e) => Action::GraphFailed {
                generation,
                error: message(e),
            },
        };
    }
    let upstream = match scope {
        GraphScope::All => None,
        GraphScope::Current => head_upstream(&repo),
    };
    let tips = tips_of(&repo, scope, upstream.as_deref());
    let skip = match from {
        Some(from) if filter.is_empty() && tips.as_deref() == Some(from.tips.as_slice()) => from.skip,
        Some(_) => {
            ctx.emit(Action::GraphRestarted { generation });
            0
        }
        None => 0,
    };
    load.set_tips(tips);
    let scope = match scope {
        GraphScope::All => LogScope::All,
        GraphScope::Current => LogScope::Head { upstream },
    };
    let result = log_graph(&repo, &scope, &filter, &load, skip, limit, ctx);
    if matches!(result, Action::GraphFailed { .. }) {
        // So the next refs change tries again.
        load.set_tips(None);
    }
    result
}

/// The rest of [`load_graph`] for a whole-graph `scope`, leaving out the
/// first `skip` commits (only without a `filter`).
fn log_graph(
    repo: &Repo,
    scope: &LogScope,
    filter: &LogFilter,
    load: &LoadToken,
    skip: usize,
    limit: usize,
    ctx: &Ctx,
) -> Action {
    let generation = load.generation();
    if !filter.is_empty() {
        return match repo.filtered_commits(scope, filter, limit) {
            Ok(commits) => Action::GraphBatch {
                generation,
                commits,
                done: true,
            },
            Err(e) => Action::GraphFailed {
                generation,
                error: message(e),
            },
        };
    }
    let mut batch = Vec::with_capacity(GRAPH_BATCH);
    let result = repo.stream_commits_from(scope, skip, limit, |commit| {
        if !load.is_current() {
            return false;
        }
        batch.push(commit);
        if batch.len() == GRAPH_BATCH {
            ctx.emit(Action::GraphBatch {
                generation,
                commits: std::mem::replace(&mut batch, Vec::with_capacity(GRAPH_BATCH)),
                done: false,
            });
        }
        true
    });
    match result {
        Ok(()) => Action::GraphBatch {
            generation,
            commits: batch,
            done: true,
        },
        Err(e) => Action::GraphFailed {
            generation,
            error: message(e),
        },
    }
}

/// Checks what a whole-graph load of `scope` would walk from now, for
/// comparing with what load `generation` did.
pub fn graph_tips(repo: Repo, scope: GraphScope, generation: u64) -> Action {
    let upstream = match scope {
        GraphScope::All => None,
        GraphScope::Current => head_upstream(&repo),
    };
    Action::GraphTipsChecked {
        generation,
        tips: tips_of(&repo, scope, upstream.as_deref()),
    }
}

/// Everything a whole-graph log of `scope` depends on besides the commits
/// themselves: the refs it starts from (HEAD, its upstream and every other
/// ref, as stored) and where history is cut short, plus the scope and the
/// upstream's name. Files only, so it adds no git process before the log.
fn tips_of(repo: &Repo, scope: GraphScope, upstream: Option<&str>) -> Option<Vec<u8>> {
    let Some(refs) = repo.ref_fingerprint() else {
        tracing::debug!("couldn't fingerprint the graph's refs");
        return None;
    };
    let head = match scope {
        GraphScope::All => "all".to_string(),
        GraphScope::Current => format!("current {}", upstream.unwrap_or("")),
    };
    let mut tips = head.into_bytes();
    tips.push(0);
    tips.extend(refs);
    Some(tips)
}

/// Full ref name of HEAD's upstream, if it has one that still exists.
fn head_upstream(repo: &Repo) -> Option<String> {
    let out = repo
        .git()
        .run(&["rev-parse", "--symbolic-full-name", "--verify", "-q", "@{upstream}"])
        .ok()?;
    let name = out.stdout_str().trim().to_string();
    (!name.is_empty()).then_some(name)
}

const COMMIT_HEADER_FORMAT: &str =
    "--format=commit %H%nParents: %P%nAuthor: %aN <%aE>%nDate:   %ad%nCommitter: %cN <%cE>%n%n%w(0,4,4)%B";

pub fn load_commit_details(repo: Repo, oid: String, highlight: bool) -> Action {
    let text = match repo.git().run(&[
        "show",
        COMMIT_HEADER_FORMAT,
        "--date=iso",
        "--stat",
        "--patch",
        "--diff-merges=first-parent",
        "-M",
        &oid,
        "--",
    ]) {
        Ok(out) => cap_lines(out.into_stdout_string()),
        Err(e) => format!("could not load commit: {e}"),
    };
    Action::DetailsLoaded {
        subject: DetailsSubject::Commit(oid),
        syntax: diff_syntax(&text, highlight),
        text,
    }
}

/// The commit header and changed files for the details files view.
pub fn load_commit_files(repo: Repo, oid: String) -> Action {
    let header = match repo.git().run(&[
        "show",
        "-s",
        "--format=%s%n%h · %aN <%aE> · %ad%n%n%b",
        "--date=format:%Y-%m-%d %H:%M",
        &oid,
        "--",
    ]) {
        Ok(out) => out.into_stdout_string(),
        Err(e) => format!("could not load commit: {e}"),
    };
    let files = repo.commit_files(&oid).map_err(|e| e.to_string());
    Action::CommitFilesLoaded { oid, header, files }
}

pub fn load_commit_file_diff(repo: Repo, oid: String, file: CommitFile, highlight: bool) -> Action {
    let text = match repo.commit_file_diff(&oid, &file) {
        Ok(t) if t.trim().is_empty() => format!("{}: no textual changes", file.path),
        Ok(t) => cap_lines(t),
        Err(e) => format!("could not diff {}: {e}", file.path),
    };
    Action::CommitFileDiffLoaded {
        oid,
        path: file.path,
        syntax: diff_syntax(&text, highlight),
        text,
    }
}

/// `text`'s lines (as [`str::lines`] splits them) joined with `\n`, cut off
/// after [`MAX_DETAIL_LINES`] with a note of how many more there were.
fn cap_lines(text: String) -> String {
    cap_lines_at(text, MAX_DETAIL_LINES)
}

fn cap_lines_at(mut text: String, max: usize) -> String {
    // Without a '\r' each line ends at a '\n' with nothing to strip, so
    // text short enough just loses its final line ending, in place.
    if !text.contains('\r') {
        let newlines = text.bytes().filter(|&b| b == b'\n').count();
        let lines = newlines + usize::from(!text.is_empty() && !text.ends_with('\n'));
        if lines <= max {
            if text.ends_with('\n') {
                text.pop();
            }
            return text;
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut lines = text.lines();
    for (i, line) in lines.by_ref().take(max).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    let rest = lines.count();
    if rest > 0 {
        if max > 0 {
            out.push('\n');
        }
        out.push_str(&format!("… {rest} more lines not shown"));
    }
    out
}

pub fn run_op(repo: Repo, op: GitOp) -> Action {
    let result = match &op {
        GitOp::Checkout { plan, stash: false } => repo.checkout(plan),
        GitOp::Checkout { plan, stash: true } => repo.stash_and_checkout(plan),
        GitOp::CreateBranch {
            name,
            start,
            worktree: false,
        } => repo.create_branch(name, start),
        GitOp::CreateBranch {
            name,
            start,
            worktree: true,
        } => repo.create_worktree(name, start),
        GitOp::DeleteBranch { name, force } => repo.delete_branch(name, *force),
        GitOp::DeleteRemoteBranch { remote, branch } => repo.delete_remote_branch(remote, branch),
        GitOp::DeleteTag { name, remote: None } => repo.delete_tag(name),
        GitOp::DeleteTag {
            name,
            remote: Some(remote),
        } => repo.delete_remote_tag(remote, name).and_then(|there| {
            Ok(format!(
                "{there}
{}",
                repo.delete_tag(name)?
            ))
        }),
        GitOp::CreateTag { name, target, message } => repo.create_tag(name, target, message.as_deref()),
        GitOp::PushTag { name } => repo.push_tag(name),
        GitOp::RenameBranch { old, new } => repo.rename_branch(old, new),
        GitOp::FetchAll => repo.fetch_all(),
        GitOp::SetBaseBranch(base) => repo.set_base_branch(base.as_deref()),
        GitOp::Stage(paths) => repo.stage(paths),
        GitOp::Unstage(paths) => repo.unstage(paths),
        GitOp::StageAll => repo.stage_all(),
        GitOp::UnstageAll => repo.unstage_all(),
        GitOp::Discard { path, kind } => repo.discard(path, kind),
        GitOp::ApplyPatch {
            path,
            patch,
            op,
            untracked,
            ..
        } => repo.apply_patch(path, patch, *op, *untracked),
        GitOp::Commit { message, amend } => repo.commit(message.as_deref(), *amend),
        GitOp::Push => repo.push(),
        GitOp::Pull => repo.pull(),
        GitOp::Merge { target } => repo.merge(target),
        GitOp::Rebase { onto } => repo.rebase(onto),
        GitOp::CherryPick { oid, .. } => repo.cherry_pick(oid),
        GitOp::Continue(op) => repo.continue_operation(*op),
        GitOp::Abort(op) => repo.abort_operation(*op),
        GitOp::StashPush { message, scope } => repo.stash_push(message.as_deref(), scope),
        GitOp::StashApply { index, pop } => repo.stash_apply(*index, *pop),
        GitOp::StashDrop { index } => repo.stash_drop(*index),
        GitOp::StashBranch { name, index } => repo.stash_branch(name, *index),
        GitOp::Undo { step, stash } => repo.apply_undo(step, *stash),
        GitOp::RemoveWorktree { path, force, .. } => repo.remove_worktree(path, *force),
    };
    let head = match result {
        Ok(_) => repo.head().ok().and_then(|h| h.oid().map(str::to_string)),
        Err(_) => None,
    };
    Action::OpDone {
        op,
        result: result.map_err(|e| e.stderr().map(str::to_string).unwrap_or_else(|| e.to_string())),
        head,
    }
}

/// Works out the next undo step.
pub fn plan_undo(repo: Repo) -> Action {
    Action::UndoPlanned(repo.undo_step().map_err(|e| e.to_string()))
}

/// Largest untracked file shown in full in the details pane.
const MAX_UNTRACKED_PREVIEW: usize = 256 * 1024;

pub fn load_file_diff(repo: Repo, entry: tt_git_core::StatusEntry, highlight: bool) -> Action {
    let text = match repo.file_diff(&entry, MAX_UNTRACKED_PREVIEW) {
        Ok(t) if t.trim().is_empty() => format!("{}: no textual changes", entry.path),
        Ok(t) => cap_lines(t),
        Err(e) => format!("could not diff {}: {e}", entry.path),
    };
    Action::DetailsLoaded {
        subject: DetailsSubject::File(entry.path),
        syntax: diff_syntax(&text, highlight),
        text,
    }
}

/// What a stash holds, for the details pane.
pub fn load_stash_details(repo: Repo, stash: tt_git_core::Stash, highlight: bool) -> Action {
    let text = match repo.stash_details(&stash) {
        Ok(t) => cap_lines(t),
        Err(e) => format!("could not show {}: {e}", stash.name()),
    };
    Action::DetailsLoaded {
        subject: DetailsSubject::Stash(stash.oid),
        syntax: diff_syntax(&text, highlight),
        text,
    }
}

/// Both sides of `entry`'s changes, for the staging view.
pub fn load_stage_diffs(repo: Repo, entry: tt_git_core::StatusEntry, highlight: bool) -> Action {
    let side = |staged| {
        repo.stage_diff(&entry, staged, MAX_UNTRACKED_PREVIEW)
            .unwrap_or_else(|e| Err(format!("could not diff {}: {e}", entry.path)))
    };
    let (unstaged, staged) = (side(false), side(true));
    let code = |side: &tt_git_core::StageSide| match side {
        Ok(diff) if highlight => syntax::hunks(&entry.path, diff),
        _ => Vec::new(),
    };
    let syntax = (code(&unstaged), code(&staged));
    Action::StageDiffsLoaded {
        entry,
        unstaged,
        staged,
        syntax,
    }
}

/// `entry` from the status pane, to open in the editor at its first
/// conflict marker if it's conflicted, else its first unstaged change, else
/// its first staged one (carried through the unstaged changes).
pub fn status_edit_target(repo: Repo, entry: tt_git_core::StatusEntry) -> Action {
    let line = if entry.is_untracked() {
        None
    } else if entry.is_conflicted() {
        std::fs::read(abs_path(repo.root(), &entry.path))
            .ok()
            .and_then(|text| text.split(|b| *b == b'\n').position(|l| l.starts_with(b"<<<<<<<")))
            .map(|i| i as u32 + 1)
    } else {
        let side = |staged| {
            repo.stage_diff(&entry, staged, MAX_UNTRACKED_PREVIEW)
                .ok()
                .and_then(Result::ok)
        };
        let unstaged = side(false);
        unstaged.as_ref().and_then(FileDiff::first_change_line).or_else(|| {
            let line = side(true)?.first_change_line()?;
            Some(unstaged.as_ref().map_or(line, |u| u.old_to_new(line)))
        })
    };
    Action::OpenInEditor(EditTarget {
        path: entry.path,
        line,
        rev: None,
    })
}

/// A read-only copy under `dir` of `target`'s file as of its commit, for a
/// file that isn't in the working tree.
pub fn edit_snapshot(repo: Repo, dir: PathBuf, target: EditTarget) -> Action {
    let rev = target.rev.as_deref().unwrap_or("HEAD");
    let path = target.path;
    let result = repo
        .file_at(rev, &path)
        .map_err(|e| {
            format!(
                "couldn't read {path} from {rev}: {}",
                e.stderr().unwrap_or(&e.to_string()).trim()
            )
        })
        .and_then(|(contents, commit)| {
            let file = write_snapshot(&dir, &commit, &path, &contents)
                .map_err(|e| format!("couldn't write a copy of {path}: {e}"))?;
            Ok(Snapshot {
                path,
                commit,
                file,
                line: target.line,
            })
        });
    Action::EditSnapshot(result)
}

/// Blames `path` as of `rev` (the working tree when `None`).
pub fn load_blame(repo: Repo, id: u64, rev: Option<String>, path: String, highlight: bool) -> Action {
    let result = repo.blame(rev.as_deref(), &path).map_err(message);
    let syntax = match &result {
        Ok(blame) if highlight => {
            let lines: Vec<_> = blame.lines.iter().map(|l| expand_tabs(&l.text)).collect();
            syntax::file(&path, &lines)
        }
        _ => Vec::new(),
    };
    let changed = match &result {
        Ok(_) => repo.file_changed_at(rev.as_deref(), &path).ok().flatten(),
        Err(_) => None,
    };
    Action::BlameLoaded {
        id,
        result,
        syntax,
        changed,
    }
}

/// `oid`'s message below its subject line.
pub fn load_commit_message(repo: Repo, oid: String) -> Action {
    let body = match repo.git().run(&["show", "-s", "--format=%b", &oid, "--"]) {
        Ok(out) => out.into_stdout_string().trim_end().to_string(),
        Err(e) => format!("could not load the message: {}", message(e)),
    };
    Action::CommitMessageLoaded { oid, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cap_lines_at` as it was: collect the lines, then join them.
    fn joined(text: &str, max: usize) -> String {
        let mut lines = text.lines();
        let mut out: Vec<&str> = lines.by_ref().take(max).collect();
        let rest = lines.count();
        let note;
        if rest > 0 {
            note = format!("… {rest} more lines not shown");
            out.push(&note);
        }
        out.join("\n")
    }

    #[test]
    fn cap_lines_matches_joining_the_lines() {
        let texts = [
            "",
            "\n",
            "\n\n",
            "a",
            "a\n",
            "a\nb",
            "a\nb\n",
            "a\n\n",
            "a\nb\nc\n",
            "a\r\nb\r\n",
            "a\r\nb",
            "a\rb\n",
            "a\r",
            "\r\n",
            "\r",
            "a\nb\r\nc\n\n\nd",
            "ü\n€\n\u{1f600}\n",
        ];
        for text in texts {
            for max in 0..6 {
                assert_eq!(
                    cap_lines_at(text.to_string(), max),
                    joined(text, max),
                    "{text:?} max {max}"
                );
            }
        }
    }
}
