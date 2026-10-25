//! Plugin panes: a plugin entry point running in a pseudo-terminal, drawn
//! from a vt100 emulator into the frame (herdr-style "a pane is a terminal").

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear};
use tt_git_plugin::manifest::{Dimension, PaneSpec, Placement};
use tt_git_plugin::runtime::{CommandRequest, HostEnv, plugin_env, resolve_program};

use crate::action::Action;
use crate::jobs::Ctx;
use crate::keymap::{KeyCode, KeyCombo, KeyModifiers};
use crate::theme::Theme;

pub type PaneId = u64;

/// The `plugin_id` of panes tt-git opens itself rather than for a plugin.
pub const BUILTIN: &str = "tt-git";

/// Who a pane belongs to and where it goes.
struct PaneMeta {
    plugin_id: String,
    entrypoint: String,
    title: String,
    placement: Placement,
    width: Option<Dimension>,
    height: Option<Dimension>,
}

/// Lines the emulator keeps after they scroll off. Panes only ever show the
/// live screen (nothing scrolls back), so none are kept.
const SCROLLBACK: usize = 0;

pub struct PtyPane {
    pub id: PaneId,
    pub plugin_id: String,
    pub entrypoint: String,
    pub title: String,
    pub placement: Placement,
    pub width: Option<Dimension>,
    pub height: Option<Dimension>,
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    size: (u16, u16),
    /// A `PaneOutput` was sent and the app hasn't handled it yet, so more
    /// output needn't send another (one redraw shows it all).
    output_pending: Arc<AtomicBool>,
    /// Set once the process ended; the text describes how.
    pub exited: Option<String>,
}

impl std::fmt::Debug for PtyPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyPane")
            .field("id", &self.id)
            .field("plugin_id", &self.plugin_id)
            .field("entrypoint", &self.entrypoint)
            .field("placement", &self.placement)
            .finish()
    }
}

/// tt-git has no tabs or zoom; those placements open as overlays.
fn effective_placement(p: Placement) -> Placement {
    match p {
        Placement::Tab | Placement::Zoomed => Placement::Overlay,
        other => other,
    }
}

impl PtyPane {
    /// Starts `req` in a new PTY of `rows` x `cols`. Output and exit are
    /// reported to the app as `PaneOutput` / `PaneExited` actions.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        id: PaneId,
        spec: &PaneSpec,
        placement: Option<Placement>,
        req: &CommandRequest,
        host: &HostEnv,
        extra_env: &[(String, String)],
        rows: u16,
        cols: u16,
        ctx: Ctx,
    ) -> Result<Self, String> {
        let (program, args) = req.argv.split_first().ok_or("pane command is empty")?;
        let mut cmd = CommandBuilder::new(resolve_program(program, &req.plugin_root));
        cmd.args(args);
        cmd.cwd(&req.plugin_root);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        // Host-managed variables last so they win.
        for (k, v) in plugin_env(host, req) {
            cmd.env(k, v);
        }
        let meta = PaneMeta {
            plugin_id: req.plugin_id.clone(),
            entrypoint: spec.id.clone(),
            title: spec.title.clone(),
            placement: effective_placement(placement.unwrap_or(spec.placement)),
            width: spec.width.clone(),
            height: spec.height.clone(),
        };
        Self::start(id, cmd, program, meta, rows, cols, ctx)
    }

    /// Starts `argv` in `cwd` as a split pane titled `title`, for tt-git's
    /// own use (the agent). It reports as plugin [`BUILTIN`].
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_command(
        id: PaneId,
        entrypoint: &str,
        title: &str,
        argv: &[String],
        cwd: &std::path::Path,
        rows: u16,
        cols: u16,
        ctx: Ctx,
    ) -> Result<Self, String> {
        let (program, args) = argv.split_first().ok_or("pane command is empty")?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        let meta = PaneMeta {
            plugin_id: BUILTIN.into(),
            entrypoint: entrypoint.into(),
            title: title.into(),
            placement: Placement::Split,
            width: None,
            height: None,
        };
        Self::start(id, cmd, program, meta, rows, cols, ctx)
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        id: PaneId,
        cmd: CommandBuilder,
        program: &str,
        meta: PaneMeta,
        rows: u16,
        cols: u16,
        ctx: Ctx,
    ) -> Result<Self, String> {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("could not open a pty: {e}"))?;
        let mut child = pty
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("could not start {program:?}: {e}"))?;
        let killer = child.clone_killer();
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = Arc::new(Mutex::new(pty.master.take_writer().map_err(|e| e.to_string())?));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK)));

        let p = parser.clone();
        let w = writer.clone();
        let output_pending = Arc::new(AtomicBool::new(false));
        let pending = output_pending.clone();
        let exit_ctx = ctx.clone();
        std::thread::Builder::new()
            .name("tt-git-pane-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = &buf[..n];
                            let mut parser = p.lock().unwrap_or_else(|e| e.into_inner());
                            parser.process(chunk);
                            // Answer cursor-position queries like a real terminal;
                            // ConPTY sends one at startup and waits for the reply.
                            if contains(chunk, b"[6n") {
                                let (r, c) = parser.screen().cursor_position();
                                let reply = format!("[{};{}R", r + 1, c + 1);
                                let mut w = w.lock().unwrap_or_else(|e| e.into_inner());
                                let _ = w.write_all(reply.as_bytes());
                                let _ = w.flush();
                            }
                            drop(parser);
                            // AcqRel pairs with `take_output`, so the redraw it
                            // leads to sees everything processed above.
                            if !pending.swap(true, Ordering::AcqRel) {
                                ctx.emit(Action::PaneOutput(id));
                            }
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        // Exit is detected by waiting on the child: ConPTY keeps the output
        // pipe open after the process ends, so EOF alone is not enough.
        std::thread::Builder::new()
            .name("tt-git-pane-wait".into())
            .stack_size(crate::IO_THREAD_STACK)
            .spawn(move || {
                let status = match child.wait() {
                    Ok(s) if s.success() => "process exited".to_string(),
                    Ok(s) => format!("process exited with code {}", s.exit_code()),
                    Err(e) => format!("process ended: {e}"),
                };
                exit_ctx.emit(Action::PaneExited { id, status });
            })
            .map_err(|e| e.to_string())?;

        Ok(Self {
            id,
            plugin_id: meta.plugin_id,
            entrypoint: meta.entrypoint,
            title: meta.title,
            placement: meta.placement,
            width: meta.width,
            height: meta.height,
            parser,
            writer,
            master: pty.master,
            killer,
            size: (rows, cols),
            output_pending,
            exited: None,
        })
    }

    /// Notes that the app handled this pane's `PaneOutput`: output after
    /// this sends a new one. Call before drawing.
    pub fn take_output(&self) {
        self.output_pending.swap(false, Ordering::AcqRel);
    }

    pub fn is_modal(&self) -> bool {
        matches!(self.placement, Placement::Overlay | Placement::Popup)
    }

    /// The emulated screen as plain text (for tests and the API).
    pub fn screen_text(&self) -> String {
        self.parser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .screen()
            .contents()
    }

    pub fn send_key(&mut self, key: KeyCombo) {
        let app_cursor = self
            .parser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .screen()
            .application_cursor();
        let bytes = encode_key(key, app_cursor);
        if !bytes.is_empty() {
            self.send_bytes(&bytes);
        }
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    /// Records how the process ended.
    pub fn mark_exited(&mut self, status: String) {
        self.exited = Some(status);
    }

    pub fn kill(&mut self) {
        if self.exited.is_none() {
            let _ = self.killer.kill();
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if (rows, cols) == self.size || rows == 0 || cols == 0 {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .screen_mut()
            .set_size(rows, cols);
    }

    /// The area this pane occupies given the main area and the details area
    /// (used by split panes).
    pub fn area(&self, main: Rect, details: Rect) -> Rect {
        match self.placement {
            Placement::Split => details,
            Placement::Popup => {
                let w = self
                    .width
                    .as_ref()
                    .map_or(main.width * 4 / 5, |d| d.resolve(main.width));
                let h = self
                    .height
                    .as_ref()
                    .map_or(main.height * 4 / 5, |d| d.resolve(main.height));
                crate::app::centered(main, w.max(10), h.max(4))
            }
            _ => main,
        }
    }

    /// Draws the pane. Returns whether it placed the terminal cursor.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool, release_hint: &str, theme: &Theme) -> bool {
        frame.render_widget(Clear, area);
        let mut title = format!(" {} ", self.title);
        if let Some(why) = &self.exited {
            title.push_str(&format!("[{why}] "));
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(if focused { theme.border_focused } else { theme.border })
            .title(Span::styled(
                title,
                if focused { theme.title_focused } else { theme.title },
            ))
            .title_bottom(Span::styled(format!(" {release_hint} to leave "), theme.muted));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.resize(inner.height, inner.width);

        let parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let screen = parser.screen();
        let buf = frame.buffer_mut();
        for row in 0..inner.height {
            for col in 0..inner.width {
                let Some(cell) = screen.cell(row, col) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let target = &mut buf[(inner.x + col, inner.y + row)];
                let contents = cell.contents();
                target.set_symbol(if contents.is_empty() { " " } else { contents });
                target.set_style(cell_style(cell));
            }
        }
        if focused && self.exited.is_none() && !screen.hide_cursor() {
            let (r, c) = screen.cursor_position();
            if r < inner.height && c < inner.width {
                frame.set_cursor_position(Position::new(inner.x + c, inner.y + r));
                return true;
            }
        }
        false
    }
}

impl Drop for PtyPane {
    fn drop(&mut self) {
        self.kill();
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn convert_color(c: vt100::Color) -> Option<Color> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::new();
    if let Some(fg) = convert_color(cell.fgcolor()) {
        style = style.fg(fg);
    }
    if let Some(bg) = convert_color(cell.bgcolor()) {
        style = style.bg(bg);
    }
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    style.add_modifier(m)
}

/// The bytes a terminal sends for a key press.
pub fn encode_key(key: KeyCombo, application_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let arrow = |c: u8| {
        if application_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    let mut out = match key.code {
        KeyCode::Char(c) if ctrl => {
            let lower = c.to_ascii_lowercase();
            match lower {
                'a'..='z' => vec![lower as u8 - b'a' + 1],
                ' ' | '@' | '2' => vec![0],
                '[' | '3' => vec![0x1b],
                '\\' | '4' => vec![0x1c],
                ']' | '5' => vec![0x1d],
                '^' | '6' => vec![0x1e],
                '_' | '7' | '/' => vec![0x1f],
                _ => c.to_string().into_bytes(),
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => arrow(b'A'),
        KeyCode::Down => arrow(b'B'),
        KeyCode::Right => arrow(b'C'),
        KeyCode::Left => arrow(b'D'),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + (n - 1)],
        KeyCode::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return Vec::new(),
            };
            format!("\x1b[{code}~").into_bytes()
        }
        _ => Vec::new(),
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyCombo {
        s.parse().unwrap()
    }

    #[test]
    fn encodes_keys() {
        assert_eq!(encode_key(k("a"), false), b"a");
        assert_eq!(encode_key(k("A"), false), b"A");
        assert_eq!(encode_key(KeyCombo::char('é'), false), "é".as_bytes());
        assert_eq!(encode_key(k("ctrl+c"), false), [3]);
        assert_eq!(encode_key(k("ctrl+\\"), false), [0x1c]);
        assert_eq!(encode_key(k("alt+x"), false), b"\x1bx");
        assert_eq!(encode_key(k("enter"), false), b"\r");
        assert_eq!(encode_key(k("backspace"), false), [0x7f]);
        assert_eq!(encode_key(k("up"), false), b"\x1b[A");
        assert_eq!(encode_key(k("up"), true), b"\x1bOA");
        assert_eq!(encode_key(k("shift+tab"), false), b"\x1b[Z");
        assert_eq!(encode_key(k("f1"), false), b"\x1bOP");
        assert_eq!(encode_key(k("f5"), false), b"\x1b[15~");
        assert_eq!(encode_key(k("pagedown"), false), b"\x1b[6~");
    }
}
