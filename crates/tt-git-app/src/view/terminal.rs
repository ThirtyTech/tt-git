//! The view model for plugin and agent panes: where a pane goes, its title,
//! and its pseudo-terminal's screen as runs of equally styled cells, for the
//! TUI and the GUI to draw.
//!
//! Everything here is in terminal cells. The TUI's areas are cells already;
//! the GUI turns its pixels into cells with its font's cell size, and
//! reports that size with [`App::set_cell_size`](crate::App::set_cell_size)
//! so [`pty_size`] can size a new pane's PTY from the areas it reports.
//!
//! A cell's colors are the program's own, not the theme's, so a run carries
//! a [`Style`] rather than a [`StyleRole`](super::StyleRole): default colors
//! are left unset, for the front end's own text and background.

use tt_git_plugin::manifest::Placement;

use crate::geom::Rect;
use crate::pty_pane::PtyPane;
use crate::style::{Color, Modifier, Style};

/// The smallest PTY a new pane starts with, before its first frame sizes it.
const MIN_ROWS: u16 = 5;
const MIN_COLS: u16 = 20;
/// The smallest popup, border included.
const MIN_POPUP: (u16, u16) = (10, 4);

/// The rows and columns of a new pane's PTY drawn in `area` (border
/// included), in a front end whose cells are `cell` (width, height) of the
/// area's units: 1×1 in the TUI, the font's cell in pixels in the GUI.
/// The first frame resizes it to what really fits.
pub fn pty_size(area: Rect, cell: (u16, u16)) -> (u16, u16) {
    let (w, h) = (cell.0.max(1), cell.1.max(1));
    let rows = (area.height / h).saturating_sub(2).max(MIN_ROWS);
    let cols = (area.width / w).saturating_sub(2).max(MIN_COLS);
    (rows, cols)
}

/// Where a pane is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalSpot {
    /// In place of the details pane.
    Details,
    /// Over the whole area under the help bar.
    Main,
    /// In the middle of the area under the help bar, `cols` by `rows`
    /// cells, border included.
    Centered { cols: u16, rows: u16 },
}

/// A pane's screen: one row of runs per screen line, top to bottom, and the
/// cursor when it's shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalView {
    pub rows: Vec<Vec<CellRun>>,
    pub cursor: Option<TerminalCursor>,
}

/// Cells next to each other on a line that look the same. A double-width
/// character is a run of its own, two cells wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRun {
    /// The first cell's column.
    pub col: u16,
    /// Every cell's text, one after another; a blank cell is a space.
    pub text: String,
    pub style: Style,
    /// One double-width character.
    pub wide: bool,
    /// Where each cell's text ends in `text`.
    ends: Vec<usize>,
}

impl CellRun {
    /// How many columns the run covers.
    pub fn width(&self) -> u16 {
        let cells = self.ends.len() as u16;
        if self.wide { cells * 2 } else { cells }
    }

    /// Each cell's text, left to right (one for a wide run).
    pub fn cells(&self) -> impl Iterator<Item = &str> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts.zip(self.ends.iter().copied()).map(|(a, b)| &self.text[a..b])
    }

    fn push(&mut self, text: &str) {
        self.text.push_str(text);
        self.ends.push(self.text.len());
    }
}

/// Where the cursor is, and what's under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCursor {
    pub row: u16,
    pub col: u16,
    /// The text of the cell under it; a space when blank.
    pub text: String,
    /// It's on a double-width character.
    pub wide: bool,
}

impl PtyPane {
    /// The title: the pane's own, then how the process ended once it has.
    pub fn frame_title(&self) -> String {
        match &self.exited {
            Some(why) => format!("{} [{why}]", self.title),
            None => self.title.clone(),
        }
    }

    /// Where the pane goes, in an area under the help bar of `cols` by
    /// `rows` cells. A popup's size is the manifest's (cells or a share of
    /// the area), four fifths of the area by default.
    pub fn spot(&self, cols: u16, rows: u16) -> TerminalSpot {
        match self.placement {
            Placement::Split => TerminalSpot::Details,
            Placement::Popup => {
                let w = self.width.as_ref().map_or(cols * 4 / 5, |d| d.resolve(cols));
                let h = self.height.as_ref().map_or(rows * 4 / 5, |d| d.resolve(rows));
                TerminalSpot::Centered {
                    cols: w.max(MIN_POPUP.0),
                    rows: h.max(MIN_POPUP.1),
                }
            }
            _ => TerminalSpot::Main,
        }
    }

    /// Sizes the PTY to `rows` by `cols` (nothing changes for a zero) and
    /// returns its screen, cut to that size. The cursor is included only
    /// when the pane is `focused`, still running and hasn't hidden it.
    pub fn view(&mut self, rows: u16, cols: u16, focused: bool) -> TerminalView {
        self.resize(rows, cols);
        let parser = self.lock_parser();
        let screen = parser.screen();
        let rows_out = (0..rows).map(|row| row_runs(screen, row, cols)).collect();
        let cursor = (focused && self.exited.is_none() && !screen.hide_cursor())
            .then(|| screen.cursor_position())
            .filter(|&(r, c)| r < rows && c < cols)
            .map(|(row, col)| {
                let cell = screen.cell(row, col);
                TerminalCursor {
                    row,
                    col,
                    text: cell.map(cell_text).unwrap_or(" ").to_string(),
                    wide: cell.is_some_and(vt100::Cell::is_wide),
                }
            });
        TerminalView { rows: rows_out, cursor }
    }
}

/// The text along the bottom of a pane: how to leave it.
pub fn leave_hint(release_hint: &str) -> String {
    format!("{release_hint} to leave")
}

fn row_runs(screen: &vt100::Screen, row: u16, cols: u16) -> Vec<CellRun> {
    let mut runs: Vec<CellRun> = Vec::new();
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else { continue };
        if cell.is_wide_continuation() {
            continue;
        }
        let style = cell_style(cell);
        let wide = cell.is_wide();
        let text = cell_text(cell);
        match runs.last_mut() {
            Some(run) if !wide && !run.wide && run.style == style && run.col + run.width() == col => run.push(text),
            _ => {
                let mut run = CellRun {
                    col,
                    text: String::new(),
                    style,
                    wide,
                    ends: Vec::new(),
                };
                run.push(text);
                runs.push(run);
            }
        }
    }
    runs
}

fn cell_text(cell: &vt100::Cell) -> &str {
    let contents = cell.contents();
    if contents.is_empty() { " " } else { contents }
}

fn convert_color(c: vt100::Color) -> Option<Color> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

/// How a cell looks: its colors (unset for the default) and attributes.
pub fn cell_style(cell: &vt100::Cell) -> Style {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut p = vt100::Parser::new(rows, cols, 0);
        p.process(bytes);
        p
    }

    fn texts(runs: &[CellRun]) -> Vec<(u16, &str)> {
        runs.iter().map(|r| (r.col, r.text.as_str())).collect()
    }

    #[test]
    fn a_new_pty_fits_the_area_in_cells() {
        // The TUI: cells already, minus the border.
        assert_eq!(pty_size(Rect::new(0, 0, 80, 24), (1, 1)), (22, 78));
        // The GUI: 8×18 pixel cells.
        assert_eq!(pty_size(Rect::new(0, 0, 800, 540), (8, 18)), (28, 98));
        // Never smaller than a usable terminal, even with no area yet.
        assert_eq!(pty_size(Rect::default(), (8, 18)), (MIN_ROWS, MIN_COLS));
        // A zero cell counts as one.
        assert_eq!(pty_size(Rect::new(0, 0, 40, 10), (0, 0)), (8, 38));
    }

    #[test]
    fn runs_join_cells_that_look_the_same() {
        let p = screen(2, 12, b"ab\x1b[31mcd\x1b[0me");
        let runs = row_runs(p.screen(), 0, 12);
        assert_eq!(texts(&runs), [(0, "ab"), (2, "cd"), (4, "e       ")]);
        assert_eq!(runs[0].style, Style::new());
        assert_eq!(runs[1].style, Style::new().fg(Color::Indexed(1)));
        assert_eq!(runs[2].width(), 8);
        // A blank row is one run of spaces.
        let blank = row_runs(p.screen(), 1, 12);
        assert_eq!(texts(&blank), [(0, "            ")]);
        assert_eq!(blank[0].cells().count(), 12);
    }

    #[test]
    fn a_wide_character_is_a_run_of_its_own() {
        let p = screen(1, 6, "a漢b".as_bytes());
        let runs = row_runs(p.screen(), 0, 6);
        assert_eq!(texts(&runs), [(0, "a"), (1, "漢"), (3, "b  ")]);
        assert!(runs[1].wide);
        assert_eq!(runs[1].width(), 2);
        assert_eq!(runs[1].cells().collect::<Vec<_>>(), ["漢"]);
    }

    #[test]
    fn cells_keep_their_attributes() {
        let p = screen(1, 4, b"\x1b[1;3;4;7;38;2;1;2;3;48;5;200mx");
        let cell = p.screen().cell(0, 0).unwrap();
        let want = Style::new()
            .fg(Color::Rgb(1, 2, 3))
            .bg(Color::Indexed(200))
            .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED | Modifier::REVERSED);
        assert_eq!(cell_style(cell), want);
    }

    #[test]
    fn runs_stop_at_the_width() {
        let p = screen(1, 10, b"0123456789");
        assert_eq!(texts(&row_runs(p.screen(), 0, 4)), [(0, "0123")]);
    }

    #[test]
    fn the_leave_hint_names_the_key() {
        assert_eq!(leave_hint("f12"), "f12 to leave");
    }
}
