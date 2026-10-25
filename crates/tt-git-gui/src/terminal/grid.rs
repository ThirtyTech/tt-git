//! [`TerminalGrid`]: a custom element that paints a pane's screen, the way
//! Zed's terminal view does: each run of equally styled cells shaped as one
//! line of text with every glyph forced onto the cell grid, cell
//! backgrounds as quads under it, and the cursor as a block over it.

use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Font, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, ShapedLine, SharedString, StrikethroughStyle, Style, TextAlign,
    TextRun, UnderlineStyle, Window, fill, point, relative, size,
};
use tt_git_app::view::terminal::TerminalView;

use crate::font::MonoFont;
use crate::theme::GuiTheme;
use crate::ui::Measures;

/// A run of cells, its look resolved to colors and a font.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintRun {
    pub row: u16,
    pub col: u16,
    /// Columns it covers.
    pub width: u16,
    pub text: SharedString,
    pub color: Hsla,
    pub background: Option<Hsla>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// One double-width character, drawn as it shapes rather than forced
    /// into one column.
    pub wide: bool,
}

/// The block cursor: where, and the text under it.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintCursor {
    pub row: u16,
    pub col: u16,
    pub width: u16,
    pub text: SharedString,
}

/// A pane's screen, ready to paint. Fills its parent, and records where it
/// was laid out as `id` in [`Measures`] (so [`Ui::fit`](crate::ui::Ui::fit)
/// says how many rows and columns the PTY gets next frame).
pub struct TerminalGrid {
    id: SharedString,
    measures: Measures,
    font: Rc<MonoFont>,
    pub runs: Vec<PaintRun>,
    pub cursor: Option<PaintCursor>,
    cursor_color: Hsla,
    cursor_text: Hsla,
}

impl TerminalGrid {
    /// `view` in `theme`'s colors: default colors are the window's text
    /// and background.
    pub fn new(
        id: SharedString,
        measures: Measures,
        font: Rc<MonoFont>,
        theme: &GuiTheme,
        view: &TerminalView,
    ) -> Self {
        let mut runs = Vec::new();
        for (row, line) in (0..).zip(&view.rows) {
            for run in line {
                let look = theme.style(run.style);
                let mut color = look.color.unwrap_or(theme.foreground);
                if let Some(fade) = look.fade_out {
                    color.fade_out(fade);
                }
                runs.push(PaintRun {
                    row,
                    col: run.col,
                    width: run.width(),
                    text: run.text.clone().into(),
                    color,
                    background: look.background_color,
                    bold: look.font_weight.is_some_and(|w| w == FontWeight::BOLD),
                    italic: look.font_style == Some(FontStyle::Italic),
                    underline: look.underline.is_some(),
                    strikethrough: look.strikethrough.is_some(),
                    wide: run.wide,
                });
            }
        }
        let cursor = view.cursor.as_ref().map(|c| PaintCursor {
            row: c.row,
            col: c.col,
            width: if c.wide { 2 } else { 1 },
            text: c.text.clone().into(),
        });
        Self {
            id,
            measures,
            font,
            runs,
            cursor,
            cursor_color: theme.foreground,
            cursor_text: theme.background,
        }
    }

    fn font(&self, bold: bool, italic: bool) -> Font {
        let mut font = self.font.font();
        if bold {
            font.weight = FontWeight::BOLD;
        }
        if italic {
            font.style = FontStyle::Italic;
        }
        font
    }

    fn cell_bounds(&self, origin: gpui::Point<Pixels>, row: u16, col: u16, width: u16) -> Bounds<Pixels> {
        let (w, h) = (self.font.cell_width, self.font.line_height);
        Bounds {
            origin: point(origin.x + w * f32::from(col), origin.y + h * f32::from(row)),
            size: size(w * f32::from(width), h),
        }
    }

    fn shape(&self, text: &SharedString, run: TextRun, wide: bool, window: &Window) -> ShapedLine {
        // Every glyph on its own column, whatever font it fell back to.
        let force = (!wide).then_some(self.font.cell_width);
        window
            .text_system()
            .shape_line(text.clone(), self.font.size, &[run], force)
    }
}

/// What prepaint works out for paint.
pub struct GridPaint {
    quads: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(gpui::Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, ShapedLine)>,
}

impl IntoElement for TerminalGrid {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TerminalGrid {
    type RequestLayoutState = ();
    type PrepaintState = GridPaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> GridPaint {
        if self.measures.record(&self.id, bounds) {
            // The PTY gets the new size next frame. GPUI ignores `refresh`
            // while it draws, so ask for that frame.
            window.on_next_frame(|window, _| window.refresh());
        }
        let origin = bounds.origin;
        let mut quads = Vec::new();
        let mut lines = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            let cell = self.cell_bounds(origin, run.row, run.col, run.width);
            if let Some(bg) = run.background {
                quads.push((cell, bg));
            }
            if run.text.trim().is_empty() && !run.underline && !run.strikethrough {
                continue;
            }
            let text_run = TextRun {
                len: run.text.len(),
                font: self.font(run.bold, run.italic),
                color: run.color,
                background_color: None,
                underline: run.underline.then(|| UnderlineStyle {
                    thickness: gpui::px(1.),
                    color: Some(run.color),
                    wavy: false,
                }),
                strikethrough: run.strikethrough.then(|| StrikethroughStyle {
                    thickness: gpui::px(1.),
                    color: Some(run.color),
                }),
            };
            lines.push((cell.origin, self.shape(&run.text, text_run, run.wide, window)));
        }
        let cursor = self.cursor.as_ref().map(|c| {
            let cell = self.cell_bounds(origin, c.row, c.col, c.width);
            let run = TextRun {
                len: c.text.len(),
                font: self.font(false, false),
                color: self.cursor_text,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            (cell, self.shape(&c.text, run, c.width > 1, window))
        });
        GridPaint { quads, lines, cursor }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        paint: &mut GridPaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = self.font.line_height;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (cell, color) in &paint.quads {
                window.paint_quad(fill(*cell, *color));
            }
            for (origin, line) in &paint.lines {
                let _ = line.paint(*origin, line_height, TextAlign::Left, None, window, cx);
            }
            if let Some((cell, text)) = &paint.cursor {
                window.paint_quad(fill(*cell, self.cursor_color));
                let _ = text.paint(cell.origin, line_height, TextAlign::Left, None, window, cx);
            }
        });
    }
}
