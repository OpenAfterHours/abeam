//! The reader's two palettes, and the switch between them.
//!
//! Scoped to the viewer on purpose. The git and diagnostics panes draw in
//! *named* ANSI colours, which resolve against whatever palette the host
//! terminal has and therefore already follow it. This pane cannot do that,
//! because it is the one pane that paints its own background — see
//! [`Theme::base`] — and the moment a background is painted, every colour on
//! top of it has to be owned too. A `Color::Cyan` resolved from a dark terminal
//! profile lands on our white page as unreadable pale blue. So everything here
//! is absolute RGB, twice, and none of it is negotiable with the terminal.
//!
//! Why paint at all, when the rest of abeam is careful not to: this pane is for
//! *reading*, and reading in a bright room wants a bright page regardless of
//! what the terminal around it is set to. One key has to be enough — a toggle
//! that also required changing the Windows Terminal profile would be half a
//! feature.
//!
//! Both palettes are contrast-checked against their own background rather than
//! chosen by eye: body text clears 7:1, recessive chrome clears 3:1, and every
//! accent clears 4:1. The dark set is base16-ocean, which is also the syntax
//! theme, so highlighted code and the chrome around it agree. The one departure
//! is `danger`: base16's `#bf616a` measures 3.23:1 on its own background, which
//! is under the floor for the one colour whose entire job is to be noticed, so
//! it is lightened to `#d97a83` (4.45:1). The light set is GitHub's, to match
//! `InspiredGitHub` on the syntax side.

use ratatui::style::{Color, Modifier, Style};

/// Which palette the reader is using. Toggled by F3, and nothing else reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

impl Mode {
    pub fn flipped(self) -> Self {
        match self {
            Mode::Dark => Mode::Light,
            Mode::Light => Mode::Dark,
        }
    }

    pub fn theme(self) -> &'static Theme {
        match self {
            Mode::Dark => &DARK,
            Mode::Light => &LIGHT,
        }
    }

    /// The syntect theme to highlight code with. Named rather than held as a
    /// value because loading one costs ~100 ms and both are built once, behind
    /// the `OnceLock` in `source`.
    pub fn syntax(self) -> &'static str {
        match self {
            Mode::Dark => "base16-ocean.dark",
            Mode::Light => "InspiredGitHub",
        }
    }
}

/// One palette. Fields are raw colours rather than styles because several
/// callers need to *patch* a colour onto a style they are already carrying —
/// inline code inside a bold heading keeps the bold.
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    /// Everything the pane says in its own voice: gutters, line numbers, the
    /// hashes on a heading, hints. Recessive, never invisible.
    pub dim: Color,
    /// Inline `code` spans.
    pub code: Color,
    /// Link text, always underlined as well — colour alone is not a signal
    /// every reader receives.
    pub link: Color,
    /// List bullets and numbers, and directories in the file list.
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub danger: Color,
    pub special: Color,
    pub info: Color,
    /// The selected row in the file list.
    pub sel_bg: Color,
    pub sel_fg: Color,
}

impl Theme {
    /// The page itself: foreground *and* background, applied to the whole rect
    /// before anything is drawn on it.
    ///
    /// ratatui styles are patches, so a span that names only a foreground keeps
    /// this background, and a blank row keeps both. That is what makes one fill
    /// enough for the whole pane.
    pub fn base(&self) -> Style {
        Style::new().fg(self.fg).bg(self.bg)
    }

    pub fn dim(&self) -> Style {
        Style::new().fg(self.dim)
    }

    /// Colour by level rather than size, since a terminal has no size. Distinct
    /// hues beat shades: a reader skimming for the next section is pattern
    /// matching, not reading.
    pub fn heading(&self, level: u8) -> Style {
        let colour = match level {
            1 => self.special,
            2 => self.accent,
            3 => self.ok,
            _ => self.warn,
        };
        Style::new().fg(colour).add_modifier(Modifier::BOLD)
    }

    pub fn selection(&self) -> Style {
        Style::new().fg(self.sel_fg).bg(self.sel_bg)
    }
}

/// base16-ocean, matching the syntax theme of the same name.
pub static DARK: Theme = Theme {
    bg: Color::Rgb(0x2b, 0x30, 0x3b),
    fg: Color::Rgb(0xc0, 0xc5, 0xce),
    dim: Color::Rgb(0x8a, 0x94, 0xa3),
    code: Color::Rgb(0xeb, 0xcb, 0x8b),
    link: Color::Rgb(0x8f, 0xa1, 0xb3),
    accent: Color::Rgb(0x96, 0xb5, 0xb4),
    ok: Color::Rgb(0xa3, 0xbe, 0x8c),
    warn: Color::Rgb(0xeb, 0xcb, 0x8b),
    // Lightened from base16's #bf616a, which does not clear 4:1 on this
    // background. See the module note.
    danger: Color::Rgb(0xd9, 0x7a, 0x83),
    special: Color::Rgb(0xb4, 0x8e, 0xad),
    info: Color::Rgb(0x8f, 0xa1, 0xb3),
    sel_bg: Color::Rgb(0x4f, 0x5b, 0x66),
    sel_fg: Color::Rgb(0xc0, 0xc5, 0xce),
};

/// GitHub's light palette, matching `InspiredGitHub` on the syntax side.
pub static LIGHT: Theme = Theme {
    bg: Color::Rgb(0xff, 0xff, 0xff),
    fg: Color::Rgb(0x1f, 0x23, 0x28),
    dim: Color::Rgb(0x6e, 0x77, 0x81),
    code: Color::Rgb(0x9a, 0x67, 0x00),
    link: Color::Rgb(0x09, 0x69, 0xda),
    accent: Color::Rgb(0x1b, 0x7c, 0x83),
    ok: Color::Rgb(0x1a, 0x7f, 0x37),
    warn: Color::Rgb(0x9a, 0x67, 0x00),
    danger: Color::Rgb(0xcf, 0x22, 0x2e),
    special: Color::Rgb(0x82, 0x50, 0xdf),
    info: Color::Rgb(0x09, 0x69, 0xda),
    sel_bg: Color::Rgb(0xd0, 0xd7, 0xde),
    sel_fg: Color::Rgb(0x1f, 0x23, 0x28),
};

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance. The palettes are chosen against this rather
    /// than by eye, so the check belongs in the build and not in a note.
    fn luminance(c: Color) -> f64 {
        let Color::Rgb(r, g, b) = c else {
            panic!("{c:?} is not absolute — a painted page cannot use a palette colour");
        };
        let f = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn check(name: &str, t: &Theme) {
        // Body text is read for minutes at a time; chrome is glanced at and is
        // allowed to recede; an accent has to be legible but carries meaning
        // through hue as much as through contrast.
        let cases: &[(&str, Color, f64)] = &[
            ("fg", t.fg, 7.0),
            ("dim", t.dim, 3.0),
            ("code", t.code, 4.0),
            ("link", t.link, 4.0),
            ("accent", t.accent, 4.0),
            ("ok", t.ok, 4.0),
            ("warn", t.warn, 4.0),
            ("danger", t.danger, 4.0),
            ("special", t.special, 4.0),
            ("info", t.info, 4.0),
        ];
        for (role, colour, floor) in cases {
            let got = contrast(*colour, t.bg);
            assert!(
                got >= *floor,
                "{name}.{role} is {got:.2}:1 on its own background, needs {floor}:1"
            );
        }
        // The selected row repaints the background under it, so it has to be
        // checked against *that*, not against the page.
        let sel = contrast(t.sel_fg, t.sel_bg);
        assert!(sel >= 4.0, "{name} selection is {sel:.2}:1, needs 4:1");
    }

    #[test]
    fn both_palettes_are_legible_on_their_own_background() {
        check("dark", &DARK);
        check("light", &LIGHT);
    }

    #[test]
    fn every_heading_level_is_legible_and_distinct() {
        for t in [&DARK, &LIGHT] {
            let mut seen = Vec::new();
            for level in 1..=4u8 {
                let fg = t.heading(level).fg.expect("a heading names a colour");
                assert!(contrast(fg, t.bg) >= 4.0, "heading {level} is too faint");
                seen.push(fg);
            }
            // H1..H3 are the three that carry structure in a document this
            // size; H4 and below share a colour deliberately.
            assert_ne!(seen[0], seen[1]);
            assert_ne!(seen[1], seen[2]);
            assert_ne!(seen[0], seen[2]);
        }
    }

    #[test]
    fn the_two_modes_are_actually_different_and_flip_back() {
        assert_eq!(Mode::default(), Mode::Dark);
        assert_eq!(Mode::Dark.flipped(), Mode::Light);
        assert_eq!(Mode::Light.flipped().flipped(), Mode::Light);
        assert_ne!(Mode::Dark.syntax(), Mode::Light.syntax());
        assert_ne!(DARK.bg, LIGHT.bg);
    }

    /// The light page must actually be lighter than the dark one. Trivial to
    /// state and exactly the kind of thing a copy-paste between the two static
    /// blocks would invert without any other test noticing.
    #[test]
    fn light_is_lighter_than_dark() {
        assert!(luminance(LIGHT.bg) > luminance(DARK.bg));
        assert!(luminance(LIGHT.fg) < luminance(DARK.fg));
    }
}
