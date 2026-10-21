//! Plugin host integration: the API socket, events, actions and the palette.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use serde_json::{Value, json};
use tt_git_core::CheckoutPlan;
use tt_git_plugin::host::PluginHost;
use tt_git_plugin::manifest::ActionContext;
use tt_git_plugin::registry::{self, PluginDirs};
use tt_git_plugin::runtime::{CommandLog, CommandStatus, EventEnvelope, HostEnv, InvocationContext, Runner};
use tt_git_plugin::socket::{Call, EventLine, Reply, Response, Server};

use super::App;
use crate::action::Action;
use crate::keymap::{Command, Context};
use crate::modal::{GitOp, PaletteItem, PaletteTarget};
use crate::panes::{DetailsSubject, PaneId};

/// How to set up the plugin host.
#[derive(Debug, Clone)]
pub struct PluginOptions {
    pub dirs: PluginDirs,
    /// `TTGIT_BIN_PATH` for plugins (normally the running executable).
    pub bin_path: Option<PathBuf>,
    /// Listen on the API socket.
    pub socket: bool,
    /// Extra environment for every plugin command.
    pub extra_env: Vec<(OsString, OsString)>,
}

struct Subscriber {
    tx: Sender<EventLine>,
    /// Only these event names, or everything when `None`.
    events: Option<Vec<String>>,
}

pub(super) struct PluginState {
    pub(super) host: PluginHost,
    lease: Option<tt_git_plugin::install::Lease>,
    server: Option<Server>,
    socket_wanted: bool,
    subscribers: Vec<Subscriber>,
    correlation: u64,
    last_head: Option<(Option<String>, Option<String>)>,
    started: bool,
}

/// Built-in commands offered in the palette (besides the focused pane's).
const PALETTE_BUILTINS: &[Command] = &[
    Command::Continue,
    Command::Abort,
    Command::Undo,
    Command::Fetch,
    Command::Pull,
    Command::Push,
    Command::Refresh,
    Command::ToggleScope,
    Command::FocusGraph,
    Command::FocusStatus,
    Command::FocusBranches,
    Command::FocusDetails,
    Command::Zoom,
    Command::PluginLogs,
    Command::Help,
    Command::Quit,
];

fn pane_from_name(s: &str) -> Option<PaneId> {
    Some(match s {
        "status" | "files" => PaneId::Status,
        "branches" => PaneId::Branches,
        "graph" => PaneId::Graph,
        "details" | "diff" => PaneId::Details,
        _ => return None,
    })
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn ok(v: Value) -> Result<Value, (&'static str, String)> {
    Ok(v)
}

fn to_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

impl App {
    /// Loads installed plugins. Call before [`start`](App::start), which
    /// opens the socket and runs startup commands.
    pub fn enable_plugins(&mut self, opts: PluginOptions) {
        let ctx = self.ctx.clone();
        let runner = Runner::new(
            HostEnv {
                socket_path: None,
                bin_path: opts.bin_path,
                extra: opts.extra_env,
            },
            move |log: &Arc<CommandLog>| ctx.emit(Action::PluginFinished(log.clone())),
        );
        let host = PluginHost::new(opts.dirs.clone(), tt_git_plugin::host_version(), runner);
        // Claim our plugins' checkouts, then clear out unclaimed old ones
        // (off the main thread: it can delete whole checkouts).
        let lease = tt_git_plugin::install::Lease::acquire(&opts.dirs, &host.registered_paths())
            .map_err(|e| tracing::warn!("plugin lease: {e}"))
            .ok();
        let dirs = opts.dirs.clone();
        let gc = std::thread::Builder::new()
            .name("plugin-gc".into())
            .stack_size(crate::IO_THREAD_STACK)
            .spawn(move || {
                tt_git_plugin::install::gc(&dirs);
            });
        if let Err(e) = gc {
            tracing::warn!("plugin gc: {e}");
        }
        self.plugins = Some(PluginState {
            host,
            lease,
            server: None,
            socket_wanted: opts.socket,
            subscribers: Vec::new(),
            correlation: 0,
            last_head: None,
            started: false,
        });
    }

    pub fn plugin_host(&self) -> Option<&PluginHost> {
        self.plugins.as_ref().map(|p| &p.host)
    }

    /// The API socket address (`TTGIT_SOCKET_PATH`), once listening.
    pub fn socket_address(&self) -> Option<&str> {
        self.plugins.as_ref()?.server.as_ref().map(Server::address)
    }

    /// No background jobs and no plugin commands running.
    pub fn is_idle(&self) -> bool {
        self.ctx.jobs().in_flight() == 0 && self.plugins.as_ref().is_none_or(|p| p.host.runner().in_flight() == 0)
    }

    pub(super) fn start_plugins(&mut self) {
        let ctx = self.ctx.clone();
        let Some(state) = self.plugins.as_mut() else {
            return;
        };
        if state.started {
            return;
        }
        state.started = true;
        if state.socket_wanted {
            let address = tt_git_plugin::socket::new_address();
            match Server::start(&address, move |call| ctx.emit(Action::Socket(call))) {
                Ok(server) => {
                    let mut env = state.host.runner().host();
                    env.socket_path = Some(server.address().to_string());
                    state.host.runner().set_host(env);
                    state.server = Some(server);
                }
                Err(e) => tracing::warn!("plugin socket disabled: {e}"),
            }
        }
        let context = self.invocation_context("startup");
        if let Some(state) = &self.plugins {
            state.host.run_startup(&context);
        }
        let root = self.repo.root().display().to_string();
        self.emit_event("app.started", json!({ "version": env!("CARGO_PKG_VERSION") }));
        self.emit_event("repo.opened", json!({ "path": root, "name": self.repo.name() }));
    }

    /// Re-reads plugins when the registry changed (called on ticks).
    /// Returns whether it did.
    pub(super) fn poll_plugins(&mut self) -> bool {
        let Some(state) = &mut self.plugins else {
            return false;
        };
        if !state.host.reload_if_changed() {
            return false;
        }
        if let Some(lease) = &state.lease {
            let _ = lease.set_paths(&state.host.registered_paths());
        }
        true
    }

    pub(super) fn invocation_context(&mut self, source: &str) -> InvocationContext {
        let correlation = self.plugins.as_mut().map(|p| {
            p.correlation += 1;
            format!("tt-{}-{}", std::process::id(), p.correlation)
        });
        let status = self.status.status();
        let head = self.head.as_ref();
        InvocationContext {
            repo_path: Some(self.repo.root().display().to_string()),
            repo_name: Some(self.repo.name()),
            head: head.and_then(|h| h.oid()).map(str::to_string),
            branch: head.and_then(|h| h.branch()).map(str::to_string),
            upstream: status.and_then(|s| s.branch.upstream.clone()),
            selected_commit: self.graph.selected().map(|c| c.oid.to_string()),
            selected_branch: self.branches.selected().map(|b| b.name.clone()),
            selected_file: self.status.selected().map(|e| e.path.clone()),
            selected_text: None,
            focused_pane: Some(self.focus.name().into()),
            invocation_source: Some(source.into()),
            correlation_id: correlation,
            clicked_url: None,
            link_handler_id: None,
        }
    }

    /// Runs a plugin action, reporting failures in the UI.
    pub(super) fn invoke_plugin_action(&mut self, action_id: &str, source: &str) -> Result<u64, String> {
        let context = self.invocation_context(source);
        let Some(state) = &self.plugins else {
            return Err("plugins are not enabled".into());
        };
        match state.host.invoke_action(action_id, context) {
            Ok(id) => {
                self.notice = Some(format!("▶ {action_id}"));
                Ok(id)
            }
            Err(e) => {
                let msg = e.to_string();
                self.error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// Delivers an event to hooks and socket subscribers.
    pub(super) fn emit_event(&mut self, event: &str, data: Value) {
        if self.plugins.is_none() {
            return;
        }
        let envelope = EventEnvelope {
            event: event.to_string(),
            data,
        };
        let context = self.invocation_context("event");
        let Some(state) = &mut self.plugins else {
            return;
        };
        for result in state.host.fire_event(&envelope, &context) {
            if let Err(e) = result {
                tracing::warn!(event, "plugin hook not run: {e}");
            }
        }
        if state.subscribers.is_empty() {
            return;
        }
        let line = match EventLine::new(&to_json(&envelope)) {
            Ok(line) => line,
            Err(e) => {
                tracing::warn!(event, "event not sent to subscribers: {e}");
                return;
            }
        };
        state.subscribers.retain(|s| {
            let wanted = s.events.as_ref().is_none_or(|names| names.iter().any(|n| n == event));
            !wanted || s.tx.send(line.clone()).is_ok()
        });
    }

    /// Emits the plugin event for a finished write operation.
    pub(super) fn op_event(&mut self, op: &GitOp, head: Option<&str>) {
        let branch = self.head.as_ref().and_then(|h| h.branch()).map(str::to_string);
        if let GitOp::Checkout { stash: true, .. } = op {
            self.emit_event("stash.created", json!({ "message": null }));
        }
        if let GitOp::Undo { step, stash: true } = op
            && step.target.is_switch()
        {
            self.emit_event("stash.created", json!({ "message": null }));
        }
        let (event, data) = match op {
            GitOp::Checkout {
                plan: CheckoutPlan::Local(name),
                ..
            } => ("branch.checked_out", json!({ "branch": name, "created": false })),
            GitOp::Checkout {
                plan: CheckoutPlan::Track { name, remote_ref },
                ..
            } => (
                "branch.checked_out",
                json!({ "branch": name, "created": true, "tracking": remote_ref }),
            ),
            // No branch to report; `head.changed` covers it.
            GitOp::Checkout {
                plan: CheckoutPlan::Detach(_),
                ..
            } => return,
            GitOp::CreateBranch { name, start, worktree } => {
                let path = worktree.then(|| self.repo.worktree_path(name));
                (
                    "branch.created",
                    json!({ "branch": name, "start": start, "worktree": path }),
                )
            }
            GitOp::DeleteBranch { name, .. } => ("branch.deleted", json!({ "branch": name, "remote": null })),
            GitOp::DeleteRemoteBranch { remote, branch } => {
                ("branch.deleted", json!({ "branch": branch, "remote": remote }))
            }
            GitOp::RenameBranch { old, new } => ("branch.renamed", json!({ "from": old, "to": new })),
            GitOp::CreateTag { name, target, message } => (
                "tag.created",
                json!({ "tag": name, "target": target, "annotated": message.is_some() }),
            ),
            GitOp::PushTag { name } => ("tag.pushed", json!({ "tag": name })),
            GitOp::DeleteTag { name, remote } => ("tag.deleted", json!({ "tag": name, "remote": remote })),
            GitOp::FetchAll => ("fetch.completed", json!({})),
            GitOp::Pull => ("pull.completed", json!({ "branch": branch, "head": head })),
            GitOp::Push => ("push.completed", json!({ "branch": branch })),
            GitOp::Stage(paths) => ("files.staged", json!({ "paths": paths })),
            GitOp::StageAll => ("files.staged", json!({ "paths": "all" })),
            GitOp::Unstage(paths) => ("files.unstaged", json!({ "paths": paths })),
            GitOp::UnstageAll => ("files.unstaged", json!({ "paths": "all" })),
            GitOp::Discard { path, .. } => ("files.discarded", json!({ "path": path })),
            GitOp::ApplyPatch { path, op, .. } => match op {
                tt_git_core::PatchOp::Stage => ("files.staged", json!({ "paths": [path], "partial": true })),
                tt_git_core::PatchOp::Unstage => ("files.unstaged", json!({ "paths": [path], "partial": true })),
                tt_git_core::PatchOp::Discard => ("files.discarded", json!({ "path": path, "partial": true })),
            },
            GitOp::Merge { target } => ("merge.completed", json!({ "target": target, "head": head })),
            GitOp::Rebase { onto } => ("rebase.completed", json!({ "onto": onto, "head": head })),
            GitOp::CherryPick { oid, summary } => (
                "cherry_pick.completed",
                json!({ "picked": oid, "summary": summary, "head": head }),
            ),
            GitOp::Continue(op) => (
                match op {
                    tt_git_core::InProgress::Merge => "merge.completed",
                    tt_git_core::InProgress::Rebase => "rebase.completed",
                    tt_git_core::InProgress::CherryPick => "cherry_pick.completed",
                    tt_git_core::InProgress::Revert => "commit.created",
                },
                json!({ "head": head, "continued": true }),
            ),
            GitOp::StashPush { message, .. } => ("stash.created", json!({ "message": message })),
            GitOp::StashApply { index, pop } => (
                "stash.applied",
                json!({ "stash": format!("stash@{{{index}}}"), "pop": pop }),
            ),
            GitOp::StashDrop { index } => ("stash.dropped", json!({ "stash": format!("stash@{{{index}}}") })),
            GitOp::StashBranch { name, index } => (
                "branch.created",
                json!({ "branch": name, "start": format!("stash@{{{index}}}"), "worktree": null }),
            ),
            // HEAD moving is reported as `head.changed`.
            GitOp::Abort(_) | GitOp::SetBaseBranch(_) | GitOp::Undo { .. } => return,
            GitOp::Commit { message, amend } => (
                "commit.created",
                json!({
                    "oid": head,
                    "amend": amend,
                    "summary": message.as_deref().and_then(|m| m.lines().next()),
                }),
            ),
        };
        self.emit_event(event, data);
        if let GitOp::CreateBranch {
            name, worktree: false, ..
        } = op
        {
            self.emit_event("branch.checked_out", json!({ "branch": name, "created": true }));
        }
    }

    /// Fires `head.changed` when a refs reload shows HEAD moved.
    pub(super) fn track_head(&mut self) {
        let Some(head) = self.head.clone() else {
            return;
        };
        // Switching between branches at the same commit is a HEAD change too.
        let now = (head.branch().map(str::to_string), head.oid().map(str::to_string));
        let Some(state) = &mut self.plugins else {
            return;
        };
        let before = state.last_head.replace(now.clone());
        if let Some(before) = before
            && before != now
        {
            self.emit_event(
                "head.changed",
                json!({ "from": before.1, "from_branch": before.0, "to": now.1, "branch": now.0 }),
            );
        }
    }

    pub(super) fn plugin_finished(&mut self, log: Arc<CommandLog>) {
        if log.status == CommandStatus::Failed {
            let what = log.action_id.as_deref().or(log.event.as_deref()).unwrap_or("command");
            let why = match (&log.error, log.exit_code) {
                (Some(e), _) => e.clone(),
                (None, Some(code)) => format!("exit {code}"),
                (None, None) => "killed".into(),
            };
            self.notice = Some(format!("✗ plugin {} {what}: {why}", log.plugin_id));
        }
    }

    /// Items for the command palette: the focused pane's commands, then
    /// plugin actions offered in the focused context, then common built-ins.
    pub(super) fn palette_items(&self) -> Vec<PaletteItem> {
        let context = self.focused().context();
        let builtin = |cmd: Command| {
            let hint = self
                .keymap
                .display_key(context, cmd)
                .map(|k| k.to_string())
                .unwrap_or_default();
            let mut label = cmd.label().to_string();
            if let Some(first) = label.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            PaletteItem {
                label,
                hint,
                target: PaletteTarget::Command(cmd),
            }
        };
        let pane_cmds = self.focused().commands();
        let mut items: Vec<PaletteItem> = pane_cmds.iter().map(|c| builtin(*c)).collect();
        items.extend(self.plugin_palette_items(context));
        items.extend(
            PALETTE_BUILTINS
                .iter()
                .filter(|c| !pane_cmds.contains(c))
                .map(|c| builtin(*c)),
        );
        items
    }

    fn plugin_palette_items(&self, context: Context) -> Vec<PaletteItem> {
        let Some(state) = &self.plugins else {
            return Vec::new();
        };
        let wanted = match self.focus {
            PaneId::Graph => Some(ActionContext::Commit),
            PaneId::Branches => Some(ActionContext::Branch),
            PaneId::Status => Some(ActionContext::File),
            PaneId::Details => None,
        };
        let panes: Vec<PaletteItem> = state
            .host
            .records()
            .iter()
            .filter(|r| r.is_runnable())
            .flat_map(|r| {
                let m = &r.manifest.as_ref().expect("runnable").manifest;
                m.panes.iter().map(move |p| PaletteItem {
                    label: format!("Open {}", p.title),
                    hint: m.name.clone(),
                    target: PaletteTarget::PluginPane(m.id.clone(), p.id.clone()),
                })
            })
            .collect();
        let mut items: Vec<PaletteItem> = state
            .host
            .actions()
            .into_iter()
            .filter(|a| {
                a.contexts
                    .iter()
                    .any(|c| *c == ActionContext::Global || Some(*c) == wanted)
            })
            .map(|a| {
                let key = self
                    .keymap
                    .custom()
                    .iter()
                    .find(|k| k.action == a.action_id && (k.context == context || k.context == Context::Global))
                    .map(|k| format!("{}  ", k.key))
                    .unwrap_or_default();
                PaletteItem {
                    label: a.title.clone(),
                    hint: format!("{key}{}", a.plugin_name),
                    target: PaletteTarget::PluginAction(a.action_id.clone()),
                }
            })
            .collect();
        items.extend(panes);
        items
    }

    /// Runs the first plugin link handler matching a URL in view in the
    /// details pane (the first URL on screen, else anywhere in the text).
    pub(super) fn open_link(&mut self) {
        let url = find_url(&self.details.visible_lines()).or_else(|| find_url(self.details.lines()));
        let Some(url) = url else {
            self.notice = Some("no link in view".into());
            return;
        };
        self.run_link_handler(url);
    }

    fn run_link_handler(&mut self, url: String) {
        let handler = self
            .plugins
            .as_ref()
            .and_then(|s| s.host.link_handlers_for(&url).into_iter().next());
        let Some((_plugin, handler_id, action)) = handler else {
            self.notice = Some(format!("no plugin handles {url}"));
            return;
        };
        let mut context = self.invocation_context("link_click");
        context.clicked_url = Some(url);
        context.link_handler_id = Some(handler_id);
        let state = self.plugins.as_ref().expect("checked above");
        match state.host.invoke_action(&action, context) {
            Ok(_) => self.notice = Some(format!("▶ {action}")),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Ctrl+click in the details pane: run the link handler for the URL on
    /// that line.
    pub(super) fn ctrl_click_details(&mut self, row: usize) {
        let line = self.details.visible_lines().get(row).cloned();
        if let Some(url) = line.and_then(|l| find_url(std::slice::from_ref(&l))) {
            self.run_link_handler(url);
        }
    }

    pub(super) fn show_plugin_logs(&mut self) {
        let text = match &self.plugins {
            None => "plugins are not enabled".to_string(),
            Some(state) => {
                let logs = state.host.runner().logs(None, 50);
                let mut out = String::new();
                for r in state.host.records() {
                    let status = if r.is_runnable() {
                        "enabled"
                    } else if r.installed.enabled {
                        "unavailable"
                    } else {
                        "disabled"
                    };
                    out.push_str(&format!("plugin {} ({status})\n", r.installed.id));
                    for w in &r.warnings {
                        out.push_str(&format!("    warning: {w}\n"));
                    }
                }
                if state.host.records().is_empty() {
                    out.push_str("no plugins installed (tt-git plugin link <dir>)\n");
                }
                out.push('\n');
                for l in logs {
                    let what = l.action_id.as_deref().or(l.event.as_deref()).unwrap_or("-");
                    out.push_str(&format!(
                        "#{} {} {what} {:?} exit={}\n",
                        l.log_id,
                        l.plugin_id,
                        l.status,
                        l.exit_code.map_or("-".into(), |c| c.to_string())
                    ));
                    for line in l.stdout.lines().take(20) {
                        out.push_str(&format!("    {line}\n"));
                    }
                    for line in l.stderr.lines().take(20) {
                        out.push_str(&format!("  ! {line}\n"));
                    }
                    if let Some(e) = &l.error {
                        out.push_str(&format!("  ! {e}\n"));
                    }
                }
                out
            }
        };
        let subject = DetailsSubject::Text("plugin-logs".into());
        self.details.request(subject.clone(), "Plugin logs".into());
        self.details.set_content(&subject, &text);
        self.focus = PaneId::Details;
    }

    /// Answers one API socket request.
    pub(super) fn handle_call(&mut self, call: Call) {
        let Call { request, replier } = call;
        let id = request.id.clone();
        let params = request.params.clone();

        if request.method == "events.subscribe" {
            let events = params.get("events").and_then(Value::as_array).map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
            let (tx, rx) = std::sync::mpsc::channel();
            if let Some(state) = &mut self.plugins {
                state.subscribers.push(Subscriber { tx, events });
            }
            replier.send(Reply::Stream {
                first: Response::ok(id, json!({ "subscribed": true })),
                events: rx,
            });
            return;
        }

        let result = self.api(&request.method, &params);
        replier.respond(match result {
            Ok(v) => Response::ok(id, v),
            Err((code, msg)) => Response::err(id, code, msg),
        });
    }

    fn api(&mut self, method: &str, params: &Value) -> Result<Value, (&'static str, String)> {
        match method {
            "ping" => ok(json!({ "pong": true, "version": env!("CARGO_PKG_VERSION") })),
            "repo.info" => {
                let status = self.status.status();
                let head = self.head.as_ref();
                ok(json!({
                    "path": self.repo.root(),
                    "name": self.repo.name(),
                    "head": head.and_then(|h| h.oid()),
                    "branch": head.and_then(|h| h.branch()),
                    "upstream": status.and_then(|s| s.branch.upstream.clone()),
                    "ahead": status.map(|s| s.branch.ahead),
                    "behind": status.map(|s| s.branch.behind),
                }))
            }
            "status.get" => ok(to_json(&self.status.status())),
            "branch.list" => {
                let list: Vec<Value> = self
                    .branches
                    .entries()
                    .iter()
                    .map(|e| {
                        let mut v = to_json(e);
                        v["state"] = to_json(&e.state());
                        v
                    })
                    .collect();
                ok(Value::Array(list))
            }
            "graph.selection" => ok(json!({
                "scope": self.graph.scope.label(),
                "branch": self.graph.focus.as_ref().map(|f| &f.branch),
                "index": self.graph.selected_index(),
                "commit": self.graph.selected(),
            })),
            "ui.notify" => {
                let msg = str_param(params, "message").ok_or(("invalid_params", "message is required".into()))?;
                self.notice = Some(msg.to_string());
                ok(json!({}))
            }
            "ui.error" => {
                let msg = str_param(params, "message").ok_or(("invalid_params", "message is required".into()))?;
                self.error = Some(msg.to_string());
                ok(json!({}))
            }
            "ui.refresh" => {
                self.update(Action::Refresh);
                ok(json!({}))
            }
            "ui.focus" => {
                let name = str_param(params, "pane").unwrap_or_default();
                let pane = pane_from_name(name).ok_or(("invalid_params", format!("unknown pane {name:?}")))?;
                self.focus = pane;
                self.refresh_details(false);
                ok(json!({}))
            }
            "plugin.list" => {
                let state = self
                    .plugins
                    .as_ref()
                    .ok_or(("plugins_disabled", "plugins are not enabled".into()))?;
                let list: Vec<Value> = state
                    .host
                    .records()
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.installed.id,
                            "name": r.manifest.as_ref().map(|m| m.manifest.name.clone()),
                            "version": r.manifest.as_ref().map(|m| m.manifest.version.clone()),
                            "enabled": r.installed.enabled,
                            "runnable": r.is_runnable(),
                            "source": r.installed.source,
                            "manifest_path": r.installed.manifest_path,
                            "warnings": r.warnings,
                        })
                    })
                    .collect();
                ok(Value::Array(list))
            }
            "plugin.link" | "plugin.unlink" | "plugin.enable" | "plugin.disable" => self.registry_api(method, params),
            "plugin.pane.open" | "plugin.pane.focus" | "plugin.pane.close" | "plugin.pane.screen"
            | "plugin.pane.list" | "popup.close" => self.pane_api(method, params),
            "plugin.action.list" => {
                let state = self
                    .plugins
                    .as_ref()
                    .ok_or(("plugins_disabled", "plugins are not enabled".into()))?;
                let filter = str_param(params, "plugin_id");
                let list: Vec<_> = state
                    .host
                    .actions()
                    .into_iter()
                    .filter(|a| filter.is_none_or(|f| a.plugin_id == f))
                    .collect();
                ok(to_json(&list))
            }
            "plugin.action.invoke" => {
                let mut action = str_param(params, "action_id")
                    .ok_or(("invalid_params", "action_id is required".into()))?
                    .to_string();
                if let Some(plugin) = str_param(params, "plugin_id")
                    && !action.starts_with(&format!("{plugin}."))
                {
                    action = format!("{plugin}.{action}");
                }
                let context = self.invocation_context("socket");
                let state = self
                    .plugins
                    .as_ref()
                    .ok_or(("plugins_disabled", "plugins are not enabled".into()))?;
                let log_id = state
                    .host
                    .invoke_action(&action, context)
                    .map_err(|e| ("plugin_action_failed", e.to_string()))?;
                ok(json!({ "log_id": log_id }))
            }
            "plugin.log.list" => {
                let state = self
                    .plugins
                    .as_ref()
                    .ok_or(("plugins_disabled", "plugins are not enabled".into()))?;
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
                let logs = state.host.runner().logs(str_param(params, "plugin_id"), limit);
                ok(to_json(&logs))
            }
            _ => Err(("unknown_method", format!("unknown method {method:?}"))),
        }
    }

    fn registry_api(&mut self, method: &str, params: &Value) -> Result<Value, (&'static str, String)> {
        let state = self
            .plugins
            .as_mut()
            .ok_or(("plugins_disabled", "plugins are not enabled".into()))?;
        let dirs = state.host.dirs().clone();
        let fail = |e: registry::RegistryError| ("plugin_registry_error", e.to_string());
        let id = || str_param(params, "plugin_id").ok_or(("invalid_params", "plugin_id is required".to_string()));
        let result = match method {
            "plugin.link" => {
                let path = str_param(params, "path").ok_or(("invalid_params", "path is required".into()))?;
                let enabled = params.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                let rec = registry::link(
                    &dirs,
                    std::path::Path::new(path),
                    enabled,
                    &tt_git_plugin::host_version(),
                )
                .map_err(fail)?;
                json!({ "id": rec.installed.id, "warnings": rec.warnings })
            }
            "plugin.unlink" => json!({ "id": registry::unlink(&dirs, id()?).map_err(fail)?.id }),
            "plugin.enable" => json!({ "id": registry::set_enabled(&dirs, id()?, true).map_err(fail)?.id }),
            _ => json!({ "id": registry::set_enabled(&dirs, id()?, false).map_err(fail)?.id }),
        };
        state.host.reload();
        Ok(result)
    }
}

/// The first `http(s)://` URL in `lines`, without trailing punctuation.
fn find_url(lines: &[String]) -> Option<String> {
    for line in lines {
        let Some(start) = line.find("https://").or_else(|| line.find("http://")) else {
            continue;
        };
        let rest = &line[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
            .unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', ',', ';', ':', ')', ']', '!', '?']);
        if url.len() > "https://".len() {
            return Some(url.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::find_url;

    fn s(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn finds_urls() {
        assert_eq!(
            find_url(&s(&["no link", "see https://github.com/a/b/issues/1."])),
            Some("https://github.com/a/b/issues/1".into())
        );
        assert_eq!(find_url(&s(&["(http://x.io/path)"])), Some("http://x.io/path".into()));
        assert_eq!(find_url(&s(&["<https://a.b/c>"])), Some("https://a.b/c".into()));
        assert_eq!(find_url(&s(&["https://", "nothing"])), None);
    }
}
