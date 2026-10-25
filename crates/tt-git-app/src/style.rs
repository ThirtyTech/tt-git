//! Colors and text styles that don't depend on a front end. The TUI turns
//! them into ratatui's; a GUI turns them into its own.

/// A color: one of the terminal's 16 named ones, an index into its 256
/// color palette, RGB, or the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Color {
    /// The front end's default (the terminal's own foreground or background).
    #[default]
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

/// Text attributes. The bits match ratatui's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifier(u16);

impl Modifier {
    pub const BOLD: Self = Self(0b0000_0000_0001);
    pub const DIM: Self = Self(0b0000_0000_0010);
    pub const ITALIC: Self = Self(0b0000_0000_0100);
    pub const UNDERLINED: Self = Self(0b0000_0000_1000);
    pub const SLOW_BLINK: Self = Self(0b0000_0001_0000);
    pub const RAPID_BLINK: Self = Self(0b0000_0010_0000);
    pub const REVERSED: Self = Self(0b0000_0100_0000);
    pub const HIDDEN: Self = Self(0b0000_1000_0000);
    pub const CROSSED_OUT: Self = Self(0b0001_0000_0000);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl std::ops::BitOr for Modifier {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Modifier {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// How text looks: colors and attributes, each left as it is underneath
/// when unset. Built like ratatui's `Style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    /// Attributes turned on.
    pub add_modifier: Modifier,
    /// Attributes turned off.
    pub sub_modifier: Modifier,
}

impl Style {
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::empty(),
        }
    }

    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    pub const fn add_modifier(mut self, modifier: Modifier) -> Self {
        self.sub_modifier = self.sub_modifier.difference(modifier);
        self.add_modifier = self.add_modifier.union(modifier);
        self
    }

    pub const fn remove_modifier(mut self, modifier: Modifier) -> Self {
        self.add_modifier = self.add_modifier.difference(modifier);
        self.sub_modifier = self.sub_modifier.union(modifier);
        self
    }

    /// `other` laid over this: its colors where it sets them, and its
    /// attributes turned on and off.
    pub const fn patch(mut self, other: Self) -> Self {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        if other.bg.is_some() {
            self.bg = other.bg;
        }
        self.add_modifier = self
            .add_modifier
            .difference(other.sub_modifier)
            .union(other.add_modifier);
        self.sub_modifier = self
            .sub_modifier
            .difference(other.add_modifier)
            .union(other.sub_modifier);
        self
    }
}
