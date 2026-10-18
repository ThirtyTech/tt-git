//! Plugin panes in the layout: overlays and popups float over everything
//! and take all keys; split panes replace the details pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use serde_json::{Value, json};
use tt_git_plugin::manifest::{Dimension, Placement};

use super::App;
use crate::keymap::KeyCombo;
use crate::panes::PaneId;
use crate::pty_pane::{self, PtyPane};

/// Optional overrides when opening a pane (from the API/CLI).
#[derive(Debug, Clone, Default)]
pub struct PaneRequest {
    pub placement: Option<Placement>,
    pub width: Option<Dimension>,
    pub height: Option<Dimension>,
    pub env: Vec<(String, String)>,
    pub focus: bool,
}

impl App {
    /// Starts a plugin's `[[panes]]` entry point. Returns the pane id.
    pub fn open_pane(
        &mut self,
        plugin_id: &str,
        entrypoint: &str,
        req: PaneRequest,
    ) -> Result<pty_pane::PaneId, String> {
        let context = self.invocation_context("pane");
        let state = self.plugins.as_ref().ok_or("plugins are not enabled")?;
        let (mut spec, command) = state
            .host
            .pane(plugin_id, entrypoint, context)
            .map_err(|e| e.to_string())?;
        if let Some(w) = req.width {
            spec.width = Some(w);
        }
        if let Some(h) = req.height {
            spec.height = Some(h);
        }
        let placement = req.placement.unwrap_or(spec.placement);
        if placement == Placement::Popup && self.panes.iter().any(|p| p.placement == Placement::Popup) {
            return Err("ui_busy: another popup is open".into());
        }
        let host_env = state.host.runner().host();
        let (main, details) = self.layout_rects;
        let target = match placement {
            Placement::Split => details,
            _ => main,
        };
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        let pane = PtyPane::spawn(
            id,
            &spec,
            Some(placement),
            &command,
            &host_env,
            &req.env,
            target.height.saturating_sub(2).max(5),
            target.width.saturating_sub(2).max(20),
            self.ctx.clone(),
        )?;
        if pane.placement == Placement::Split {
            // One split at a time: it takes the details pane's place.
            self.panes.retain(|p| p.placement != Placement::Split);
            self.details_open = true;
            if req.focus {
                self.focus = PaneId::Details;
            }
        }
        self.panes.push(pane);
        Ok(id)
    }

    pub fn close_pane(&mut self, id: pty_pane::PaneId) -> bool {
        let before = self.panes.len();
        self.panes.retain(|p| p.id != id);
        before != self.panes.len()
    }

    pub fn plugin_panes(&self) -> &[PtyPane] {
        &self.panes
    }

    /// Screen text of a pane (tests, `plugin.pane.screen`).
    pub fn pane_screen(&self, id: pty_pane::PaneId) -> Option<String> {
        self.panes.iter().find(|p| p.id == id).map(PtyPane::screen_text)
    }

    fn top_modal_pane(&mut self) -> Option<&mut PtyPane> {
        self.panes.iter_mut().rev().find(|p| p.is_modal())
    }

    fn split_pane(&mut self) -> Option<&mut PtyPane> {
        self.panes.iter_mut().find(|p| p.placement == Placement::Split)
    }

    pub(super) fn has_split_pane(&self) -> bool {
        self.panes.iter().any(|p| p.placement == Placement::Split)
    }

    fn is_release_key(&self, key: KeyCombo) -> bool {
        self.release_keys.contains(&key)
    }

    /// Routes a key to a focused plugin pane. Returns whether it was taken.
    pub(super) fn pane_key(&mut self, key: KeyCombo) -> bool {
        let release = self.is_release_key(key);
        if let Some(pane) = self.top_modal_pane() {
            let id = pane.id;
            if release || pane.exited.is_some() {
                self.close_pane(id);
            } else {
                pane.send_key(key);
            }
            return true;
        }
        if self.focus == PaneId::Details && self.has_split_pane() {
            let pane = self.split_pane().expect("checked");
            let (id, exited) = (pane.id, pane.exited.is_some());
            if release {
                if exited {
                    self.close_pane(id);
                }
                self.focus = PaneId::Graph;
                self.settle_focus(PaneId::Details);
                self.refresh_details(false);
            } else if exited {
                // Any key dismisses a finished split pane.
                self.close_pane(id);
            } else {
                self.split_pane().expect("checked").send_key(key);
            }
            return true;
        }
        false
    }

    pub(super) fn pane_exited(&mut self, id: pty_pane::PaneId, status: String) {
        let Some(pane) = self.panes.iter_mut().find(|p| p.id == id) else {
            return;
        };
        pane.mark_exited(status);
        let why = pane.exited.clone().unwrap_or_default();
        let title = pane.title.clone();
        if pane.is_modal() {
            self.close_pane(id);
        }
        self.notice = Some(format!("{title}: {why}"));
    }

    /// Draws the split pane into `details` if there is one. Returns whether it did.
    pub(super) fn draw_split_pane(&mut self, frame: &mut Frame, details: Rect) -> bool {
        let focused = self.focus == PaneId::Details;
        let hint = self.release_hint();
        let theme = self.theme.clone();
        match self.split_pane() {
            Some(pane) => {
                pane.draw(frame, details, focused, &hint, &theme);
                true
            }
            None => false,
        }
    }

    pub(super) fn draw_modal_panes(&mut self, frame: &mut Frame, main: Rect, details: Rect) {
        let hint = self.release_hint();
        let theme = self.theme.clone();
        let top = self.panes.iter().rposition(|p| p.is_modal());
        for (i, pane) in self.panes.iter_mut().enumerate().filter(|(_, p)| p.is_modal()) {
            let area = pane.area(main, details);
            pane.draw(frame, area, Some(i) == top, &hint, &theme);
        }
    }

    fn release_hint(&self) -> String {
        self.release_keys.first().map(ToString::to_string).unwrap_or_default()
    }

    /// `plugin.pane.*` and `popup.close` socket methods.
    pub(super) fn pane_api(&mut self, method: &str, params: &Value) -> Result<Value, (&'static str, String)> {
        let id_param = || {
            params
                .get("pane_id")
                .and_then(Value::as_u64)
                .ok_or(("invalid_params", "pane_id is required".to_string()))
        };
        match method {
            "plugin.pane.open" => {
                let plugin_id = params
                    .get("plugin_id")
                    .and_then(Value::as_str)
                    .ok_or(("invalid_params", "plugin_id is required".into()))?
                    .to_string();
                let entrypoint = params
                    .get("entrypoint")
                    .and_then(Value::as_str)
                    .ok_or(("invalid_params", "entrypoint is required".into()))?
                    .to_string();
                let placement = match params.get("placement") {
                    Some(v) if !v.is_null() => Some(
                        serde_json::from_value(v.clone()).map_err(|e| ("invalid_params", format!("placement: {e}")))?,
                    ),
                    _ => None,
                };
                let dim = |key: &str| -> Result<Option<Dimension>, (&'static str, String)> {
                    match params.get(key) {
                        Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                            .map(Some)
                            .map_err(|e| ("invalid_params", format!("{key}: {e}"))),
                        _ => Ok(None),
                    }
                };
                let env = params
                    .get("env")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                let req = PaneRequest {
                    placement,
                    width: dim("width")?,
                    height: dim("height")?,
                    env,
                    focus: params.get("focus").and_then(Value::as_bool).unwrap_or(true),
                };
                let id = self.open_pane(&plugin_id, &entrypoint, req).map_err(|e| {
                    let code = if e.starts_with("ui_busy") {
                        "ui_busy"
                    } else {
                        "plugin_pane_failed"
                    };
                    (code, e)
                })?;
                Ok(json!({ "pane_id": id }))
            }
            "plugin.pane.focus" => {
                let id = id_param()?;
                let pane = self
                    .panes
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or(("plugin_pane_not_found", format!("no pane {id}")))?;
                if pane.placement == Placement::Split {
                    self.focus = PaneId::Details;
                }
                Ok(json!({}))
            }
            "plugin.pane.close" => {
                let id = id_param()?;
                if self.close_pane(id) {
                    Ok(json!({}))
                } else {
                    Err(("plugin_pane_not_found", format!("no pane {id}")))
                }
            }
            "plugin.pane.screen" => {
                let id = id_param()?;
                let text = self
                    .pane_screen(id)
                    .ok_or(("plugin_pane_not_found", format!("no pane {id}")))?;
                Ok(json!({ "text": text }))
            }
            "plugin.pane.list" => Ok(Value::Array(
                self.panes
                    .iter()
                    .map(|p| {
                        json!({
                            "pane_id": p.id,
                            "plugin_id": p.plugin_id,
                            "entrypoint": p.entrypoint,
                            "placement": p.placement,
                            "exited": p.exited,
                        })
                    })
                    .collect(),
            )),
            _ => {
                // popup.close
                let popup = self
                    .panes
                    .iter()
                    .find(|p| p.placement == Placement::Popup)
                    .map(|p| p.id);
                match popup {
                    Some(id) => {
                        self.close_pane(id);
                        Ok(json!({ "closed": id }))
                    }
                    None => Ok(json!({ "closed": null })),
                }
            }
        }
    }
}
