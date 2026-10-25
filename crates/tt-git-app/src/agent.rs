//! Agent actions: ask an AI coding agent (`claude` by default) about the
//! selected commit, or a diff hunk or lines. The agent opens in a new herdr tab when tt-git runs
//! inside herdr, otherwise in place of the details pane.

use std::path::{Path, PathBuf};
use std::process::Command as Process;

use serde::Deserialize;
use serde_json::Value;
use tt_git_core::patch::{Hunk, LineKind};
use tt_git_core::repo::Repo;

use crate::editor::split_command;

/// The `[agent]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// The agent command. `{prompt}` or `{prompt_file}` in it make it a
    /// template; without them the prompt is the last argument.
    pub command: String,
    pub placement: AgentPlacement,
    /// Goes ahead of every prompt. Placeholders work here too.
    pub system_prompt: Option<String>,
    /// Goes after the system prompt when asking about a diff hunk.
    pub hunk_prompt: Option<String>,
    /// Replaces the built-in actions when set.
    pub actions: Option<Vec<AgentAction>>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            command: "claude".into(),
            placement: AgentPlacement::Auto,
            system_prompt: None,
            hunk_prompt: None,
            actions: None,
        }
    }
}

/// Where the agent runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentPlacement {
    /// A herdr tab inside herdr, the details pane otherwise.
    #[default]
    Auto,
    HerdrTab,
    /// A herdr pane split off beside tt-git's.
    HerdrSplit,
    Details,
}

/// A ready-made request, offered with `tab` in the agent dialog.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAction {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub when: When,
}

/// When an action is offered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    #[default]
    Always,
    /// Only while a merge, rebase, cherry-pick or revert is stopped.
    Merging,
    /// Only when asking about a diff hunk or lines, and then only these.
    Hunk,
}

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You were started from tt-git, a terminal git client, in the git repository at {repo}. \
The current branch is {branch}. Run git yourself to read history and diffs. \
Don't push, and don't commit unless you're asked to.";

pub const DEFAULT_HUNK_PROMPT: &str = "\
This is about {lines} of {file}, in {source}:

```diff
{hunk}
```";

fn action(name: &str, description: &str, when: When, prompt: &str) -> AgentAction {
    AgentAction {
        name: name.into(),
        description: Some(description.into()),
        prompt: prompt.into(),
        when,
    }
}

/// The actions offered when `[[agent.actions]]` isn't set.
pub fn default_actions() -> Vec<AgentAction> {
    vec![
        action(
            "Resolve conflicts",
            "resolve the conflicts using both sides' history",
            When::Merging,
            "A {state} of {merge_head} into {branch} stopped with conflicts in:\n{conflicts}\n\n\
             Read the git log of both sides to understand why each change was made, then resolve \
             every conflict so both intents are kept. Stage each resolved file with git add. \
             Don't commit and don't continue the {state}; tell me what you decided for each file.",
        ),
        action(
            "Explain commit",
            "what the selected commit changes and why",
            When::Always,
            "Explain commit {commit} (\"{subject}\"): what it changes, why, and anything a reviewer \
             should look at closely. Use git show {commit}.",
        ),
        action(
            "Explain branch",
            "how the feature on this branch works",
            When::Always,
            "Explain what {branch} adds compared to {base} and how the new feature works, from the \
             entry points down. Its commits:\n{log}",
        ),
        action(
            "Review commit",
            "look for bugs in the selected commit",
            When::Always,
            "Review commit {commit} (\"{subject}\") for bugs, missed cases and risky changes. \
             List only real problems, most serious first, with file and line.",
        ),
        action(
            "Explain change",
            "what these lines change and why",
            When::Hunk,
            "Explain what this change does and why it was made. Read the code around it in {file}, \
             and its history with git log or git blame, as you need.",
        ),
        action(
            "Review change",
            "look for bugs in these lines",
            When::Hunk,
            "Review this change for bugs, missed cases and risky edits. List only real problems, \
             most serious first, with file and line.",
        ),
    ]
}

/// The actions to offer: the configured ones (or the defaults), with the
/// merge-only ones first while merging and left out otherwise. About a
/// hunk, only the hunk ones.
pub fn offered(cfg: &AgentConfig, merging: bool, hunk: bool) -> Vec<AgentAction> {
    let all = cfg.actions.clone().unwrap_or_else(default_actions);
    let (hunks, all): (Vec<_>, Vec<_>) = all.into_iter().partition(|a| a.when == When::Hunk);
    if hunk {
        return hunks;
    }
    let (merge, other): (Vec<_>, Vec<_>) = all.into_iter().partition(|a| a.when == When::Merging);
    if merging {
        merge.into_iter().chain(other).collect()
    } else {
        other
    }
}

/// What the placeholders in a prompt are filled with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentContext {
    pub repo: String,
    pub commit: String,
    pub subject: String,
    pub branch: String,
    pub base: String,
    pub upstream: String,
    /// `merge`, `rebase`… while one is stopped.
    pub state: String,
    /// Conflicted paths, one per line.
    pub conflicts: String,
    /// What's being merged, rebased or picked.
    pub merge_head: String,
    /// `git log --oneline base..branch`, capped.
    pub log: String,
    /// About a hunk: its file, lines, diff and what it's a diff of.
    pub file: String,
    pub lines: String,
    pub hunk: String,
    pub source: String,
}

/// Commits listed for `{log}` at most.
const LOG_LIMIT: usize = 50;

impl AgentContext {
    /// Fills in what needs git: the branch's log and what's being merged.
    pub fn load(&mut self, repo: &Repo) {
        if !self.base.is_empty() && !self.branch.is_empty() {
            let range = format!("{}..{}", self.base, self.branch);
            let limit = format!("-{LOG_LIMIT}");
            if let Ok(out) = repo.git().run(&["log", "--oneline", "--no-decorate", &limit, &range]) {
                self.log = out.stdout_str().trim_end().to_string();
            }
        }
        if !self.state.is_empty() && self.merge_head.is_empty() {
            for head in ["MERGE_HEAD", "REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
                if let Ok(out) = repo.git().run(&["name-rev", "--name-only", "--always", head]) {
                    let name = out.stdout_str().trim().to_string();
                    if !name.is_empty() {
                        self.merge_head = name;
                        break;
                    }
                }
            }
        }
    }

    fn value(&self, key: &str) -> Option<&str> {
        Some(match key {
            "repo" => &self.repo,
            "commit" => &self.commit,
            "subject" => &self.subject,
            "branch" => &self.branch,
            "base" => &self.base,
            "upstream" => &self.upstream,
            "state" => &self.state,
            "conflicts" => &self.conflicts,
            "merge_head" => &self.merge_head,
            "log" => &self.log,
            "file" => &self.file,
            "lines" => &self.lines,
            "hunk" => &self.hunk,
            "source" => &self.source,
            _ => return None,
        })
    }

    /// Points the placeholders at `hunk`, and at its commit if it has one.
    pub fn set_hunk(&mut self, hunk: &AgentHunk) {
        self.file = hunk.file.clone();
        self.lines = hunk.lines.clone();
        self.hunk = hunk.diff.clone();
        self.source = hunk.source.clone();
        if let Some(commit) = &hunk.commit {
            self.commit = commit.clone();
            self.subject = hunk.subject.clone().unwrap_or_default();
        }
    }

    /// `template` with each known `{name}` replaced. Unknown ones stay as
    /// they are, so braces in a prompt don't need escaping.
    pub fn fill(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after
                .find('}')
                .and_then(|close| self.value(&after[..close]).map(|v| (close, v)))
            {
                Some((close, value)) => {
                    out.push_str(value);
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// What the dialog asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRequest {
    /// The ready-made action picked, or none for a custom message.
    pub action: Option<AgentAction>,
    /// What was typed: the request, or extra instructions for the action.
    pub message: String,
    /// The hunk or lines asked about, rather than the graph's commit.
    pub hunk: Option<AgentHunk>,
}

/// A diff hunk, or a range of its lines, to ask an agent about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHunk {
    pub file: String,
    /// `line 12`, `lines 12-30`, or `old lines 5-7` when only deleted
    /// lines are picked.
    pub lines: String,
    /// The picked lines, each hunk's under its `@@` heading.
    pub diff: String,
    /// What the diff is of: `the unstaged changes…`, `commit <hash>`.
    pub source: String,
    /// The commit (or stash) the diff is from, and its subject.
    pub commit: Option<String>,
    pub subject: Option<String>,
}

impl AgentHunk {
    /// Lines `first..=last` of each hunk in `picks`, of `file`, from `source`.
    pub fn new(file: &str, source: String, picks: &[(&Hunk, usize, usize)]) -> Self {
        let mut new_lines = Vec::new();
        let mut old_lines = Vec::new();
        let mut diff = Vec::new();
        for &(hunk, first, last) in picks {
            diff.push(hunk.heading());
            // `git diff` numbers an empty range by the line before it.
            let mut old = if hunk.old_count == 0 {
                hunk.old_start + 1
            } else {
                hunk.old_start
            };
            for (i, line) in hunk.lines.iter().enumerate().take(last + 1) {
                if i >= first {
                    let prefix = match line.kind {
                        LineKind::Context => ' ',
                        LineKind::Add => '+',
                        LineKind::Del => '-',
                    };
                    diff.push(format!("{prefix}{}", line.text.trim_end_matches('\r')));
                    match line.kind {
                        LineKind::Del => old_lines.push(old),
                        _ => new_lines.push(hunk.new_line_of(i)),
                    }
                }
                if line.kind != LineKind::Add {
                    old += 1;
                }
            }
        }
        let span = |nums: &[u32]| match (nums.iter().min(), nums.iter().max()) {
            (Some(a), Some(b)) if a == b => format!("line {a}"),
            (Some(a), Some(b)) => format!("lines {a}-{b}"),
            _ => String::new(),
        };
        let lines = if new_lines.is_empty() {
            format!("old {}", span(&old_lines))
        } else {
            span(&new_lines)
        };
        Self {
            file: file.to_string(),
            lines,
            diff: diff.join("\n"),
            source,
            commit: None,
            subject: None,
        }
    }

    /// Says it's from `commit`.
    pub fn in_commit(mut self, commit: &str, subject: Option<String>) -> Self {
        self.commit = Some(commit.to_string());
        self.subject = subject;
        self
    }
}

impl AgentRequest {
    /// A short name for the tab or pane title.
    pub fn label(&self) -> String {
        match &self.action {
            Some(a) => a.name.clone(),
            None => "ask".into(),
        }
    }
}

/// The whole prompt: the system prompt, the hunk prompt when it's about a
/// hunk, then the action's prompt or the message, with the placeholders
/// filled.
pub fn build_prompt(cfg: &AgentConfig, req: &AgentRequest, ctx: &AgentContext) -> String {
    let system = cfg.system_prompt.as_deref().unwrap_or(DEFAULT_SYSTEM_PROMPT);
    let hunk = match &req.hunk {
        Some(_) => cfg.hunk_prompt.as_deref().unwrap_or(DEFAULT_HUNK_PROMPT),
        None => "",
    };
    let message = req.message.trim();
    let body = match &req.action {
        Some(a) if message.is_empty() => a.prompt.clone(),
        Some(a) => format!("{}\n\n{message}", a.prompt.trim_end()),
        None => message.to_string(),
    };
    let parts = [ctx.fill(system.trim()), ctx.fill(hunk.trim()), ctx.fill(body.trim())];
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The agent's command line. `{prompt}` and `{prompt_file}` are replaced
/// wherever they appear; `prompt_in_args` says whether one did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub argv: Vec<String>,
    pub prompt_in_args: bool,
}

pub fn agent_command(command: &str, prompt: &str, prompt_file: &Path) -> Result<AgentCommand, String> {
    let mut argv = split_command(command, cfg!(windows));
    if argv.is_empty() {
        return Err("[agent] command is empty".into());
    }
    let file = prompt_file.display().to_string();
    let mut prompt_in_args = false;
    for w in &mut argv[1..] {
        if w.contains("{prompt}") || w.contains("{prompt_file}") {
            prompt_in_args = true;
            *w = w.replace("{prompt_file}", &file).replace("{prompt}", prompt);
        }
    }
    Ok(AgentCommand { argv, prompt_in_args })
}

/// Where the agent ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    HerdrTab,
    HerdrSplit,
    Details,
}

pub fn target(placement: AgentPlacement, in_herdr: bool) -> Target {
    match placement {
        AgentPlacement::Auto if in_herdr => Target::HerdrTab,
        AgentPlacement::Auto | AgentPlacement::Details => Target::Details,
        AgentPlacement::HerdrTab => Target::HerdrTab,
        AgentPlacement::HerdrSplit => Target::HerdrSplit,
    }
}

/// How a launch went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStart {
    /// Running in herdr; the text says where.
    Herdr(String),
    /// Start `argv` in a pane in place of details. `note` says why it
    /// isn't in herdr when it was meant to be.
    Pane {
        argv: Vec<String>,
        title: String,
        note: Option<String>,
    },
}

/// Agent kinds herdr can start and track itself (`herdr agent start --kind`).
const HERDR_KINDS: &[&str] = &[
    "pi",
    "claude",
    "codex",
    "gemini",
    "cursor",
    "devin",
    "agy",
    "cline",
    "omp",
    "mastracode",
    "opencode",
    "copilot",
    "kimi",
    "kiro",
    "droid",
    "amp",
    "grok",
    "hermes",
    "kilo",
    "qodercli",
    "qwen",
    "letta",
    "maki",
    "muse",
];

/// The herdr kind for `program`, if herdr knows it.
pub fn herdr_kind(program: &str) -> Option<&'static str> {
    let name = Path::new(program).file_name()?.to_str()?;
    let name = name.strip_suffix(".exe").unwrap_or(name);
    HERDR_KINDS.iter().copied().find(|k| k.eq_ignore_ascii_case(name))
}

/// The herdr session tt-git runs in, from the variables herdr sets in its panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Herdr {
    pub bin: String,
    pub workspace: Option<String>,
    pub pane: Option<String>,
}

impl Herdr {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        env("HERDR_ENV").filter(|v| !v.is_empty() && v != "0")?;
        // herdr's own path can go stale after it updates itself ("… (deleted)").
        let bin = env("HERDR_BIN_PATH")
            .filter(|p| Path::new(p).is_file())
            .unwrap_or_else(|| "herdr".into());
        Some(Self {
            bin,
            workspace: env("HERDR_WORKSPACE_ID").filter(|v| !v.is_empty()),
            pane: env("HERDR_PANE_ID").filter(|v| !v.is_empty()),
        })
    }

    /// Runs `herdr <args>` and returns its JSON `result`.
    fn call(&self, args: &[&str]) -> Result<Value, String> {
        let out = Process::new(&self.bin)
            .args(args)
            .output()
            .map_err(|e| format!("couldn't run {}: {e}", self.bin))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let json: Option<Value> = serde_json::from_str(text.trim()).ok();
        if !out.status.success() {
            let why = json
                .as_ref()
                .and_then(|j| j.pointer("/error/message"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).trim().to_string());
            return Err(format!("herdr {}: {why}", args.first().copied().unwrap_or_default()));
        }
        Ok(json.and_then(|j| j.get("result").cloned()).unwrap_or(Value::Null))
    }

    /// Opens a tab (or a split beside tt-git) in `cwd` and returns its pane.
    fn open(&self, target: Target, cwd: &Path, label: &str) -> Result<String, String> {
        let cwd = cwd.display().to_string();
        let (result, key) = if target == Target::HerdrSplit {
            let mut args = vec!["pane", "split", "--direction", "right", "--cwd", &cwd];
            if let Some(pane) = &self.pane {
                args.extend(["--pane", pane]);
            }
            (self.call(&args)?, "pane")
        } else {
            let mut args = vec!["tab", "create", "--cwd", &cwd, "--label", label, "--focus"];
            if let Some(ws) = &self.workspace {
                args.extend(["--workspace", ws]);
            }
            (self.call(&args)?, "root_pane")
        };
        pane_id(result.get(key)).ok_or_else(|| format!("herdr didn't say which pane it opened: {result}"))
    }

    /// Starts the agent in a new herdr tab or split and hands it the prompt.
    pub fn launch(
        &self,
        target: Target,
        cwd: &Path,
        label: &str,
        cmd: &AgentCommand,
        prompt: &str,
    ) -> Result<String, String> {
        let pane = self.open(target, cwd, label)?;
        let kind = (!cmd.prompt_in_args).then(|| herdr_kind(&cmd.argv[0])).flatten();
        match kind {
            Some(kind) => {
                let name = agent_name(label);
                let mut args = vec!["agent", "start", &name, "--kind", kind, "--pane", &pane];
                if cmd.argv.len() > 1 {
                    args.push("--");
                    args.extend(cmd.argv[1..].iter().map(String::as_str));
                }
                self.call(&args)?;
                self.call(&["agent", "prompt", &name, prompt])?;
            }
            None => {
                let mut argv = cmd.argv.clone();
                if !cmd.prompt_in_args {
                    argv.push(prompt.to_string());
                }
                self.call(&["pane", "run", &pane, &shell_join(&argv, cfg!(windows))])?;
            }
        }
        Ok(match target {
            Target::HerdrSplit => format!("agent started in herdr pane {pane}"),
            _ => format!("agent started in herdr tab \"{label}\""),
        })
    }
}

/// A pane id from herdr: a string, or an object with `pane_id`.
fn pane_id(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("pane_id").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// A herdr agent name (`[a-z][a-z0-9_-]{0,31}`), unique to this run.
fn agent_name(label: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let slug: String = label
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    let mut name = format!("tt{}-{n}-{}", std::process::id() % 100_000, slug.join("-"));
    name.truncate(32);
    name.trim_end_matches('-').to_string()
}

/// `argv` as one shell command line: POSIX single quotes, or PowerShell's
/// on Windows (where `'` doubles instead).
pub fn shell_join(argv: &[String], windows: bool) -> String {
    let quote = |a: &String| {
        let plain = !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
        if plain {
            a.clone()
        } else if windows {
            format!("'{}'", a.replace('\'', "''"))
        } else {
            format!("'{}'", a.replace('\'', r"'\''"))
        }
    };
    let words: Vec<String> = argv.iter().map(quote).collect();
    // PowerShell runs a quoted program name only after `&`.
    if windows && words.first().is_some_and(|w| w.starts_with('\'')) {
        format!("& {}", words.join(" "))
    } else {
        words.join(" ")
    }
}

/// Where the prompt is written for `{prompt_file}`: a new file each time,
/// so an agent still reading one isn't handed the next.
pub fn prompt_file(repo_name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join("tt-git")
        .join(repo_name)
        .join(format!("agent-prompt-{}-{n}.md", std::process::id()))
}

/// Builds the prompt and starts the agent: in herdr when that's where it
/// goes (falling back to details if herdr fails), else returns the
/// command for a pane. Runs on a background job.
pub fn start(
    repo: &Repo,
    cfg: &AgentConfig,
    req: &AgentRequest,
    mut ctx: AgentContext,
    herdr: Option<Herdr>,
) -> Result<AgentStart, String> {
    if let Some(hunk) = &req.hunk {
        ctx.set_hunk(hunk);
    }
    ctx.load(repo);
    let prompt = build_prompt(cfg, req, &ctx);
    let file = prompt_file(&repo.name());
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't write the prompt file: {e}"))?;
    }
    std::fs::write(&file, &prompt).map_err(|e| format!("couldn't write the prompt file: {e}"))?;
    let cmd = agent_command(&cfg.command, &prompt, &file)?;
    let label = format!("agent: {}", req.label());
    let mut note = None;
    if let (Target::HerdrTab | Target::HerdrSplit, Some(h)) = (target(cfg.placement, herdr.is_some()), &herdr) {
        match h.launch(target(cfg.placement, true), repo.root(), &label, &cmd, &prompt) {
            Ok(where_) => return Ok(AgentStart::Herdr(where_)),
            Err(e) => note = Some(format!("{e}; opened here instead")),
        }
    } else if target(cfg.placement, false) != Target::Details {
        note = Some("not running inside herdr; opened here instead".into());
    }
    let mut argv = cmd.argv;
    if !cmd.prompt_in_args {
        argv.push(prompt);
    }
    Ok(AgentStart::Pane {
        argv,
        title: label,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> AgentContext {
        AgentContext {
            repo: "/r".into(),
            commit: "abc1234".into(),
            subject: "Add x".into(),
            branch: "topic".into(),
            base: "origin/main".into(),
            ..Default::default()
        }
    }

    #[test]
    fn fills_known_placeholders_only() {
        let c = ctx();
        assert_eq!(c.fill("{branch} vs {base}"), "topic vs origin/main");
        assert_eq!(c.fill("keep {this} and {"), "keep {this} and {");
        assert_eq!(c.fill("{{commit}}"), "{abc1234}");
        assert_eq!(c.fill("fn x() { {subject} }"), "fn x() { Add x }");
    }

    #[test]
    fn prompt_is_system_then_action_then_message() {
        let cfg = AgentConfig {
            system_prompt: Some("In {repo}.".into()),
            ..Default::default()
        };
        let mut req = AgentRequest {
            action: Some(action("Explain", "", When::Always, "Explain {commit}.")),
            message: String::new(),
            hunk: None,
        };
        assert_eq!(build_prompt(&cfg, &req, &ctx()), "In /r.\n\nExplain abc1234.");
        req.message = " keep it short ".into();
        assert_eq!(
            build_prompt(&cfg, &req, &ctx()),
            "In /r.\n\nExplain abc1234.\n\nkeep it short"
        );
        req.action = None;
        assert_eq!(build_prompt(&cfg, &req, &ctx()), "In /r.\n\nkeep it short");
        let no_system = AgentConfig {
            system_prompt: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(build_prompt(&no_system, &req, &ctx()), "keep it short");
    }

    #[test]
    fn merge_actions_come_first_and_only_while_merging() {
        let cfg = AgentConfig::default();
        let names = |merging, hunk| {
            offered(&cfg, merging, hunk)
                .into_iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(false, false),
            ["Explain commit", "Explain branch", "Review commit"]
        );
        assert_eq!(names(true, false)[0], "Resolve conflicts");
        assert_eq!(names(true, false).len(), 4);
        assert_eq!(names(false, true), ["Explain change", "Review change"]);
        assert_eq!(names(true, true), ["Explain change", "Review change"]);
    }

    fn hunk() -> Hunk {
        let diff = "@@ -10,4 +10,4 @@ fn f\n a\n-b\n+B\n+C\n c\n-d\n";
        tt_git_core::patch::FileDiff::parse(diff).unwrap().hunks.remove(0)
    }

    #[test]
    fn a_hunk_names_its_lines() {
        let h = hunk();
        let whole = AgentHunk::new("f.rs", "commit x".into(), &[(&h, 0, h.lines.len() - 1)]);
        assert_eq!(whole.lines, "lines 10-13");
        assert_eq!(whole.diff, "@@ -10,4 +10,4 @@ fn f\n a\n-b\n+B\n+C\n c\n-d");
        let added = AgentHunk::new("f.rs", String::new(), &[(&h, 2, 3)]);
        assert_eq!(added.lines, "lines 11-12");
        assert_eq!(added.diff, "@@ -10,4 +10,4 @@ fn f\n+B\n+C");
        assert_eq!(AgentHunk::new("f.rs", String::new(), &[(&h, 2, 2)]).lines, "line 11");
        // Only deletions: their lines on the old side.
        assert_eq!(
            AgentHunk::new("f.rs", String::new(), &[(&h, 1, 1)]).lines,
            "old line 11"
        );
        assert_eq!(
            AgentHunk::new("f.rs", String::new(), &[(&h, 5, 5)]).lines,
            "old line 13"
        );
    }

    #[test]
    fn a_hunk_request_carries_the_hunk() {
        let cfg = AgentConfig {
            system_prompt: Some(String::new()),
            ..Default::default()
        };
        let h = hunk();
        let found = AgentHunk::new("f.rs", "commit abc".into(), &[(&h, 2, 3)]).in_commit("abc", Some("S".into()));
        let req = AgentRequest {
            action: None,
            message: "why?".into(),
            hunk: Some(found.clone()),
        };
        let mut c = ctx();
        c.set_hunk(&found);
        assert_eq!(c.fill("{commit} {subject}"), "abc S");
        assert_eq!(
            build_prompt(&cfg, &req, &c),
            "This is about lines 11-12 of f.rs, in commit abc:\n\n```diff\n@@ -10,4 +10,4 @@ fn f\n+B\n+C\n```\n\nwhy?"
        );
        let custom = AgentConfig {
            hunk_prompt: Some("{file}:{lines}".into()),
            ..cfg
        };
        assert_eq!(build_prompt(&custom, &req, &c), "f.rs:lines 11-12\n\nwhy?");
    }

    #[test]
    fn parses_config() {
        let cfg: AgentConfig = toml::from_str(
            r#"
            command = "codex --model o4"
            placement = "herdr-split"
            [[actions]]
            name = "Fix"
            prompt = "fix {commit}"
            when = "merging"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.placement, AgentPlacement::HerdrSplit);
        let actions = cfg.actions.unwrap();
        assert_eq!(actions[0].when, When::Merging);
        assert!(toml::from_str::<AgentConfig>("placement = \"tab\"").is_err());
    }

    #[test]
    fn prompt_goes_where_the_template_says() {
        let file = Path::new("/tmp/p.md");
        let plain = agent_command("claude --model opus", "hi there", file).unwrap();
        assert_eq!(plain.argv, ["claude", "--model", "opus"]);
        assert!(!plain.prompt_in_args);
        let t = agent_command("aider --message-file {prompt_file}", "hi", file).unwrap();
        assert_eq!(t.argv, ["aider", "--message-file", "/tmp/p.md"]);
        assert!(t.prompt_in_args);
        let t = agent_command("x -p {prompt}", "a b", file).unwrap();
        assert_eq!(t.argv, ["x", "-p", "a b"]);
        assert!(agent_command("  ", "x", file).is_err());
    }

    #[test]
    fn picks_where_it_runs() {
        assert_eq!(target(AgentPlacement::Auto, true), Target::HerdrTab);
        assert_eq!(target(AgentPlacement::Auto, false), Target::Details);
        assert_eq!(target(AgentPlacement::Details, true), Target::Details);
        assert_eq!(target(AgentPlacement::HerdrSplit, false), Target::HerdrSplit);
    }

    #[test]
    fn knows_herdr_kinds() {
        assert_eq!(herdr_kind("claude"), Some("claude"));
        assert_eq!(herdr_kind("/usr/local/bin/codex"), Some("codex"));
        assert_eq!(
            herdr_kind(r"C:\bin\Claude.exe"),
            if cfg!(windows) { Some("claude") } else { None }
        );
        assert_eq!(herdr_kind("aider"), None);
    }

    #[test]
    fn reads_herdr_from_env() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(Herdr::from_env(env(&[])), None);
        assert_eq!(Herdr::from_env(env(&[("HERDR_ENV", "0")])), None);
        let h = Herdr::from_env(env(&[
            ("HERDR_ENV", "1"),
            ("HERDR_BIN_PATH", "/gone/herdr (deleted)"),
            ("HERDR_WORKSPACE_ID", "w1"),
        ]))
        .unwrap();
        assert_eq!(h.bin, "herdr");
        assert_eq!(h.workspace.as_deref(), Some("w1"));
        assert_eq!(h.pane, None);
    }

    #[test]
    fn pane_ids_and_names() {
        assert_eq!(pane_id(Some(&Value::from("w1:p3"))), Some("w1:p3".into()));
        assert_eq!(
            pane_id(Some(&serde_json::json!({"pane_id": "w1:p4"}))),
            Some("w1:p4".into())
        );
        assert_eq!(pane_id(None), None);
        let name = agent_name("agent: Resolve conflicts!");
        assert!(name.len() <= 32 && name.starts_with("tt"), "{name}");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert_ne!(agent_name("x"), agent_name("x"));
    }

    #[test]
    fn joins_for_the_shell() {
        let argv = ["aider".to_string(), "it's a \"test\"".to_string(), "a-b".to_string()];
        assert_eq!(shell_join(&argv, false), r#"aider 'it'\''s a "test"' a-b"#);
        assert_eq!(shell_join(&argv, true), r#"aider 'it''s a "test"' a-b"#);
        let spaced = ["C:/Program Files/x.exe".to_string()];
        assert_eq!(shell_join(&spaced, true), "& 'C:/Program Files/x.exe'");
    }
}
