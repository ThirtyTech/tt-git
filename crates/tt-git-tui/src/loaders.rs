//! Background jobs that read from git and report back as actions.

use tt_git_core::{BaseBranches, BranchEntry, CommitFile, GitError, GitRef, LogScope, Repo};

use crate::action::{Action, RefsSnapshot};
use crate::jobs::Ctx;
use crate::modal::GitOp;
use crate::panes::{BranchFocus, DetailsSubject, GraphScope};

/// Commits per `GraphBatch`, so the first screenful appears quickly.
const GRAPH_BATCH: usize = 1000;
/// Diff output beyond this many lines is cut off in the details pane.
const MAX_DETAIL_LINES: usize = 5000;

fn message(e: GitError) -> String {
    e.to_string()
}

/// `exclude` hides unmerged branches whose name contains any of its strings.
pub fn load_refs(repo: Repo, exclude: Vec<String>) -> Action {
    let result = (|| {
        let head = repo.head()?;
        let refs = repo.refs()?;
        let checkouts = repo.checkouts(500)?;
        let branches = tt_git_core::branches::unify(&refs, &head, &checkouts, |r| repo.ref_activity(r));
        let base = match load_base(&repo, &refs, &branches, &exclude) {
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
        })
    })();
    Action::RefsLoaded(result.map_err(message))
}

/// Splits the branches by whether they're merged into the base branch.
fn load_base(
    repo: &Repo,
    refs: &[GitRef],
    branches: &[BranchEntry],
    exclude: &[String],
) -> Result<Option<BaseBranches>, GitError> {
    use tt_git_core::branches::{base_scope, compare_with_base, default_base, resolve_base};

    let setting = repo.base_branch_setting();
    let Some(base) = resolve_base(refs, setting.as_deref()) else {
        return Ok(None);
    };
    let detected = default_base(refs).map(|r| r.short_name.clone());
    let scope = base_scope(base);
    let merged = repo.merged_refs(&base.full_name, &scope)?;
    let (unmerged, merged) = compare_with_base(branches, base, &merged, exclude);
    Ok(Some(BaseBranches {
        base: base.short_name.clone(),
        base_ref: base.full_name.clone(),
        chosen: setting.is_some_and(|s| s == base.short_name || s == base.full_name),
        detected,
        unmerged,
        merged,
        authors: repo.ref_authors(&scope)?,
    }))
}

pub fn load_status(repo: Repo) -> Action {
    Action::StatusLoaded(repo.status().map_err(message))
}

/// Streams the graph's commits in batches, ending with a `done` batch; or,
/// with a `focus`, loads just that branch's history in one go.
pub fn load_graph(
    repo: Repo,
    scope: GraphScope,
    focus: Option<BranchFocus>,
    generation: u64,
    limit: usize,
    ctx: &Ctx,
) -> Action {
    if let Some(focus) = focus {
        return match repo.branch_history(&focus.tip, &focus.base_ref, limit) {
            Ok(history) => Action::BranchHistoryLoaded { generation, history },
            Err(e) => Action::GraphFailed {
                generation,
                error: message(e),
            },
        };
    }
    let scope = match scope {
        GraphScope::All => LogScope::All,
        GraphScope::Current => LogScope::Head {
            upstream: head_upstream(&repo),
        },
    };
    let mut batch = Vec::with_capacity(GRAPH_BATCH);
    let result = repo.stream_commits(&scope, limit, |commit| {
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

pub fn load_commit_details(repo: Repo, oid: String) -> Action {
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
        Ok(out) => cap_lines(&out.stdout_str()),
        Err(e) => format!("could not load commit: {e}"),
    };
    Action::DetailsLoaded {
        subject: DetailsSubject::Commit(oid),
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
        Ok(out) => out.stdout_str(),
        Err(e) => format!("could not load commit: {e}"),
    };
    let files = repo.commit_files(&oid).map_err(|e| e.to_string());
    Action::CommitFilesLoaded { oid, header, files }
}

pub fn load_commit_file_diff(repo: Repo, oid: String, file: CommitFile) -> Action {
    let text = match repo.commit_file_diff(&oid, &file) {
        Ok(t) if t.trim().is_empty() => format!("{}: no textual changes", file.path),
        Ok(t) => cap_lines(&t),
        Err(e) => format!("could not diff {}: {e}", file.path),
    };
    Action::CommitFileDiffLoaded {
        oid,
        path: file.path,
        text,
    }
}

fn cap_lines(text: &str) -> String {
    let mut lines = text.lines();
    let mut out: Vec<&str> = lines.by_ref().take(MAX_DETAIL_LINES).collect();
    let rest = lines.count();
    let note;
    if rest > 0 {
        note = format!("… {rest} more lines not shown");
        out.push(&note);
    }
    out.join("\n")
}

pub fn run_op(repo: Repo, op: GitOp) -> Action {
    let result = match &op {
        GitOp::Checkout(plan) => repo.checkout(plan),
        GitOp::CreateBranch { name, start } => repo.create_branch(name, start),
        GitOp::DeleteBranch { name, force } => repo.delete_branch(name, *force),
        GitOp::DeleteRemoteBranch { remote, branch } => repo.delete_remote_branch(remote, branch),
        GitOp::RenameBranch { old, new } => repo.rename_branch(old, new),
        GitOp::FetchAll => repo.fetch_all(),
        GitOp::SetBaseBranch(base) => repo.set_base_branch(base.as_deref()),
        GitOp::Stage(paths) => repo.stage(paths),
        GitOp::Unstage(paths) => repo.unstage(paths),
        GitOp::StageAll => repo.stage_all(),
        GitOp::UnstageAll => repo.unstage_all(),
        GitOp::Discard { path, kind } => repo.discard(path, kind),
        GitOp::Commit { message, amend } => repo.commit(message.as_deref(), *amend),
        GitOp::Push => repo.push(),
        GitOp::Pull => repo.pull(),
        GitOp::Merge { target } => repo.merge(target),
        GitOp::Rebase { onto } => repo.rebase(onto),
        GitOp::CherryPick { oid, .. } => repo.cherry_pick(oid),
        GitOp::Continue(op) => repo.continue_operation(*op),
        GitOp::Abort(op) => repo.abort_operation(*op),
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

/// Largest untracked file shown in full in the details pane.
const MAX_UNTRACKED_PREVIEW: usize = 256 * 1024;

pub fn load_file_diff(repo: Repo, entry: tt_git_core::StatusEntry) -> Action {
    let text = match repo.file_diff(&entry, MAX_UNTRACKED_PREVIEW) {
        Ok(t) if t.trim().is_empty() => format!("{}: no textual changes", entry.path),
        Ok(t) => cap_lines(&t),
        Err(e) => format!("could not diff {}: {e}", entry.path),
    };
    Action::DetailsLoaded {
        subject: DetailsSubject::File(entry.path),
        text,
    }
}
