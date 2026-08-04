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
//! chosen by eye: body text clears 7:1, recessive chrome clears 3:1, every
//! accent clears 4:1, and anything that repaints the page as a *block* — the
//! two search highlights — clears 1.1:1 against the page it sits on, on top of
//! the 7:1 its own text owes it. That last floor is the one a future palette
//! edit is likeliest to break without noticing, because a block that reads
//! beautifully in isolation can be invisible on the paper: GitHub's own search
//! yellow `#fff8c5` is 1.08:1 on white, and a highlight nobody can find on the
//! page is not a highlight. The dark set is base16-ocean, which is also the syntax
//! theme, so highlighted code and the chrome around it agree. The one departure
//! is `danger`: base16's `#bf616a` measures 3.23:1 on its own background, which
//! is under the floor for the one colour whose entire job is to be noticed, so
//! it is lightened to `#d97a83` (4.45:1). The light set is GitHub's, to match
//! `InspiredGitHub` on the syntax side.
//!
//! ## The search highlight
//!
//! A hit repaints the cells under it, so — like the selected row in the file
//! list — both halves of the pair are named here rather than one of them being
//! inherited from whatever the document happened to be drawing there. Unlike
//! the selected row, which is held to 4:1, both highlight pairs are held to the
//! **body** floor of 7:1: a selected row is one row the reader deliberately put
//! the cursor on, while a hit is a word buried inside a paragraph they are
//! still reading around, and a hit that dropped to 4:1 would be the one word on
//! the page harder to read than its neighbours.
//!
//! Every hit gets a neutral block and the one the reader is on gets an amber
//! one. Written as the struct holds them, background first:
//!
//! - dark: `#2b303b` on `#c0c5ce` (7.63:1) — literally the page inverted, which
//!   is the one pair this palette already guarantees — and `#2b303b` on
//!   `#ebcb8b` (8.46:1), base16's own yellow, which is what `warn` and `code`
//!   already mean "look here" in.
//! - light: `#1f2328` on `#d0d7de` (10.88:1), the grey the file list already
//!   selects rows with, and `#1f2328` on `#ffb454` (8.96:1). That amber is
//!   **not** this palette's attention colour — GitHub's `warn` and `code` are
//!   `#9a6700`, which is a foreground and far too dark to be a page a reader
//!   has to read black text on. It is chosen against the two floors above and
//!   nothing else, which its own comment says where it is declared.
//!
//! The current hit is underlined as well, and that is not decoration. The two
//! blocks differ almost entirely in hue: measured against each other they are
//! **1.11:1** on the dark page and **1.21:1** on the light one, so a reader who
//! does not receive the hue difference receives no difference at all. `link`
//! carries the same rule for the same reason — colour alone is not a signal
//! every reader gets. See [`Theme::hit`], which is also where the non-current
//! block has to *remove* the underline rather than merely not add one.

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
    /// Every match of a search over the document. Neutral on purpose: a long
    /// document can have forty of these on one screen, and forty blocks of the
    /// attention colour is a page nobody can read.
    pub hit_bg: Color,
    pub hit_fg: Color,
    /// The one match the reader is on — what `n` and `N` move between.
    pub hit_now_bg: Color,
    pub hit_now_fg: Color,
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

    /// A match of the reader's search, and whether it is the one they are on.
    ///
    /// Both halves of each pair are named, because a hit repaints the cells
    /// under it and a foreground left to whatever syntect had put there would
    /// be a pairing nobody chose and nobody measured — the same argument the
    /// selected row makes in `browse.rs`. Which also means the matched word
    /// loses its own colour for as long as it is a match, and that is the
    /// point: a hit has to be findable by sweeping the page, not by reading it.
    ///
    /// The underline on the current one carries the whole distinction for a
    /// reader who does not receive hue. See the module note for the measured
    /// 1.11:1 and 1.21:1 that make it load-bearing rather than decorative.
    ///
    /// Which is why the other branch *removes* the underline instead of simply
    /// not adding one. These are applied with `Style::patch`, and `patch` takes
    /// the union of the modifiers — so an ordinary hit landing inside a link,
    /// which `markdown` underlines precisely because colour alone is not enough
    /// there either, would arrive underlined and be indistinguishable from the
    /// current one. Searching rendered markdown for a word that also appears in
    /// a link is not an exotic thing to do.
    pub fn hit(&self, current: bool) -> Style {
        if current {
            Style::new()
                .fg(self.hit_now_fg)
                .bg(self.hit_now_bg)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new()
                .fg(self.hit_fg)
                .bg(self.hit_bg)
                .remove_modifier(Modifier::UNDERLINED)
        }
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
    // The page inverted, which is `fg` on `bg` read the other way round and so
    // the one block this palette already guarantees at the body floor.
    hit_bg: Color::Rgb(0xc0, 0xc5, 0xce),
    hit_fg: Color::Rgb(0x2b, 0x30, 0x3b),
    hit_now_bg: Color::Rgb(0xeb, 0xcb, 0x8b),
    hit_now_fg: Color::Rgb(0x2b, 0x30, 0x3b),
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
    // The same grey a selected row uses. The two are never on screen together
    // — one is the list, the other is the document — and a highlight and a
    // selection are the same idea wearing different names.
    hit_bg: Color::Rgb(0xd0, 0xd7, 0xde),
    hit_fg: Color::Rgb(0x1f, 0x23, 0x28),
    // Not GitHub's `#fff8c5`: on a white page that measures 1.08:1 against the
    // paper, which is a highlight you have to hunt for. This one is 1.76:1 and
    // still 8.96:1 under the text.
    hit_now_bg: Color::Rgb(0xff, 0xb4, 0x54),
    hit_now_fg: Color::Rgb(0x1f, 0x23, 0x28),
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

        // A hit repaints its cells too, and is held a floor higher than the
        // selected row: it is a word inside a paragraph still being read, not
        // a row the reader put the cursor on. See the module note.
        for (role, pair) in [
            ("hit", (t.hit_fg, t.hit_bg)),
            ("hit_now", (t.hit_now_fg, t.hit_now_bg)),
        ] {
            let got = contrast(pair.0, pair.1);
            assert!(got >= 7.0, "{name}.{role} is {got:.2}:1, needs 7:1");
        }
        // ...and a hit has to be visible *as* a block, or there is nothing to
        // sweep the page for. The floor is set from the failure it exists to
        // exclude: GitHub's own search yellow is 1.08:1 on white, which is a
        // highlight a reader hunts for rather than sees.
        for (role, bg) in [("hit", t.hit_bg), ("hit_now", t.hit_now_bg)] {
            let got = contrast(bg, t.bg);
            assert!(got >= 1.1, "{name}.{role} is {got:.2}:1 against the page");
        }
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

    /// The two highlight blocks differ almost entirely in hue — 1.11:1 and
    /// 1.21:1 measured against each other — so without the underline a reader
    /// who does not receive hue cannot tell which hit `n` is on. That is the
    /// rule `link` already follows, and this is the assertion that keeps a
    /// tidy-up from dropping the modifier as redundant.
    #[test]
    fn the_current_hit_is_marked_by_something_other_than_its_colour() {
        for (name, t) in [("dark", &DARK), ("light", &LIGHT)] {
            // WCAG's weakest floor for anything a reader has to distinguish —
            // 3:1, what it asks of a non-text boundary — is the number this is
            // safely under. Two blocks that far apart in luminance would tell
            // themselves apart in greyscale and the underline would be
            // ornament; at these it is the whole signal.
            let blocks = contrast(t.hit_bg, t.hit_now_bg);
            assert!(
                blocks < 3.0,
                "{name}'s two blocks are {blocks:.2}:1 apart, which clears \
                 WCAG's boundary floor — the claim in the module note is now \
                 the thing that is wrong, not the code"
            );
            assert!(
                t.hit(true).add_modifier.contains(Modifier::UNDERLINED),
                "{name}'s current hit is colour and nothing else"
            );
            assert_ne!(t.hit(true).bg, t.hit(false).bg);

            // Composed, not in isolation: `Style::patch` unions modifiers, and
            // `markdown` underlines link text. Asserting `hit(false)` alone is
            // exactly what let an underlined ordinary hit through review.
            let link = Style::new().fg(t.link).add_modifier(Modifier::UNDERLINED);
            assert!(
                !link
                    .patch(t.hit(false))
                    .add_modifier
                    .contains(Modifier::UNDERLINED),
                "{name}: a hit inside a link keeps the link's underline, so it \
                 is indistinguishable from the one the reader is on"
            );
            assert!(
                link.patch(t.hit(true))
                    .add_modifier
                    .contains(Modifier::UNDERLINED)
            );
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
