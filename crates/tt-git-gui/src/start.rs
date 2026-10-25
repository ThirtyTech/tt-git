//! The start window, shown when `tt-git-gui` starts outside a repository:
//! the repositories opened recently, and a button for the folder picker.
//! Opening one opens its window ([`crate::windows`]) and closes this one.

use std::path::{Path, PathBuf};

use gpui::component::Root;
use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Render, StatefulInteractiveElement, Styled, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, div, px, size,
};
use tt_git_app::theme::Theme;
use tt_git_app::view::StyleRole;

use crate::font::MonoFont;
use crate::theme::GuiTheme;
use crate::{window_state, windows};

/// The start window's size.
const SIZE: (f32, f32) = (720., 480.);

/// A recent repository, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recent {
    pub path: PathBuf,
    /// The folder is gone (or isn't a folder any more).
    pub missing: bool,
}

/// The start window's view.
pub struct StartView {
    recent: Vec<Recent>,
    selected: usize,
    /// What went wrong opening the last one (shown above the list).
    error: Option<String>,
    theme: GuiTheme,
    font: MonoFont,
    focus: FocusHandle,
}

/// Opens the start window, with `error` above the list (why tt-git didn't
/// open a repository right away, say).
pub fn open(error: Option<String>, cx: &mut App) -> gpui::Result<WindowHandle<Root>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(SIZE.0), px(SIZE.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("tt-git".into()),
            ..Default::default()
        }),
        app_id: Some("tt-git".into()),
        ..Default::default()
    };
    let recent = window_state::state(cx)
        .recent
        .into_iter()
        .map(|path| Recent {
            missing: !path.is_dir(),
            path,
        })
        .collect();
    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| StartView::new(recent, error, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
}

impl StartView {
    fn new(recent: Vec<Recent>, error: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = GuiTheme::new(&Theme::default());
        let font = MonoFont::new(cx);
        theme.apply_to_components(&font, cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let selected = recent.iter().position(|r| !r.missing).unwrap_or(0);
        Self {
            recent,
            selected,
            error,
            theme,
            font,
            focus,
        }
    }

    pub fn recent(&self) -> &[Recent] {
        &self.recent
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Opens the recent repository at `index`. One that fails is dropped
    /// from the list (and from what's remembered) and the error shown.
    pub fn open_recent(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.recent.get(index).map(|r| r.path.clone()) else {
            return;
        };
        match windows::open_path(&path, cx) {
            Ok(_) => window.remove_window(),
            Err(e) => {
                window_state::forget_repo(&path, cx);
                self.recent.remove(index);
                self.selected = self.selected.min(self.recent.len().saturating_sub(1));
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    /// Asks for a folder with the platform's picker and opens it.
    pub fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let start = window.window_handle();
        windows::pick_repos(cx, move |result, cx| match result {
            Ok(0) => {}
            Ok(_) => {
                let _ = start.update(cx, |_, window, _| window.remove_window());
            }
            Err(e) => {
                let _ = this.update(cx, |view, cx| {
                    view.error = Some(e);
                    cx.notify();
                });
            }
        });
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let plain = !k.modifiers.control && !k.modifiers.alt && !k.modifiers.platform;
        match k.key.as_str() {
            "down" | "j" if plain => self.step(1),
            "up" | "k" if plain => self.step(-1),
            "enter" => self.open_recent(self.selected, window, cx),
            "o" if plain || k.modifiers.control || k.modifiers.platform => self.browse(window, cx),
            "q" | "escape" if plain => window.remove_window(),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn step(&mut self, by: isize) {
        if self.recent.is_empty() {
            return;
        }
        let last = self.recent.len() as isize - 1;
        self.selected = (self.selected as isize + by).clamp(0, last) as usize;
    }
}

impl Focusable for StartView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for StartView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let muted = theme.fg(StyleRole::Muted);
        let picked = theme.bg(StyleRole::Selected).unwrap_or(theme.fg(StyleRole::Border));
        let picked_fg = theme.role(StyleRole::Selected).color.unwrap_or(theme.foreground);
        let rows = self.recent.iter().enumerate().map(|(i, r)| {
            let mut row = div()
                .id(("recent", i))
                .px_2()
                .w_full()
                .cursor_pointer()
                .child(display_path(&r.path) + if r.missing { "  (missing)" } else { "" })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, window, cx| {
                        view.selected = i;
                        view.open_recent(i, window, cx);
                    }),
                );
            if r.missing {
                row = row.text_color(muted);
            }
            if i == self.selected {
                row = row.bg(picked).text_color(picked_fg);
            }
            row
        });
        let empty = self
            .recent
            .is_empty()
            .then(|| div().px_2().text_color(muted).child("No repositories opened yet."));
        div()
            .id("start")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(self.font.family.clone())
            .text_size(self.font.size)
            .line_height(self.font.line_height)
            .child(
                div()
                    .text_color(theme.fg(StyleRole::TitleFocused))
                    .child("Open a repository"),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_color(theme.fg(StyleRole::Error)).child(e)),
            )
            .child(
                div()
                    .id("browse")
                    .px_2()
                    .border_1()
                    .border_color(theme.fg(StyleRole::BorderFocused))
                    .cursor_pointer()
                    .child("Open folder…  (o)")
                    .on_click(cx.listener(|view, _, window, cx| view.browse(window, cx))),
            )
            .child(div().text_color(theme.fg(StyleRole::Title)).child("Recent"))
            .child(
                div()
                    .id("recent")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .children(empty),
            )
            .child(
                div()
                    .text_color(muted)
                    .child("enter open · ↑↓ pick · o browse · q close"),
            )
    }
}

/// `path` for people: `~` for the home folder, without Windows' `\\?\`.
fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    let text = text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text);
    match tt_git_core::paths::home_dir()
        .and_then(|home| Path::new(&text).strip_prefix(home).ok().map(Path::to_path_buf))
    {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => text,
    }
}
