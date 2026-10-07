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
//!
//! ## A line at a time, for something that is being typed into
//!
//! [`Lines`] is the same highlighter with its state handed back between lines
//! rather than kept inside it, and every whole-file function here is now that
//! one driven from the top. It exists for `crate::editor`, which highlights a
//! document somebody is changing and cannot pay for the whole of it on every
//! keystroke: at the 370 KB/s measured on [`HIGHLIGHT_MAX_BYTES`], a 46 KB
//! README is 125 ms a key. What makes resuming honest is that syntect's two
//! per-line states, `ParseState` and `HighlightState`, are the *whole* of what
//! one line hands the next — the parser keeps nothing else between calls — and
//! both derive `PartialEq` and `Eq` in syntect 5.3. So [`Resume`] can be
//! compared, and an editor that re-highlights from an edited line may stop at
//! the first line after which the state is the one it had cached: every line
//! below that is a function of a state and a text that are both unchanged.
//! [`Resume`]'s own doc has the one way that comparison can be wrong, and why
//! it is wrong in the safe direction.
//!
//! One routine rather than two because the alternative is two highlighters
//! that can disagree. The whole-file path and the per-line path differ only in
//! where the state lives between lines, so they are one function called from
//! two places, and a test pins that resuming from a cached state gives exactly
//! what highlighting from the top does.

use std::path::Path;
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

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
/// quadratic. Nothing readable is that wide anyway. Measured on the line as it
/// is fed: expanded for the reader, raw — tabs and all — for the editor; see
/// [`Lines::line`].
const MAX_LINE: usize = 4096;

/// Tab stop used when expanding source. Four, because the syntax highlighter
/// and the wrapper both need to agree on a column count and a literal tab has
/// no width they could agree on.
///
/// `pub(crate)` because `crate::editor` draws a literal tab at the same stops.
/// It keeps the tab in the text — a file is given back with the bytes it was
/// read with — and expands it only when it draws, so the two places a tab is
/// shown take their stop from one number and cannot settle on different ones.
pub(crate) const TAB: usize = 4;

/// Highlight a whole file, choosing the grammar from its path.
///
/// Deliberately not syntect's `find_syntax_for_file`, which re-opens the file
/// to sniff its first line. We already hold the text, and the pane must not do
/// I/O it can avoid on the draw path.
pub fn highlight_file(text: &str, path: &Path, mode: Mode) -> Vec<Vec<Span<'static>>> {
    highlight_with(text, Grammar::for_file(path, text).map(|g| g.0), mode)
}

/// Highlight a fenced block, choosing the grammar from the fence's info
/// string. An unknown or absent language is not an error — plenty of fences
/// are `text`, `console`, or nothing at all.
pub fn highlight_code(text: &str, lang: &str, mode: Mode) -> Vec<Vec<Span<'static>>> {
    highlight_with(text, Grammar::for_code(lang).map(|g| g.0), mode)
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
    syntax: Option<&'static SyntaxReference>,
    mode: Mode,
) -> Vec<Vec<Span<'static>>> {
    let Some(syntax) = syntax else {
        return plain(text, Style::default());
    };
    if text.len() > HIGHLIGHT_MAX_BYTES {
        return plain(text, Style::default());
    }

    let hl = Lines::new(Grammar(syntax), mode);
    let mut at = hl.start();
    lines(text)
        .map(|line| hl.line(&mut at, &expand_tabs(line)))
        .collect()
}

/// A grammar, chosen once and kept by something that highlights a line at a
/// time.
///
/// A handle on a syntax in the one `SyntaxSet` this module ever loads, and
/// compared by *identity* rather than by name: two handles are the same grammar
/// exactly when they point at the same definition, which is the question a
/// cache keyed by one is asking. A name comparison would be a second, weaker
/// account of the same fact.
#[derive(Clone, Copy)]
pub(crate) struct Grammar(&'static SyntaxReference);

impl PartialEq for Grammar {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Grammar {}

/// The grammar's name and nothing else. A derived `Debug` would print the
/// whole syntax definition — kilobytes of contexts and regexes — into the
/// message of any assertion that compares two of these.
impl std::fmt::Debug for Grammar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Grammar({:?})", self.0.name)
    }
}

impl Grammar {
    /// The grammar for a file, chosen from its path and, failing that, its
    /// first line — the lookup [`highlight_file`] makes, so a file edited and
    /// the same file read are coloured by the same grammar.
    ///
    /// Deliberately not syntect's `find_syntax_for_file`, which re-opens the
    /// file to sniff its first line. We already hold the text, and the pane
    /// must not do I/O it can avoid on the draw path.
    pub(crate) fn for_file(path: &Path, text: &str) -> Option<Self> {
        let set = &assets().syntaxes;
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(|e| set.find_syntax_by_extension(e))
            // `Makefile`, `Dockerfile`, `.gitignore`: the whole name is the token.
            .or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| set.find_syntax_by_extension(n))
            })
            .or_else(|| first_line_syntax(set, text))
            .map(Grammar)
    }

    /// The grammar a fence's info string names, by the same lookup
    /// [`highlight_code`] makes. `None` for a language syntect does not know,
    /// which a caller draws as plain text.
    pub(crate) fn for_code(lang: &str) -> Option<Self> {
        let set = &assets().syntaxes;
        let token = lang.split_whitespace().next().unwrap_or("");
        if token.is_empty() {
            return None;
        }
        set.find_syntax_by_token(token)
            .or_else(|| set.find_syntax_by_extension(token))
            .map(Grammar)
    }
}

/// Where the highlighter is between two lines: everything one line hands the
/// next, and nothing else.
///
/// Both halves are syntect's own and both derive `Eq` in 5.3, which is what a
/// caller resuming from a cached copy needs — see the module doc. The equality
/// is structural and therefore *strict*: two states that would colour every
/// line after them identically can still compare unequal, because a parser
/// level remembers the text a backreference captured whether or not anything
/// below it will read it. That costs a resuming caller some lines highlighted
/// that did not need to be, and never a line left with colours from a state it
/// is no longer in. Strict in that direction is the only direction this
/// comparison is allowed to be wrong in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resume {
    parse: ParseState,
    highlight: HighlightState,
}

/// One grammar in one palette, fed a line at a time.
///
/// The state is the caller's rather than this value's, which is the whole
/// difference from syntect's own `HighlightLines` and the reason this exists:
/// a caller holding the [`Resume`] after every line can restart from any of
/// them.
pub(crate) struct Lines {
    syntax: &'static SyntaxReference,
    highlighter: Highlighter<'static>,
}

impl Lines {
    pub(crate) fn new(grammar: Grammar, mode: Mode) -> Self {
        Self {
            syntax: grammar.0,
            highlighter: Highlighter::new(assets().theme(mode)),
        }
    }

    /// The state before the first line of a document.
    pub(crate) fn start(&self) -> Resume {
        Resume {
            parse: ParseState::new(self.syntax),
            highlight: HighlightState::new(&self.highlighter, ScopeStack::new()),
        }
    }

    /// Highlight one line, advancing `at` past it.
    ///
    /// `line` is drawn as given: the whole-file functions above hand it here
    /// with its tabs already expanded, and `crate::editor` hands it the line
    /// as it is held, tabs and all, because a span there has to count the same
    /// characters the caret does. The grammar then sees the bytes the file
    /// really has, which is also the more faithful reading — a Makefile's
    /// recipe is a line that begins with a tab, not four spaces.
    ///
    /// A line too long to be worth the regex engine's time comes back plain
    /// and does **not** advance the state, which is what the whole-file path
    /// has always done: the line after it is highlighted as though the long
    /// one were not there.
    pub(crate) fn line(&self, at: &mut Resume, line: &str) -> Vec<Span<'static>> {
        if line.len() > MAX_LINE {
            return vec![Span::raw(line.to_string())];
        }
        let set = &assets().syntaxes;
        // The dumps are the with-newlines variants, and several grammars key
        // end-of-context off the newline, so it has to be fed in and then not
        // drawn.
        let fed = format!("{line}\n");
        match at.parse.parse_line(&fed, set) {
            Ok(ops) => HighlightIterator::new(&mut at.highlight, &ops, &fed, &self.highlighter)
                .filter_map(|(style, piece)| {
                    let piece = piece.trim_end_matches('\n');
                    (!piece.is_empty()).then(|| Span::styled(piece.to_string(), convert(style)))
                })
                .collect(),
            // A grammar that fails on one line must not lose the line. The
            // parse state is left wherever the failure left it, which is what
            // `HighlightLines` did with it too.
            Err(_) => vec![Span::raw(line.to_string())],
        }
    }
}

/// Splitting on `\n` alone is safe: the loader has already normalised line
/// endings, which matters on Windows where every second file is CRLF and a
/// stray `\r` renders as a hole in the middle of the pane.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
}

/// Tabs to spaces, at [`TAB`] stops.
///
/// The reader's rule, and not the editor's: an edited file keeps its tabs,
/// hands them to the highlighter as they are, and draws each in cells from the
/// start of its row. `crate::editor`'s module doc has the difference and why
/// it is accepted.
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

    /// Markdown with the constructs whose state outlives a line: a fence that
    /// is opened and closed, an HTML block, a list, emphasis.
    const DOC: &str = "# Title\n\nSome *prose* here.\n\n```rust\n\
                       fn main() {\n    let x = 1;\n}\n```\n\n\
                       - one\n- two\n\n<div>\nraw\n</div>\n\nThe end.";

    #[test]
    fn resuming_from_a_cached_state_gives_what_highlighting_from_the_top_does() {
        // The claim `crate::editor` is built on. Every line's state is kept,
        // and highlighting is restarted from each of them in turn: the rows
        // that come back must be the rows the whole-file path produced, span
        // for span and colour for colour.
        let whole = highlight_code(DOC, "markdown", Mode::Dark);
        let hl = Lines::new(Grammar::for_code("markdown").expect("markdown"), Mode::Dark);
        let lines: Vec<&str> = DOC.split('\n').collect();

        let mut states = vec![hl.start()];
        let mut at = hl.start();
        for line in &lines {
            hl.line(&mut at, line);
            states.push(at.clone());
        }
        for (from, state) in states.iter().enumerate().take(lines.len()) {
            let mut at = state.clone();
            for (i, line) in lines.iter().enumerate().skip(from) {
                assert_eq!(
                    hl.line(&mut at, line),
                    whole[i],
                    "line {i} resumed from line {from} came back different"
                );
            }
        }
    }

    #[test]
    fn a_line_at_a_time_is_what_syntects_own_highlight_lines_gives() {
        // The whole-file path used to be `syntect::easy::HighlightLines`, and
        // is now `Lines` driven from the top. This pins that the change of
        // routine changed no colour: span for span against the type it
        // replaced, on markdown and on a grammar with a stateful block comment.
        for (text, lang) in [(DOC, "markdown"), ("/* a\n * b */\nfn x() {}\n// c", "rust")] {
            let grammar = Grammar::for_code(lang).expect("a known grammar");
            let mut theirs =
                syntect::easy::HighlightLines::new(grammar.0, assets().theme(Mode::Dark));
            let ours = highlight_code(text, lang, Mode::Dark);
            for (i, line) in text.split('\n').enumerate() {
                let fed = format!("{line}\n");
                let want: Vec<Span<'static>> = theirs
                    .highlight_line(&fed, &assets().syntaxes)
                    .expect("syntect highlights the line")
                    .into_iter()
                    .filter_map(|(style, piece)| {
                        let piece = piece.trim_end_matches('\n');
                        (!piece.is_empty())
                            .then(|| Span::styled(piece.to_string(), convert(style)))
                    })
                    .collect();
                assert_eq!(ours[i], want, "{lang} line {i} came back different");
            }
        }
    }

    #[test]
    fn a_state_is_equal_where_the_grammar_is_back_where_it_was_and_not_inside_a_fence() {
        // What the stop condition reads. Two ordinary paragraphs leave the
        // grammar in the same place; a fence opener does not, and the line
        // after its closer is back again.
        let hl = Lines::new(Grammar::for_code("markdown").expect("markdown"), Mode::Dark);
        let after = |lines: &[&str]| {
            let mut at = hl.start();
            for line in lines {
                hl.line(&mut at, line);
            }
            at
        };
        assert_eq!(after(&["hello", "world"]), after(&["hello", "there"]));
        assert_ne!(after(&["hello", "```rust"]), after(&["hello", "world"]));
        assert_eq!(
            after(&["hello", "```rust", "let x = 1;", "```"]),
            after(&["hello", "world", "again", "more"]),
            "a closed fence leaves the grammar where a paragraph does"
        );
    }

    #[test]
    fn a_grammar_is_the_same_grammar_only_when_it_is_the_same_definition() {
        let a = Grammar::for_code("markdown").expect("markdown");
        assert_eq!(a, Grammar::for_code("md").expect("by extension"));
        assert_ne!(a, Grammar::for_code("rust").expect("rust"));
        assert_eq!(Grammar::for_code(""), None);
        assert_eq!(Grammar::for_code("definitely-not-a-language"), None);

        // And by file, the reader's lookup: extension, whole name, first line.
        assert_eq!(Grammar::for_file(Path::new("notes.md"), ""), Some(a));
        assert!(Grammar::for_file(Path::new("Makefile"), "").is_some());
        assert_eq!(
            Grammar::for_file(Path::new("run"), "#!/usr/bin/env python\n"),
            Grammar::for_code("python")
        );
        assert_eq!(format!("{a:?}"), "Grammar(\"Markdown\")", "a name, not a dump");
    }
}
