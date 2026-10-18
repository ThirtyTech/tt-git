//! The plugin host: installed plugins, their actions and event hooks.

use std::time::SystemTime;

use serde::Serialize;

use crate::manifest::{self, ActionContext, PaneSpec, platform_matches};
use crate::registry::{PluginDirs, PluginRecord, Registry};
use crate::runtime::{CommandRequest, EventEnvelope, Invocation, InvocationContext, RunError, Runner};

/// An action as listed in the palette and `plugin.action.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionInfo {
    /// `<plugin id>.<action id>`
    pub action_id: String,
    pub plugin_id: String,
    pub plugin_name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub contexts: Vec<ActionContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("plugin_not_found: no plugin {0:?}")]
    PluginNotFound(String),
    #[error("plugin_action_not_found: no action {0:?}")]
    ActionNotFound(String),
    #[error("plugin_pane_not_found: no pane {0:?}")]
    PaneNotFound(String),
    #[error("plugin_disabled: {0} is disabled")]
    Disabled(String),
    #[error("platform_unsupported: {0} is not available on {1}")]
    Platform(String, &'static str),
    #[error(transparent)]
    Run(#[from] RunError),
}

/// Splits `acme.tools.open` into (`acme.tools`, `open`). Local ids cannot
/// contain dots, so the last dot is the separator.
pub fn split_action_id(qualified: &str) -> Option<(&str, &str)> {
    qualified.rsplit_once('.')
}

pub struct PluginHost {
    dirs: PluginDirs,
    host_version: semver::Version,
    records: Vec<PluginRecord>,
    registry_mtime: Option<SystemTime>,
    runner: Runner,
}

impl PluginHost {
    pub fn new(dirs: PluginDirs, host_version: semver::Version, runner: Runner) -> Self {
        let mut host = Self {
            dirs,
            host_version,
            records: Vec::new(),
            registry_mtime: None,
            runner,
        };
        host.reload();
        host
    }

    pub fn dirs(&self) -> &PluginDirs {
        &self.dirs
    }

    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn records(&self) -> &[PluginRecord] {
        &self.records
    }

    fn registry_mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(self.dirs.registry_file())
            .and_then(|m| m.modified())
            .ok()
    }

    /// Re-reads the registry and every manifest.
    pub fn reload(&mut self) {
        self.registry_mtime = self.registry_mtime();
        self.records = match Registry::read(&self.dirs) {
            Ok(reg) => reg.records(&self.host_version),
            Err(e) => {
                tracing::warn!("reading plugin registry: {e}");
                Vec::new()
            }
        };
    }

    /// Reloads when `plugins.json` changed on disk (another `tt-git plugin`
    /// command ran). Returns whether it reloaded.
    pub fn reload_if_changed(&mut self) -> bool {
        if self.registry_mtime() != self.registry_mtime {
            self.reload();
            true
        } else {
            false
        }
    }

    fn runnable(&self) -> impl Iterator<Item = &PluginRecord> {
        self.records.iter().filter(|r| r.is_runnable())
    }

    pub fn actions(&self) -> Vec<ActionInfo> {
        let platform = manifest::current_platform();
        let mut out = Vec::new();
        for rec in self.runnable() {
            let m = &rec.manifest.as_ref().expect("runnable has manifest").manifest;
            for a in m.actions.iter().filter(|a| platform_matches(&a.platforms, platform)) {
                out.push(ActionInfo {
                    action_id: format!("{}.{}", m.id, a.id),
                    plugin_id: m.id.clone(),
                    plugin_name: m.name.clone(),
                    title: a.title.clone(),
                    description: a.description.clone(),
                    contexts: a.effective_contexts(),
                });
            }
        }
        out
    }

    fn record(&self, plugin_id: &str) -> Result<&PluginRecord, HostError> {
        let rec = self
            .records
            .iter()
            .find(|r| r.installed.id == plugin_id)
            .ok_or_else(|| HostError::PluginNotFound(plugin_id.to_string()))?;
        if !rec.installed.enabled {
            return Err(HostError::Disabled(plugin_id.to_string()));
        }
        if rec.manifest.is_none() {
            return Err(HostError::PluginNotFound(plugin_id.to_string()));
        }
        if !rec.is_runnable() {
            return Err(HostError::Platform(plugin_id.to_string(), manifest::current_platform()));
        }
        Ok(rec)
    }

    fn request(
        &self,
        rec: &PluginRecord,
        argv: Vec<String>,
        invocation: Invocation,
        context: InvocationContext,
    ) -> CommandRequest {
        let m = rec.manifest.as_ref().expect("runnable has manifest");
        CommandRequest {
            plugin_id: rec.installed.id.clone(),
            plugin_root: m.root.clone(),
            config_dir: self.dirs.plugin_config_dir(&rec.installed.id),
            state_dir: self.dirs.plugin_state_dir(&rec.installed.id),
            argv,
            invocation,
            context,
        }
    }

    /// Runs `<plugin id>.<action id>`. Returns the log id.
    pub fn invoke_action(&self, action_id: &str, context: InvocationContext) -> Result<u64, HostError> {
        let (plugin_id, local) =
            split_action_id(action_id).ok_or_else(|| HostError::ActionNotFound(action_id.to_string()))?;
        let rec = self.record(plugin_id)?;
        let m = &rec.manifest.as_ref().expect("checked").manifest;
        let action = m
            .actions
            .iter()
            .find(|a| a.id == local)
            .ok_or_else(|| HostError::ActionNotFound(action_id.to_string()))?;
        if !platform_matches(&action.platforms, manifest::current_platform()) {
            return Err(HostError::Platform(action_id.to_string(), manifest::current_platform()));
        }
        let req = self.request(
            rec,
            action.command.clone(),
            Invocation::Action {
                action_id: action_id.to_string(),
            },
            context,
        );
        Ok(self.runner.spawn(req)?)
    }

    /// Runs every enabled plugin's hooks for `envelope.event`.
    pub fn fire_event(&self, envelope: &EventEnvelope, context: &InvocationContext) -> Vec<Result<u64, HostError>> {
        let platform = manifest::current_platform();
        let mut results = Vec::new();
        for rec in self.runnable() {
            let m = &rec.manifest.as_ref().expect("runnable").manifest;
            for hook in m
                .events
                .iter()
                .filter(|e| e.on == envelope.event && platform_matches(&e.platforms, platform))
            {
                let mut ctx = context.clone();
                ctx.invocation_source = Some("event".into());
                let req = self.request(
                    rec,
                    hook.command.clone(),
                    Invocation::Event {
                        envelope: envelope.clone(),
                    },
                    ctx,
                );
                results.push(self.runner.spawn(req).map_err(HostError::from));
            }
        }
        results
    }

    /// Runs every enabled plugin's `[[startup]]` commands.
    pub fn run_startup(&self, context: &InvocationContext) {
        let platform = manifest::current_platform();
        for rec in self.runnable() {
            let m = &rec.manifest.as_ref().expect("runnable").manifest;
            for s in m.startup.iter().filter(|s| platform_matches(&s.platforms, platform)) {
                let mut ctx = context.clone();
                ctx.invocation_source = Some("startup".into());
                let req = self.request(rec, s.command.clone(), Invocation::Startup, ctx);
                if let Err(e) = self.runner.spawn(req) {
                    tracing::warn!(plugin = %rec.installed.id, "startup command not run: {e}");
                }
            }
        }
    }

    /// A pane entry point and the request to start it (in a PTY, by the UI).
    pub fn pane(
        &self,
        plugin_id: &str,
        entrypoint: &str,
        context: InvocationContext,
    ) -> Result<(PaneSpec, CommandRequest), HostError> {
        let rec = self.record(plugin_id)?;
        let m = &rec.manifest.as_ref().expect("checked").manifest;
        let pane = m
            .panes
            .iter()
            .find(|p| p.id == entrypoint)
            .ok_or_else(|| HostError::PaneNotFound(format!("{plugin_id}.{entrypoint}")))?;
        if !platform_matches(&pane.platforms, manifest::current_platform()) {
            return Err(HostError::Platform(
                format!("{plugin_id}.{entrypoint}"),
                manifest::current_platform(),
            ));
        }
        let req = self.request(
            rec,
            pane.command.clone(),
            Invocation::Pane {
                entrypoint: entrypoint.to_string(),
            },
            context,
        );
        Ok((pane.clone(), req))
    }

    /// Link handlers whose pattern matches `url`, in manifest order, as
    /// (plugin id, handler id, qualified action id).
    pub fn link_handlers_for(&self, url: &str) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for rec in self.runnable() {
            let m = &rec.manifest.as_ref().expect("runnable").manifest;
            for l in &m.link_handlers {
                if regex::Regex::new(&l.pattern).is_ok_and(|re| re.is_match(url)) {
                    out.push((m.id.clone(), l.id.clone(), format!("{}.{}", m.id, l.action)));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_qualified_ids() {
        assert_eq!(split_action_id("acme.tools.open"), Some(("acme.tools", "open")));
        assert_eq!(split_action_id("solo"), None);
    }
}
