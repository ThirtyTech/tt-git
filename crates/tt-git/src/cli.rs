//! Non-interactive subcommands. Together with the socket these are the
//! plugin API: plugins call `$TTGIT_BIN_PATH <subcommand>`.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, WrapErr, eyre};
use serde_json::{Value, json};
use tt_git_core::branches::unify;
use tt_git_core::{GitCli, Repo};
use tt_git_plugin::install::{self, GithubSpec};
use tt_git_plugin::registry::{self, PluginDirs, Registry};
use tt_git_plugin::socket;

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Manage plugins.
    #[command(subcommand)]
    Plugin(PluginCmd),
    /// Send a raw request to the running tt-git (`METHOD [PARAMS_JSON]`).
    Api {
        method: String,
        params: Option<String>,
        #[command(flatten)]
        socket: SocketArg,
    },
    /// Control the running tt-git UI.
    #[command(subcommand)]
    Ui(UiCmd),
    /// Stream events from the running tt-git, one JSON object per line.
    Events {
        /// Only these events (repeatable).
        #[arg(long = "event")]
        events: Vec<String>,
        #[command(flatten)]
        socket: SocketArg,
    },
    /// Print working tree status.
    Status {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        repo: RepoArg,
    },
    /// Branch commands.
    #[command(subcommand)]
    Branch(BranchCmd),
    /// Internal: a scriptable fake plugin used by the test suite.
    #[command(name = "__test-plugin", hide = true)]
    TestPlugin {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        steps: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct SocketArg {
    /// Socket of the tt-git instance (default: $TTGIT_SOCKET_PATH).
    #[arg(long, env = "TTGIT_SOCKET_PATH", hide_env_values = true)]
    socket: Option<String>,
}

impl SocketArg {
    fn address(&self) -> Result<&str> {
        self.socket.as_deref().ok_or_else(|| {
            eyre!("no running tt-git: set TTGIT_SOCKET_PATH or pass --socket (plugins get it automatically)")
        })
    }
}

#[derive(Debug, Args)]
pub struct RepoArg {
    /// Repository (default: $TTGIT_REPO_PATH, then the current directory).
    #[arg(long, env = "TTGIT_REPO_PATH", hide_env_values = true)]
    repo: Option<PathBuf>,
}

impl RepoArg {
    fn open(&self) -> Result<Repo> {
        let path = match &self.repo {
            Some(p) => p.clone(),
            None => std::env::current_dir()?,
        };
        Repo::discover(&path, &GitCli::new(&path)).wrap_err("opening repository")
    }
}

#[derive(Debug, Subcommand)]
pub enum PluginCmd {
    /// Install from GitHub: `owner/repo[/subdir…]`. Shows what will run and
    /// asks first (unless --yes).
    Install {
        source: String,
        /// Branch, tag or commit (default: the repository's HEAD).
        #[arg(long = "ref")]
        git_ref: Option<String>,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Update GitHub-installed plugins (all when none are named).
    Update {
        targets: Vec<String>,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Remove an installed plugin (by id or owner/repo).
    Uninstall {
        target: String,
    },
    /// Plugin panes in the running tt-git.
    #[command(subcommand)]
    Pane(PaneCmd),
    /// Register a local plugin directory (for development; no build step).
    Link {
        path: PathBuf,
        #[arg(long)]
        disabled: bool,
    },
    /// Remove a plugin from the registry (files are kept).
    Unlink {
        plugin_id: String,
    },
    /// List installed plugins.
    List {
        #[arg(long)]
        json: bool,
        #[arg(long = "plugin")]
        plugin_id: Option<String>,
    },
    Enable {
        plugin_id: String,
    },
    Disable {
        plugin_id: String,
    },
    /// Print a plugin's config directory.
    ConfigDir {
        plugin_id: String,
    },
    /// Plugin actions (needs a running tt-git).
    #[command(subcommand)]
    Action(ActionCmd),
    /// Plugin command logs (needs a running tt-git).
    #[command(subcommand)]
    Log(LogCmd),
}

#[derive(Debug, Subcommand)]
pub enum PaneCmd {
    /// Open a `[[panes]]` entry point.
    Open {
        #[arg(long = "plugin")]
        plugin_id: String,
        #[arg(long)]
        entrypoint: String,
        /// overlay, popup, split, tab or zoomed (default: the manifest's).
        #[arg(long)]
        placement: Option<String>,
        /// Popup width: cells or "N%".
        #[arg(long)]
        width: Option<String>,
        /// Popup height: cells or "N%".
        #[arg(long)]
        height: Option<String>,
        /// Extra environment for the pane (repeatable).
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        #[arg(long, overrides_with = "no_focus")]
        focus: bool,
        #[arg(long)]
        no_focus: bool,
        #[command(flatten)]
        socket: SocketArg,
    },
    Focus {
        pane_id: u64,
        #[command(flatten)]
        socket: SocketArg,
    },
    Close {
        pane_id: u64,
        #[command(flatten)]
        socket: SocketArg,
    },
}

#[derive(Debug, Subcommand)]
pub enum ActionCmd {
    List {
        #[arg(long = "plugin")]
        plugin_id: Option<String>,
        #[command(flatten)]
        socket: SocketArg,
    },
    Invoke {
        action_id: String,
        #[arg(long = "plugin")]
        plugin_id: Option<String>,
        #[command(flatten)]
        socket: SocketArg,
    },
}

#[derive(Debug, Subcommand)]
pub enum LogCmd {
    List {
        #[arg(long = "plugin")]
        plugin_id: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[command(flatten)]
        socket: SocketArg,
    },
}

#[derive(Debug, Subcommand)]
pub enum UiCmd {
    /// Show a message in the help bar.
    Notify {
        message: String,
        #[command(flatten)]
        socket: SocketArg,
    },
    /// Show an error popup.
    Error {
        message: String,
        #[command(flatten)]
        socket: SocketArg,
    },
    /// Reload everything.
    Refresh {
        #[command(flatten)]
        socket: SocketArg,
    },
    /// Focus a pane: status, branches, graph or details.
    Focus {
        pane: String,
        #[command(flatten)]
        socket: SocketArg,
    },
}

#[derive(Debug, Subcommand)]
pub enum BranchCmd {
    /// The unified, recency-sorted branch list.
    List {
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        repo: RepoArg,
    },
}

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn api(socket: &SocketArg, method: &str, params: Value) -> Result<Value> {
    Ok(socket::call(socket.address()?, method, params)?)
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Plugin(p) => plugin(p),
        Cmd::Api { method, params, socket } => {
            let params = match params {
                Some(p) => serde_json::from_str(&p).wrap_err("PARAMS_JSON is not valid JSON")?,
                None => Value::Null,
            };
            print_json(&api(&socket, &method, params)?);
            Ok(())
        }
        Cmd::Ui(ui) => {
            match ui {
                UiCmd::Notify { message, socket } => api(&socket, "ui.notify", json!({ "message": message }))?,
                UiCmd::Error { message, socket } => api(&socket, "ui.error", json!({ "message": message }))?,
                UiCmd::Refresh { socket } => api(&socket, "ui.refresh", Value::Null)?,
                UiCmd::Focus { pane, socket } => api(&socket, "ui.focus", json!({ "pane": pane }))?,
            };
            Ok(())
        }
        Cmd::Events { events, socket } => {
            let params = if events.is_empty() {
                Value::Null
            } else {
                json!({ "events": events })
            };
            for ev in socket::subscribe(socket.address()?, params)? {
                println!("{}", serde_json::to_string(&ev).unwrap_or_default());
            }
            Ok(())
        }
        Cmd::Status { json, repo } => {
            let status = repo.open()?.status()?;
            if json {
                print_json(&serde_json::to_value(&status)?);
            } else {
                for e in &status.entries {
                    println!("{} {}", e.short_code(), e.path);
                }
            }
            Ok(())
        }
        Cmd::Branch(BranchCmd::List { json, repo }) => {
            let repo = repo.open()?;
            let (refs, head, checkouts) = (repo.refs()?, repo.head()?, repo.checkouts(500)?);
            let list = unify(&refs, &head, &checkouts, |r| repo.ref_activity(r));
            if json {
                let v: Vec<Value> = list
                    .iter()
                    .map(|e| {
                        let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
                        v["state"] = serde_json::to_value(e.state()).unwrap_or(Value::Null);
                        v
                    })
                    .collect();
                print_json(&Value::Array(v));
            } else {
                for e in &list {
                    println!("{} {}", if e.is_head { "*" } else { " " }, e.name);
                }
            }
            Ok(())
        }
        Cmd::TestPlugin { steps } => crate::test_plugin::run(&steps),
    }
}

/// Asks before running someone else's code, unless `--yes`.
fn confirm(question: &str, yes: bool) -> Result<bool> {
    use std::io::{BufRead, IsTerminal, Write};
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(eyre!("not a terminal: pass --yes to confirm non-interactively"));
    }
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// Previews, confirms, builds and registers a fetched plugin.
fn finish_install(dirs: &PluginDirs, fetched: install::Fetched, yes: bool, verb: &str, done: &str) -> Result<bool> {
    if let Err(e) = install::check_not_linked(dirs, &fetched) {
        install::discard(fetched);
        return Err(e.into());
    }
    print!("{}", fetched.preview());
    if !confirm(&format!("{verb} {}?", fetched.manifest.manifest.id), yes)? {
        install::discard(fetched);
        println!("cancelled");
        return Ok(false);
    }
    if let Err(e) = install::build(&fetched) {
        install::discard(fetched);
        return Err(e.into());
    }
    let (id, version, commit) = (
        fetched.manifest.manifest.id.clone(),
        fetched.manifest.manifest.version.clone(),
        fetched.resolved_commit.chars().take(7).collect::<String>(),
    );
    install::commit(dirs, fetched)?;
    println!("{done} {id} {version} ({commit})");
    Ok(true)
}

fn plugin(cmd: PluginCmd) -> Result<()> {
    let dirs = PluginDirs::from_env();
    let version = tt_git_plugin::host_version();
    match cmd {
        PluginCmd::Install { source, git_ref, yes } => {
            let spec = GithubSpec::parse(&source)?;
            let fetched = install::fetch(&dirs, &spec, git_ref.as_deref())?;
            finish_install(&dirs, fetched, yes, "Install", "installed")?;
        }
        PluginCmd::Update { targets, yes } => {
            let installs = install::github_installs(&dirs, &targets)?;
            if installs.is_empty() {
                println!("no GitHub-installed plugins to update");
            }
            for p in installs {
                let (spec, git_ref, old) = install::update_source(&p).expect("github installs only");
                let fetched = install::fetch(&dirs, &spec, git_ref.as_deref())?;
                if fetched.resolved_commit == old {
                    install::discard(fetched);
                    println!("{} is up to date", p.id);
                    continue;
                }
                finish_install(&dirs, fetched, yes, "Update", "updated")?;
            }
        }
        PluginCmd::Uninstall { target } => {
            let removed = install::uninstall(&dirs, &target)?;
            println!("uninstalled {}", removed.id);
        }
        PluginCmd::Pane(pane) => match pane {
            PaneCmd::Open {
                plugin_id,
                entrypoint,
                placement,
                width,
                height,
                env,
                focus: _,
                no_focus,
                socket,
            } => {
                let dim = |d: Option<String>| -> Value {
                    match d {
                        Some(s) => s.parse::<u16>().map(Value::from).unwrap_or(Value::String(s)),
                        None => Value::Null,
                    }
                };
                let env: serde_json::Map<String, Value> = env
                    .iter()
                    .filter_map(|kv| kv.split_once('='))
                    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
                    .collect();
                let v = api(
                    &socket,
                    "plugin.pane.open",
                    json!({
                        "plugin_id": plugin_id,
                        "entrypoint": entrypoint,
                        "placement": placement,
                        "width": dim(width),
                        "height": dim(height),
                        "env": env,
                        "focus": !no_focus,
                    }),
                )?;
                print_json(&v);
            }
            PaneCmd::Focus { pane_id, socket } => {
                api(&socket, "plugin.pane.focus", json!({ "pane_id": pane_id }))?;
            }
            PaneCmd::Close { pane_id, socket } => {
                api(&socket, "plugin.pane.close", json!({ "pane_id": pane_id }))?;
            }
        },
        PluginCmd::Link { path, disabled } => {
            let rec = registry::link(&dirs, &path, !disabled, &version)?;
            println!(
                "linked {} ({})",
                rec.installed.id,
                if disabled { "disabled" } else { "enabled" }
            );
            for w in rec.warnings {
                eprintln!("warning: {w}");
            }
        }
        PluginCmd::Unlink { plugin_id } => {
            registry::unlink(&dirs, &plugin_id)?;
            println!("unlinked {plugin_id}");
        }
        PluginCmd::Enable { plugin_id } => {
            registry::set_enabled(&dirs, &plugin_id, true)?;
            println!("enabled {plugin_id}");
        }
        PluginCmd::Disable { plugin_id } => {
            registry::set_enabled(&dirs, &plugin_id, false)?;
            println!("disabled {plugin_id}");
        }
        PluginCmd::ConfigDir { plugin_id } => {
            let reg = Registry::read(&dirs)?;
            if reg.get(&plugin_id).is_none() {
                return Err(registry::RegistryError::NotFound(plugin_id).into());
            }
            println!("{}", dirs.plugin_config_dir(&plugin_id).display());
        }
        PluginCmd::List { json, plugin_id } => {
            let records = Registry::read(&dirs)?.records(&version);
            let records: Vec<_> = records
                .into_iter()
                .filter(|r| plugin_id.as_deref().is_none_or(|p| r.installed.id == p))
                .collect();
            if json {
                let v: Vec<Value> = records
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
                print_json(&Value::Array(v));
            } else if records.is_empty() {
                println!("no plugins installed");
            } else {
                for r in &records {
                    let state = if !r.installed.enabled {
                        "disabled"
                    } else if r.is_runnable() {
                        "enabled"
                    } else {
                        "unavailable"
                    };
                    let version = r.manifest.as_ref().map(|m| m.manifest.version.as_str()).unwrap_or("?");
                    println!(
                        "{} {version} ({state}) {}",
                        r.installed.id,
                        display(&r.installed.manifest_path)
                    );
                    for w in &r.warnings {
                        println!("    warning: {w}");
                    }
                }
            }
        }
        PluginCmd::Action(ActionCmd::List { plugin_id, socket }) => {
            print_json(&api(&socket, "plugin.action.list", json!({ "plugin_id": plugin_id }))?);
        }
        PluginCmd::Action(ActionCmd::Invoke {
            action_id,
            plugin_id,
            socket,
        }) => {
            let v = api(
                &socket,
                "plugin.action.invoke",
                json!({ "action_id": action_id, "plugin_id": plugin_id }),
            )?;
            print_json(&v);
        }
        PluginCmd::Log(LogCmd::List {
            plugin_id,
            limit,
            socket,
        }) => {
            print_json(&api(
                &socket,
                "plugin.log.list",
                json!({ "plugin_id": plugin_id, "limit": limit }),
            )?);
        }
    }
    Ok(())
}

fn display(p: &Path) -> String {
    p.display().to_string()
}
