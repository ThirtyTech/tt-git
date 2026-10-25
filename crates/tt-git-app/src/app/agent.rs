//! Agent actions: the dialog `A` opens on a commit or a diff hunk, and starting the agent
//! in herdr or in place of the details pane.

use super::App;
use crate::action::Action;
use crate::agent::{self, AgentContext, AgentRequest, AgentStart, Herdr};
use crate::modal::{Modal, PromptAction};
use crate::panes::PaneId;
use crate::pty_pane::{self, PtyPane};

/// The `entrypoint` of the agent's pane.
const AGENT_PANE: &str = "agent";

impl App {
    /// Opens the agent dialog about the graph's selected commit, or from
    /// details about the diff hunk or lines selected there. While a merge
    /// (or rebase…) is stopped, the conflict action is picked first.
    pub(super) fn open_agent_dialog(&mut self) {
        if self.focus_running_agent() {
            return;
        }
        let merging = self.status.status().is_some_and(|s| s.in_progress.is_some());
        if self.focus == PaneId::Details
            && let Some(found) = self.details.agent_hunk()
        {
            let mut hunk = match found {
                Ok(hunk) => hunk,
                Err(why) => {
                    self.notice = Some(why);
                    return;
                }
            };
            if let (Some(oid), Some(c)) = (&hunk.commit, self.graph.selected())
                && c.oid == *oid
            {
                hunk.subject = Some(c.summary.to_string());
            }
            let title = format!("Ask an agent about {} {}", hunk.file, hunk.lines);
            let actions = agent::offered(&self.config.agent, merging, true);
            self.modal = Some(Modal::prompt(
                title,
                "",
                PromptAction::Agent {
                    actions,
                    selected: None,
                    hunk: Some(hunk),
                },
            ));
            return;
        }
        let actions = agent::offered(&self.config.agent, merging, false);
        let selected = (merging && actions.first().is_some_and(|a| a.when == agent::When::Merging)).then_some(0);
        let title = match self.graph.selected() {
            Some(c) => format!("Ask an agent about {}", c.short_oid()),
            None => "Ask an agent".to_string(),
        };
        self.modal = Some(Modal::prompt(
            title,
            "",
            PromptAction::Agent {
                actions,
                selected,
                hunk: None,
            },
        ));
    }

    /// An agent already running in the details pane gets the focus
    /// instead of starting another.
    fn focus_running_agent(&mut self) -> bool {
        let running = self
            .panes
            .iter()
            .any(|p| p.plugin_id == pty_pane::BUILTIN && p.entrypoint == AGENT_PANE && p.exited.is_none());
        if running {
            self.notice = Some("an agent is already running".into());
            self.details_open = true;
            self.focus = PaneId::Details;
        }
        running
    }

    /// What the prompt's placeholders are filled with, from what's on screen.
    fn agent_context(&self) -> AgentContext {
        let status = self.status.status();
        let head = self.head.as_ref();
        let commit = self.graph.selected();
        AgentContext {
            repo: self.repo.root().display().to_string(),
            commit: commit.map(|c| c.oid.to_string()).unwrap_or_default(),
            subject: commit.map(|c| c.summary.to_string()).unwrap_or_default(),
            branch: head
                .and_then(|h| h.branch())
                .map(str::to_string)
                .or_else(|| head.and_then(|h| h.oid()).map(str::to_string))
                .unwrap_or_default(),
            base: self.branches.base().map(|b| b.base.clone()).unwrap_or_default(),
            upstream: status.and_then(|s| s.branch.upstream.clone()).unwrap_or_default(),
            state: status
                .and_then(|s| s.in_progress)
                .map(|op| op.command().to_string())
                .unwrap_or_default(),
            conflicts: self
                .status
                .entries()
                .iter()
                .filter(|e| e.is_conflicted())
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            ..Default::default()
        }
    }

    /// Builds the prompt and starts the agent on a background job.
    pub(super) fn start_agent(&mut self, req: AgentRequest) {
        let repo = self.repo.clone();
        let cfg = self.config.agent.clone();
        let ctx = self.agent_context();
        let herdr = Herdr::from_env(|k| std::env::var(k).ok());
        self.notice = Some(format!("starting agent: {}…", req.label()));
        self.ctx
            .spawn(move || Action::AgentStarted(agent::start(&repo, &cfg, &req, ctx, herdr)));
    }

    pub(super) fn agent_started(&mut self, result: Result<AgentStart, String>) {
        match result {
            Ok(AgentStart::Herdr(where_)) => self.notice = Some(where_),
            Ok(AgentStart::Pane { argv, title, note }) => {
                if let Err(e) = self.open_agent_pane(&argv, &title) {
                    self.notice = None;
                    self.error = Some(format!("{e}. Set [agent] command in config.toml."));
                    return;
                }
                self.notice = note.or(Some(format!("{title} started")));
            }
            Err(e) => {
                self.notice = None;
                self.error = Some(e);
            }
        }
    }

    /// Runs the agent in a pane in place of details, and focuses it.
    fn open_agent_pane(&mut self, argv: &[String], title: &str) -> Result<(), String> {
        // GUI: sizes the PTY from the details rect as terminal cells (minus
        // the border). The GUI reports pixels; convert with font metrics.
        let (_, details) = self.layout_rects;
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let pane = PtyPane::spawn_command(
            id,
            AGENT_PANE,
            title,
            argv,
            self.repo.root(),
            details.height.saturating_sub(2).max(5),
            details.width.saturating_sub(2).max(20),
            self.ctx.clone(),
        )?;
        // One split at a time: it takes the details pane's place.
        self.panes
            .retain(|p| p.placement != tt_git_plugin::manifest::Placement::Split);
        self.panes.push(pane);
        self.details_open = true;
        self.focus = PaneId::Details;
        Ok(())
    }

    /// Whether pane `id` is the agent's: when it ends, the repo may have changed.
    pub(super) fn is_agent_pane(&self, id: pty_pane::PaneId) -> bool {
        self.panes
            .iter()
            .any(|p| p.id == id && p.plugin_id == pty_pane::BUILTIN && p.entrypoint == AGENT_PANE)
    }
}
