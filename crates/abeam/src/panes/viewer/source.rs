//! Syntax highlighting, for whole source files and for fenced code inside
//! markdown.
//!
//! syntect's default syntax and theme dumps cost about two megabytes and a
//! noticeable fraction of a second to deserialise, so they are built once, on
//! first use, and never for a session that only ever looks at prose. The pane
//! is the only caller and it calls from the draw path, so that cost lands as a
//! single hitch on the first file with code in it rather than at startup.
//!
//! Everything here returns *unwrapped* rows — one `Vec<Span>` per source line.
//! Fitting them to the pane is `wrap`'s job, and it has to be, because the same
//! rows get re-fitted when the window is dragged.

use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use super::theme::Mode;

/// Past this, highlighting is skipped and the file is shown as plain text.
///
/// Measured rather than guessed, because the first number here was guessed and
/// was out by a factor of three: syntect manages roughly 370 KB/s on a stateful
/// grammar in a release build on the machine this was written on. This runs on
/// the draw path, so the cap is a *time* budget — 64 KiB is about 170 ms, which
/// is already at the edge of what a keystroke can be made to wait for, and
/// covers every source file anyone hand-writes.
pub const HIGHLIGHT_MAX_BYTES: usize = 64 * 1024;

/// A minified bundle or a base64 blob on one line will send fancy-regex
/// quadratic. Nothing readable is that wide anyway.
const MAX_LINE: usize = 4096;

/// Tab stop used when expanding source. Four, because the syntax highlighter
/// and the wrapper both need to agree on a column count and a literal tab has
/// no width they could agree on.
const TAB: usize = 4;

/// Highlight a whole file, choosing the grammar from its path.
///
/// Deliberately not syntect's `find_syntax_for_file`, which re-opens the file
/// to sniff its first line. We already hold the text, and the pane must not do
/// I/O it can avoid on the draw path.
pub fn highlight_file(text: &str, path: &Path, mode: Mode) -> Vec<Vec<Span<'static>>> {
    let set = &assets().syntaxes;
    let syntax = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| set.find_syntax_by_extension(e))
        // `Makefile`, `Dockerfile`, `.gitignore`: the whole name is the token.
        .or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| set.find_syntax_by_extension(n))
        })
        .or_else(|| first_line_syntax(set, text));
    highlight_with(text, syntax, mode)
}

/// Highlight a fenced block, choosing the grammar from the fence's info
/// string. An unknown or absent language is not an error — plenty of fences
/// are `text`, `console`, or nothing at all.
pub fn highlight_code(text: &str, lang: &str, mode: Mode) -> Vec<Vec<Span<'static>>> {
    let assets = assets();
    let token = lang.split_whitespace().next().unwrap_or("");
    let syntax = if token.is_empty() {
        None
    } else {
        assets
            .syntaxes
            .find_syntax_by_token(token)
            .or_else(|| assets.syntaxes.find_syntax_by_extension(token))
    };
    highlight_with(text, syntax, mode)
}

/// One unstyled span per line. The fallback for everything above, and for
/// files with no grammar.
pub fn plain(text: &str, style: Style) -> Vec<Vec<Span<'static>>> {
    lines(text)
        .map(|l| vec![Span::styled(expand_tabs(l), style)])
        .collect()
}

fn highlight_with(
    text: &str,
    syntax: Option<&SyntaxReference>,
    mode: Mode,
) -> Vec<Vec<Span<'static>>> {
    let assets = assets();
    let Some(syntax) = syntax else {
        return plain(text, Style::default());
    };
    if text.len() > HIGHLIGHT_MAX_BYTES {
        return plain(text, Style::default());
    }

    let mut hl = HighlightLines::new(syntax, assets.theme(mode));
    lines(text)
        .map(|line| {
            let expanded = expand_tabs(line);
            if expanded.len() > MAX_LINE {
                return vec![Span::raw(expanded)];
            }
            // The dumps are the with-newlines variants, and several grammars
            // key end-of-context off the newline, so it has to be fed in and
            // then not drawn.
            let fed = format!("{expanded}\n");
            match hl.highlight_line(&fed, &assets.syntaxes) {
                Ok(parts) => parts
                    .into_iter()
                    .filter_map(|(style, piece)| {
                        let piece = piece.trim_end_matches('\n');
                        (!piece.is_empty())
                            .then(|| Span::styled(piece.to_string(), convert(style)))
                    })
                    .collect(),
                // A grammar that fails on one line must not lose the line.
                Err(_) => vec![Span::raw(expanded)],
            }
        })
        .collect()
}

/// Splitting on `\n` alone is safe: the loader has already normalised line
/// endings, which matters on Windows where every second file is CRLF and a
/// stray `\r` renders as a hole in the middle of the pane.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
}

/// Tabs to spaces, at [`TAB`] stops.
///
/// Every row the viewer draws goes through this, and so must every row
/// `super::grep` draws: a `\t` written into a terminal cell is not a character
/// the cell can hold, and `unicode_width` measures it as nothing at all, so a
/// preview of a line from a Makefile would be a row whose drawn width and
/// measured width disagree. Shared rather than repeated, so the two cannot
/// settle on different tab stops and show the same line two ways.
pub(super) fn expand_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 8);
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let pad = TAB - (col % TAB);
            out.extend(std::iter::repeat_n(' ', pad));
            col += pad;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// A `#!/usr/bin/env python` with no extension is common enough in a repo to
/// be worth the one extra lookup.
fn first_line_syntax<'a>(set: &'a SyntaxSet, text: &str) -> Option<&'a SyntaxReference> {
    let first = text.split('\n').next()?;
    set.find_syntax_by_first_line(first)
}

/// Foreground only, still — but for a different reason than it used to be.
///
/// This once discarded the background to avoid painting over the terminal's.
/// The pane now paints its own page (`theme::Theme::base`), so the reason is
/// that there would be *two* backgrounds: syntect emits one per token, and a
/// per-token background paints a ragged block behind the code that stops at the
/// end of each line. The page underneath is picked to pair with the syntax
/// theme, so dropping this one costs nothing and the two agree anyway.
fn convert(style: syntect::highlighting::Style) -> Style {
    let fg = style.foreground;
    let mut out = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

struct Assets {
    syntaxes: SyntaxSet,
    dark: Theme,
    light: Theme,
}

impl Assets {
    fn theme(&self, mode: Mode) -> &Theme {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let mut themes = ThemeSet::load_defaults();
        // Both are taken here rather than one on demand, because the cost is in
        // `load_defaults` — the dump holds every theme and is deserialised
        // whole either way. Taking the second one now means F3 never pays a
        // second hitch.
        //
        // Each is mid-contrast and separates tokens by hue rather than by
        // background, which matters because we discard their backgrounds and
        // paint the pane's own instead. The fallbacks are in the same dump and
        // exist so a syntect that renames a theme degrades to a wrong-but-
        // legible one rather than to `Theme::default`, which is black on black.
        let mut take = |first: &str, second: &str| {
            themes
                .themes
                .remove(first)
                .or_else(|| themes.themes.remove(second))
                .unwrap_or_default()
        };
        Assets {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            dark: take(Mode::Dark.syntax(), "base16-eighties.dark"),
            light: take(Mode::Light.syntax(), "Solarized (light)"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(rows: &[Vec<Span<'_>>]) -> Vec<String> {
        rows.iter()
            .map(|r| r.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn every_source_line_becomes_exactly_one_row() {
        let rows = highlight_code("fn a() {}\nfn b() {}\n", "rust", Mode::Dark);
        // Trailing newline yields a trailing empty row, same as an editor.
        assert_eq!(flat(&rows), ["fn a() {}", "fn b() {}", ""]);
    }

    #[test]
    fn a_known_language_actually_gets_colour() {
        let rows = highlight_code("let x = 1;", "rust", Mode::Dark);
        assert!(rows[0].iter().any(|s| s.style.fg.is_some()));
    }

    /// The reason the light palette exists at all. A dark syntax theme is
    /// mid-tone pastels chosen for a `#2b303b` background; on a white page they
    /// are washed out rather than merely wrong, which is the state a reader in
    /// a bright room was trying to escape.
    #[test]
    fn the_two_modes_highlight_the_same_code_in_different_colours() {
        let dark = highlight_code("let x = 1;", "rust", Mode::Dark);
        let light = highlight_code("let x = 1;", "rust", Mode::Light);
        assert_eq!(flat(&dark), flat(&light), "the text itself must not change");

        let colours =
            |rows: &[Vec<Span<'_>>]| rows.iter().flatten().map(|s| s.style.fg).collect::<Vec<_>>();
        assert_ne!(colours(&dark), colours(&light));
        // Both are real themes rather than one of them being `Theme::default`,
        // which is what a renamed theme in a future syntect would silently
        // leave behind — and which highlights everything black on black.
        assert!(colours(&dark).iter().any(Option::is_some));
        assert!(colours(&light).iter().any(Option::is_some));
    }

    #[test]
    fn an_unknown_language_degrades_to_plain_text_rather_than_failing() {
        let rows = highlight_code(
            "!!! not a language !!!",
            "definitely-not-a-language",
            Mode::Dark,
        );
        assert_eq!(flat(&rows), ["!!! not a language !!!"]);
    }

    #[test]
    fn tabs_expand_to_stops_so_columns_can_be_counted() {
        let rows = plain("a\tb\n\tc", Style::default());
        assert_eq!(flat(&rows), ["a   b", "    c"]);
    }

    #[test]
    fn a_file_too_big_to_highlight_is_still_shown() {
        let big = "let x = 1;\n".repeat(HIGHLIGHT_MAX_BYTES / 11 + 10);
        let rows = highlight_code(&big, "rust", Mode::Dark);
        assert!(rows.len() > HIGHLIGHT_MAX_BYTES / 11);
        assert_eq!(rows[0][0].content.as_ref(), "let x = 1;");
    }

    #[test]
    fn extension_lookup_finds_a_grammar_for_a_real_path() {
        let rows = highlight_file("fn main() {}", Path::new("src/main.rs"), Mode::Dark);
        assert!(rows[0].iter().any(|s| s.style.fg.is_some()));
    }
}
