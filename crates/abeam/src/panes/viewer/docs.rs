//! Where a source file stops being code and starts being prose.
//!
//! [`regions`] cuts a file into runs of lines that are code and runs of lines
//! that are documentation — `///` and `//!` in Rust, a docstring in Python —
//! so that `super::source_lines` can hand the second kind to
//! `markdown::render` and leave the first kind to syntect. The argument for
//! doing that at all belongs to the pane; what this module owes it is two
//! properties, and neither is negotiable.
//!
//! ## It is total
//!
//! What comes back is a *partition*: every line of the file is in exactly one
//! region, the regions are in order, and they do not overlap. That is not a
//! nicety, it is the only reason the caller can walk this list instead of
//! reconciling it — a scanner that returned "here are the interesting bits"
//! would make losing a line of somebody's file the default outcome of every
//! bug in it. [`partition`] is where the property is manufactured, out of a
//! list of doc blocks that knows nothing about it, and it is the one function
//! here that cannot be wrong quietly.
//!
//! ## It is cheap
//!
//! A line scanner, deliberately not a parser. One forward pass, no
//! backtracking, no regex engine, and no attempt to know what is inside a
//! string literal. The alternative — `syn` for Rust, and nothing at all for
//! Python without shipping a second grammar — buys exactness in return for a
//! parse of the whole file on the path that opens a document, next to a
//! syntect pass that already costs about 170 ms at its cap. Prose that is
//! occasionally in the wrong place is a far smaller failure than a pane that
//! takes a second to open a file, especially when the reader can press `t` and
//! see the truth.
//!
//! The result is cached on the document it was scanned from, beside the body,
//! and dies with it — so this runs once per file opened rather than once per
//! frame. That matters more than it looks: dragging the window re-lays the
//! document out on *every* frame, and re-scanning there would mean an
//! allocation per doc block per frame for the whole drag.
//!
//! ## What it gets wrong, and why that is allowed
//!
//! The Rust half is exact enough to be boring. `///`, `//!`, `/**` and `/*!`
//! are the whole of it, they must be the first thing on their line, and the
//! only way to fool them is to open a raw string whose lines happen to start
//! with a doc marker.
//!
//! The Python half is a **heuristic** and will occasionally call a
//! triple-quoted data string a docstring — see [`python`] for the exact shape
//! of the mistake. That is acceptable for one reason and it is a reason the
//! reader can act on: `t` swaps the whole document back to its source, and the
//! title says which form is up. A wrong guess costs a passage rendered as
//! prose and one keystroke; there is no state to lose and nothing is hidden,
//! because a doc region is always a whole number of lines and the source is
//! always one key away. A guess that could *hide* code would not be allowed,
//! which is why every rule below refuses a block whose closing line has code
//! after it.
//!
//! ## Prose normalisation, which is the wart
//!
//! Docstrings are not markdown and the commonest ones are barely prose. A
//! Google-style block indents four spaces under `Args:`, and CommonMark reads
//! four spaces as an *indented code block* — so the single most common shape
//! in Python would render as a grey slab of monospace. [`normalise`] pulls any
//! indent of four or more down to two, below the threshold, and [`dedent`]
//! takes the block's own Python indentation off first, because a docstring
//! inside a method starts four or eight columns in and that indent belongs to
//! the language rather than to the author.
//!
//! numpydoc needs nothing: `Parameters` over a row of `-` is a setext H2 and
//! renders as a heading for free.
//!
//! One rule for both languages rather than one per language. It costs Rust
//! something real — an *indented* code sample in a doc comment flattens into
//! the paragraph above it — and that is accepted because the fenced form is
//! what rustdoc has encouraged for years, fences are detected and exempt the
//! whole block, and a reader can learn "indentation inside documentation is
//! reduced" where they could never learn "…except in Rust".

use std::borrow::Cow;
use std::path::Path;

use super::source::expand_tabs;

/// What a run of lines is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Code,
    /// Documentation, with its markers already off and its indentation already
    /// sorted out: `text` is ready to hand straight to `markdown::render`.
    Doc {
        text: String,
        /// The column the block sits at in the file, so the rendered rows can
        /// be pushed across to line up with the code they describe. Measured
        /// after tabs are expanded, because that is the unit the pane draws in
        /// and `source::expand_tabs` has already done it to the code rows.
        indent: usize,
    },
}

/// A run of lines, `end` exclusive, indexed exactly as `text.split('\n')`
/// indexes them — which is also how `source::highlight_file` indexes the rows
/// it returns, and the two have to agree or the numbers in the gutter are a
/// lie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

impl Region {
    pub fn code(start: usize, end: usize) -> Self {
        Region {
            start,
            end,
            kind: Kind::Code,
        }
    }

    pub fn is_doc(&self) -> bool {
        matches!(self.kind, Kind::Doc { .. })
    }
}

/// Cut `text` into code and documentation, choosing the rules from the path's
/// extension.
///
/// Chosen by extension and by nothing else, which is `source::highlight_file`'s
/// rule one module along rather than a new one. That function has a second and
/// a third lookup — the whole file name for a `Makefile`, then the shebang —
/// and neither earns its place here: no extensionless file is a Rust file, and
/// a `#!/usr/bin/env python` script with no extension gets highlighted as
/// Python and shown without its docstrings rendered, which is exactly what it
/// did yesterday. Guessing wrong in this direction costs a feature; guessing
/// wrong the other way rewrites somebody's shell script as prose.
///
/// Never returns an empty vector. A file with no documentation in it is one
/// `Kind::Code` region over the lot, and an empty file is a single empty
/// region — so the caller never has to ask whether the list means "no
/// documentation" or "nothing scanned".
pub fn regions(text: &str, path: &Path) -> Vec<Region> {
    let Some(lang) = language(path) else {
        // Answered from a count rather than a line index, because this is the
        // common case — every `.md`, `.json`, `.toml` and `.txt` the pane is
        // ever pointed at — and building a `Vec` of half a megabyte's worth of
        // line slices to hand straight back one region would be the whole cost
        // of the feature charged to the files that do not have it.
        return vec![Region::code(0, text.split('\n').count())];
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let docs = match lang {
        Lang::Rust => rust(&lines),
        Lang::Python => python(&lines),
    };
    partition(docs, lines.len())
}

/// Fill the gaps between the doc blocks with code.
///
/// The whole of the partition invariant lives here, on purpose: [`rust`] and
/// [`python`] hunt for doc blocks and are allowed to think about nothing else,
/// and one function — with one test pinned to a real file — decides whether a
/// line can go missing. Written as "walk a cursor and close the gap" rather
/// than as an assertion, because an assertion on the draw path is a panic in
/// the agent's pane and this is a viewer.
fn partition(docs: Vec<Region>, lines: usize) -> Vec<Region> {
    let mut out = Vec::with_capacity(docs.len() * 2 + 1);
    let mut next = 0;
    for doc in docs {
        if doc.start > next {
            out.push(Region::code(next, doc.start));
        }
        next = doc.end;
        out.push(doc);
    }
    if next < lines || out.is_empty() {
        out.push(Region::code(next, lines));
    }
    out
}

enum Lang {
    Rust,
    Python,
}

fn language(path: &Path) -> Option<Lang> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("rs") {
        return Some(Lang::Rust);
    }
    if ext.eq_ignore_ascii_case("py") || ext.eq_ignore_ascii_case("pyi") {
        return Some(Lang::Python);
    }
    None
}

// --- Rust ----------------------------------------------------------------

/// `///` and `//!`, the outer and inner line forms.
///
/// Kept apart rather than lumped together as "a doc line", because a `//!` is
/// about the *enclosing* item and a `///` is about the next one. Two of them
/// touching is two different things said about two different subjects, and
/// running them into one markdown block would join the last sentence of a
/// module's introduction to the first sentence of a function's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    Outer,
    Inner,
}

/// Rust, where no guessing is needed.
///
/// A run of line-doc comments at the same indent and of the same marker is one
/// block; a different indent, a different marker, a blank line or any code at
/// all ends it. The indent is part of the identity because it is what the
/// rendered prose is pushed across by, and two blocks at different indents
/// belong to two different items.
fn rust(lines: &[&str]) -> Vec<Region> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = cols(lines[i]);
        let indent = indent_of(&line);
        let rest = &line[indent..];

        if let Some(marker) = line_marker(rest) {
            let start = i;
            let mut body = vec![strip_line(rest)];
            i += 1;
            while i < lines.len() {
                let next = cols(lines[i]);
                let ind = indent_of(&next);
                if ind != indent {
                    break;
                }
                match line_marker(&next[ind..]) {
                    Some(m) if m == marker => body.push(strip_line(&next[ind..])),
                    _ => break,
                }
                i += 1;
            }
            out.push(doc(start, i, indent, body, false));
            continue;
        }

        if is_block_open(rest)
            && let Some((end, body)) = rust_block(lines, i, indent)
        {
            out.push(doc(i, end, indent, body, false));
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// `///` or `//!` at the start of what is left of the line.
///
/// `////` is excluded because rustc excludes it: four slashes is the section
/// divider people rule a file with, and rendering a row of them as a markdown
/// paragraph would turn the one comment shape that is *deliberately* not prose
/// into prose.
fn line_marker(rest: &str) -> Option<Marker> {
    if rest.starts_with("////") {
        return None;
    }
    if rest.starts_with("///") {
        return Some(Marker::Outer);
    }
    if rest.starts_with("//!") {
        return Some(Marker::Inner);
    }
    None
}

/// Take the marker off, and one space after it if there is one — the space is
/// the convention rather than the content, and leaving it on would give every
/// line an indent that [`dedent`] would then have to take off again.
fn strip_line(rest: &str) -> String {
    let body = &rest[3..];
    body.strip_prefix(' ').unwrap_or(body).to_string()
}

/// `/**` or `/*!`, and not `/**/`, which is an empty ordinary comment whose
/// closing delimiter overlaps its opening one — the one case where searching
/// forward for `*/` would run past the end of the comment and swallow the code
/// after it.
fn is_block_open(rest: &str) -> bool {
    (rest.starts_with("/**") || rest.starts_with("/*!")) && !rest.starts_with("/**/")
}

/// A `/** … */` block, or `None` if it is not one this may safely take.
///
/// Two ways to be refused, and both protect the same thing: a region is a whole
/// number of lines, so anything that would leave half a line of code inside one
/// has to be declined rather than trimmed.
///
/// - **Code after the closing `*/`.** `/** doc */ let x = 1;` is a real line of
///   Rust and rendering it as prose would delete the statement from the page.
/// - **No closing `*/` at all.** Either the file was truncated at
///   `load::MAX_BYTES` mid-comment, or something upstream guessed wrong; in
///   both cases swallowing the entire rest of the file as prose is the worst
///   available answer, and showing the source is the honest one.
///
/// Nested block comments — legal in Rust, essentially unheard of inside a doc
/// comment — close at the first `*/` like everybody else's scanner.
fn rust_block(lines: &[&str], at: usize, indent: usize) -> Option<(usize, Vec<String>)> {
    let open = cols(lines[at]);
    let after = &open[indent + 3..];

    if let Some(p) = after.find("*/") {
        // `trim_end`, here and on every closing line below, because the space in
        // ` */` is the delimiter's padding and not a word the author wrote —
        // and two trailing spaces are a hard line break in CommonMark.
        return tail_is_clear(&after[p + 2..], "//")
            .then(|| (at + 1, vec![after[..p].trim_end().to_string()]));
    }

    let mut body = vec![after.to_string()];
    let mut i = at + 1;
    while i < lines.len() {
        let line = cols(lines[i]);
        match line.find("*/") {
            Some(p) => {
                if !tail_is_clear(&line[p + 2..], "//") {
                    return None;
                }
                body.push(line[..p].trim_end().to_string());
                return Some((i + 1, undecorate(body)));
            }
            None => body.push(line.into_owned()),
        }
        i += 1;
    }
    None
}

/// Strip the leading `*` column off a block comment, the way rustdoc does —
/// but only when *every* line that could have one does, because a single line
/// starting with `*` in a comment that is not drawn that way is a dereference
/// or a footnote, not decoration.
fn undecorate(mut body: Vec<String>) -> Vec<String> {
    let decorated = |l: &String| {
        let t = l.trim_start_matches(' ');
        t.is_empty() || (t.starts_with('*') && !t.starts_with("*/"))
    };
    if !body.iter().skip(1).all(decorated) {
        return body;
    }
    for line in body.iter_mut().skip(1) {
        let Some(star) = line.find('*') else {
            continue;
        };
        let rest = &line[star + 1..];
        *line = rest.strip_prefix(' ').unwrap_or(rest).to_string();
    }
    body
}

// --- Python --------------------------------------------------------------

/// What the scanner is currently expecting a docstring in.
#[derive(Clone, Copy)]
enum Expect {
    /// The first statement of the file, which has to be at column zero.
    Module,
    /// The first statement of a `def` or `class` body, which has to be
    /// indented further than the header that opened it. That last clause is
    /// what makes the heuristic survive contact with template strings — see
    /// [`python`].
    Body { indent: usize },
}

/// Python, where guessing is unavoidable.
///
/// A docstring is a triple-quoted string that is the first *statement* of the
/// file or of a `def`/`class` body. Comments and blank lines are not
/// statements, so they are stepped over rather than treated as the first one,
/// which is what makes the shebang-plus-licence-comment header of a script work
/// out to a module docstring.
///
/// A header can span physical lines — `def f(\n    a,\n):` is one logical line
/// in three — so this tracks "a header is open, and it is over at the colon"
/// rather than looking one line back. The colon is found past any trailing
/// comment, because `def f(x):  # noqa` is normal.
///
/// ## The mistake it makes
///
/// Nothing here knows what is inside a string, so a *data* string containing
/// Python at the start of a line can arm the scanner:
///
/// ```text
/// SNIPPET = '''
/// class Example:
///     """Not a docstring — this is inside a string constant."""
/// '''
/// ```
///
/// `class Example:` reads as a header and the line under it reads as its
/// docstring. That is the whole envelope of the false positive: a code template
/// or a doctest fixture, rendered as prose instead of as code, in a file where
/// `t` shows the truth. It is bounded by one guard that costs nothing — the
/// docstring has to be indented *further* than its header — which throws out the
/// far commoner shape where the fake header and the quotes sit at the same
/// column.
fn python(lines: &[&str]) -> Vec<Region> {
    let mut out = Vec::new();
    let mut expect = Some(Expect::Module);
    // `Some(indent)` while a `def`/`class` header is open and its colon has not
    // been seen yet.
    let mut header: Option<usize> = None;
    let mut i = 0;

    while i < lines.len() {
        let line = cols(lines[i]);
        let indent = indent_of(&line);
        let rest = &line[indent..];
        let code = code_part(rest).trim_end();
        if code.is_empty() {
            i += 1;
            continue;
        }

        if let Some(want) = expect.take() {
            let allowed = match want {
                Expect::Module => indent == 0,
                Expect::Body { indent: header } => indent > header,
            };
            if allowed && let Some((end, body)) = docstring(lines, i, rest) {
                out.push(doc(i, end, indent, body, true));
                i = end;
                continue;
            }
        }

        // A new header always wins over one still waiting for its colon. A
        // header left open by a colon that never arrived — one inside a string,
        // say — would otherwise swallow every `def` after it for the rest of the
        // file, and missing every docstring in a module is a worse failure than
        // missing one.
        if is_header(code) {
            header = Some(indent);
        }
        if let Some(at) = header
            && code.ends_with(':')
        {
            expect = Some(Expect::Body { indent: at });
            header = None;
        }
        i += 1;
    }
    out
}

fn is_header(code: &str) -> bool {
    let code = code
        .strip_prefix("async ")
        .map(str::trim_start)
        .unwrap_or(code);
    ["def", "class"]
        .iter()
        .any(|kw| code.strip_prefix(kw).is_some_and(starts_blank))
}

fn starts_blank(rest: &str) -> bool {
    rest.starts_with(|c: char| c.is_whitespace())
}

/// The part of a line before its comment, so that `def f(x):  # noqa` still
/// ends in the colon that closes the header.
///
/// Quote-aware only far enough to keep a `#` that is inside a string out of it,
/// which is the case that actually occurs (`sep = "#"`). It knows nothing about
/// triple quotes and does not need to: everything it is asked about is a line
/// the caller has already decided is code.
fn code_part(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == q {
                    quote = None;
                }
            }
            None => {
                if b == b'#' {
                    return &line[..i];
                }
                if b == b'"' || b == b'\'' {
                    quote = Some(b);
                }
            }
        }
        i += 1;
    }
    line
}

/// `"""` or `'''`, optionally behind a string prefix, and the bytes it takes up.
///
/// `r`, `b`, `u` and `f` in either case and in either order, which is every
/// prefix Python has. An f-string is not a docstring as far as `__doc__` is
/// concerned; it is prose as far as a reader is concerned, and this pane is on
/// the reader's side of that.
fn opener(rest: &str) -> Option<(&'static str, usize)> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < 2 && i < bytes.len() && matches!(bytes[i] | 0x20, b'r' | b'f' | b'b' | b'u') {
        i += 1;
    }
    for delim in ["\"\"\"", "'''"] {
        if rest[i..].starts_with(delim) {
            return Some((delim, i + 3));
        }
    }
    None
}

/// The lines of one docstring, or `None` if this is not one that may be taken.
///
/// Refused for the same two reasons [`rust_block`] refuses a block comment, and
/// the second one matters more here: an opener this guessed wrong about and
/// then never found a closer for would turn the entire rest of the file into
/// prose. Declining leaves it as code, which is what it looked like anyway.
fn docstring(lines: &[&str], at: usize, rest: &str) -> Option<(usize, Vec<String>)> {
    let (delim, open) = opener(rest)?;
    let after = &rest[open..];

    if let Some(p) = after.find(delim) {
        return tail_is_clear(&after[p + 3..], "#")
            .then(|| (at + 1, vec![after[..p].trim_end().to_string()]));
    }

    let mut body = vec![after.to_string()];
    let mut i = at + 1;
    while i < lines.len() {
        let line = cols(lines[i]);
        match line.find(delim) {
            Some(p) => {
                if !tail_is_clear(&line[p + 3..], "#") {
                    return None;
                }
                body.push(line[..p].trim_end().to_string());
                return Some((i + 1, body));
            }
            None => body.push(line.into_owned()),
        }
        i += 1;
    }
    None
}

// --- shared ---------------------------------------------------------------

/// Is there nothing but whitespace or a comment left on the closing line?
///
/// The one rule that keeps this scanner from ever hiding code: a region is a
/// whole number of lines, so a closing delimiter with a statement after it
/// means the block is declined outright rather than cut short.
///
/// The comment marker is the caller's, and passing it is not fussiness. `#` is
/// a comment in Python and an *attribute* in Rust, so a shared "or a comment"
/// rule would read `/** doc */ #[derive(Debug)]` as a clear tail and delete the
/// derive from the page — which is precisely the failure the rule exists to
/// prevent.
fn tail_is_clear(tail: &str, comment: &str) -> bool {
    let tail = tail.trim();
    tail.is_empty() || tail.starts_with(comment)
}

/// Turn a block's stripped lines into a region.
///
/// `first_is_special` is Python's: the first line of a docstring sits on the
/// same physical line as its opening quotes, so it has no indentation to share
/// with the rest and PEP 257 leaves it out of the common-indent calculation.
/// Rust's line comments all start at the same column, so it does not apply.
fn doc(
    start: usize,
    end: usize,
    indent: usize,
    mut body: Vec<String>,
    first_is_special: bool,
) -> Region {
    dedent(&mut body, first_is_special);
    normalise(&mut body);
    // A docstring's closing `"""` sits on its own line and leaves an empty one
    // behind; a `///` block often opens or closes with a bare marker. Neither is
    // a blank line the author wrote, and markdown would turn a run of them into
    // nothing anyway — but dropping them here keeps the *first* rendered row
    // lined up with the line number the gutter puts beside it.
    while body.first().is_some_and(|l| l.is_empty()) {
        body.remove(0);
    }
    while body.last().is_some_and(|l| l.is_empty()) {
        body.pop();
    }
    Region {
        start,
        end,
        kind: Kind::Doc {
            text: body.join("\n"),
            indent,
        },
    }
}

/// Take the block's own indentation off before the markdown parser sees it.
///
/// A docstring in a method starts four or eight columns in and every line of it
/// carries that; hand it to CommonMark unchanged and the whole block is an
/// indented code block before the first word is read. What is left afterwards
/// is the indentation the *author* chose, which is the only kind markdown
/// should be given a say over.
fn dedent(body: &mut [String], first_is_special: bool) {
    let from = usize::from(first_is_special);
    let common = body
        .iter()
        .skip(from)
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    if first_is_special && let Some(first) = body.first_mut() {
        *first = first.trim().to_string();
    }
    for line in body.iter_mut().skip(from) {
        if line.trim().is_empty() {
            line.clear();
        } else {
            line.drain(..common);
        }
    }
}

/// Pull any indent of four or more columns down to two.
///
/// Four spaces is CommonMark's indented-code threshold, and a Google-style
/// `Args:` block is four spaces under a label — so the commonest docstring in
/// Python renders as a grey monospace slab unless something moves it. Two is
/// under the threshold and still reads as a nested thing.
///
/// Flat two, not scaled, and that costs something worth naming: a list nested
/// three deep comes out two deep. Scaling would keep the nesting and put the
/// deeper levels back over four columns, which is the failure this exists to
/// prevent — and nesting past one level inside a docstring is rare where a
/// four-space `Args:` block is close to universal.
///
/// Skipped entirely for a block containing a fence, where indentation inside
/// the fence is the sample's own and rewriting it would be rewriting somebody's
/// code. numpydoc needs nothing either way: `Parameters` over a row of `-` is a
/// setext H2, which is a heading already.
fn normalise(body: &mut [String]) {
    let fenced = body.iter().any(|l| {
        let t = l.trim_start_matches(' ');
        t.starts_with("```") || t.starts_with("~~~")
    });
    if fenced {
        return;
    }
    for line in body.iter_mut() {
        let spaces = indent_of(line);
        if spaces >= 4 {
            line.replace_range(..spaces, "  ");
        }
    }
}

/// The line as the pane will draw it. Tabs are expanded through
/// `source::expand_tabs` — the same function the code rows go through — so that
/// an indent measured here is the same number of columns the highlighter put
/// the code at. Borrowed for the overwhelming majority of lines, which have no
/// tab in them at all.
fn cols(line: &str) -> Cow<'_, str> {
    if line.contains('\t') {
        Cow::Owned(expand_tabs(line))
    } else {
        Cow::Borrowed(line)
    }
}

/// Leading spaces, which after [`cols`] is the same as leading columns.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This very crate's largest source file, as a corpus. Included at compile
    /// time rather than read, because a test that opens a file by relative path
    /// is a test that fails under a different working directory — and because
    /// the point of the corpus is that it is real prose somebody wrote, not that
    /// it is on disk.
    const VIEWER: &str = include_str!("../viewer.rs");

    /// `load::normalise`'s job, done here for the same reason it exists there:
    /// this working tree checks out CRLF and every line would otherwise end in
    /// a `\r` that no scanner in the pane ever sees.
    fn lf(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    fn scan(text: &str, name: &str) -> Vec<Region> {
        regions(text, Path::new(name))
    }

    fn docs_of(regions: &[Region]) -> Vec<&str> {
        regions
            .iter()
            .filter_map(|r| match &r.kind {
                Kind::Doc { text, .. } => Some(text.as_str()),
                Kind::Code => None,
            })
            .collect()
    }

    #[test]
    fn the_regions_put_every_line_of_a_real_file_back_exactly_as_it_came() {
        // The invariant the pane leans its whole weight on. `source_lines` walks
        // this list instead of reconciling it, so a gap here is a line of
        // somebody's file that is simply not drawn, and a region that overlaps
        // its neighbour is a line drawn twice — both of them silently, and both
        // of them on a file the reader cannot check against anything.
        for (text, name) in [
            (lf(VIEWER), "viewer.rs"),
            (lf(VIEWER), "viewer.py"),
            (lf(VIEWER), "viewer.txt"),
            (String::new(), "empty.rs"),
            ("\n\n\n".to_string(), "blank.py"),
            ("no trailing newline".to_string(), "x.rs"),
        ] {
            let lines: Vec<&str> = text.split('\n').collect();
            let regions = scan(&text, name);

            let mut next = 0;
            let mut back = Vec::new();
            for r in &regions {
                assert_eq!(r.start, next, "{name}: a gap or an overlap at {}", r.start);
                assert!(r.end > r.start || lines.is_empty(), "{name}: empty region");
                back.extend_from_slice(&lines[r.start..r.end]);
                next = r.end;
            }
            assert_eq!(
                next,
                lines.len(),
                "{name}: the tail of the file went missing"
            );
            assert_eq!(back.join("\n"), text, "{name}: the file did not come back");
        }
    }

    #[test]
    fn a_file_with_nothing_to_render_is_one_code_region_over_the_lot() {
        // Not an empty vector, and not a special case the caller has to know
        // about: every non-Rust, non-Python file takes this path, and so does a
        // Rust file that happens to have no doc comments in it.
        for name in ["a.rs", "a.py", "a.txt", "Makefile", "a"] {
            let got = scan("fn main() {}\n", name);
            assert_eq!(got, vec![Region::code(0, 2)], "{name}");
        }
        assert_eq!(scan("", "a.rs"), vec![Region::code(0, 1)]);
    }

    // --- Rust -------------------------------------------------------------

    #[test]
    fn a_run_of_slash_slash_slash_lines_is_one_block_with_its_markers_off() {
        let got = scan("/// One.\n///\n/// Two.\nfn f() {}\n", "a.rs");
        assert_eq!(docs_of(&got), ["One.\n\nTwo."]);
        assert_eq!(got[0].start, 0);
        assert_eq!(got[0].end, 3);
        assert_eq!(got[1], Region::code(3, 5));
    }

    #[test]
    fn a_blank_line_a_new_indent_and_a_change_of_marker_all_end_a_block() {
        // Three separations rather than one, and the indent is the interesting
        // one: it is what the rendered prose is pushed across by, so two blocks
        // at two indents describe two items and must not be joined.
        let got = scan("//! Module.\n/// Item.\n\n    /// Inner.\n", "a.rs");
        assert_eq!(docs_of(&got), ["Module.", "Item.", "Inner."]);
        let indents: Vec<_> = got
            .iter()
            .filter_map(|r| match &r.kind {
                Kind::Doc { indent, .. } => Some(*indent),
                Kind::Code => None,
            })
            .collect();
        assert_eq!(indents, [0, 0, 4]);
    }

    #[test]
    fn four_slashes_are_a_divider_and_not_prose() {
        // The one comment shape that is deliberately not a sentence. rustc
        // agrees, which is why this is exactness rather than taste.
        assert!(docs_of(&scan("//// ------\nfn f() {}\n", "a.rs")).is_empty());
    }

    #[test]
    fn a_star_block_loses_its_opener_its_closer_and_its_decoration() {
        let got = scan("/**\n * One.\n *\n * Two.\n */\nfn f() {}\n", "a.rs");
        assert_eq!(docs_of(&got), ["One.\n\nTwo."]);
        assert_eq!(got[0].end, 5);
        // And on one line, which is how a short one is actually written.
        assert_eq!(docs_of(&scan("/*! Inner. */\n", "a.rs")), ["Inner."]);
    }

    #[test]
    fn a_block_comment_with_code_after_it_or_no_end_at_all_stays_code() {
        // A region is a whole number of lines, so a block that would leave half
        // a line of Rust inside one is declined rather than trimmed. The
        // unterminated case is the same rule pointed at the file that
        // `load::MAX_BYTES` cut in half.
        assert!(docs_of(&scan("/** doc */ let x = 1;\n", "a.rs")).is_empty());
        assert!(docs_of(&scan("/**\n * doc\nfn f() {}\n", "a.rs")).is_empty());
        // `/**/` closes where it opens: searching past it would swallow the file.
        assert!(docs_of(&scan("/**/\nfn f() {}\n/* x */\n", "a.rs")).is_empty());
    }

    #[test]
    fn this_files_own_neighbours_scan_the_way_a_reader_would_read_them() {
        // The corpus with the most narrative doc comments in the repository, so
        // this is the closest thing to "does it work on real input" that a unit
        // test can be.
        let text = lf(VIEWER);
        let got = scan(&text, "viewer.rs");
        let docs = docs_of(&got);
        assert!(
            docs[0].starts_with("The file / markdown view."),
            "the module doc is the first block: {:?}",
            &docs[0][..40.min(docs[0].len())]
        );
        assert!(docs.len() > 50, "{} blocks found", docs.len());
        assert!(
            docs.iter().all(|d| !d.starts_with("//")),
            "a marker survived the strip"
        );
    }

    // --- Python -----------------------------------------------------------

    #[test]
    fn a_module_docstring_is_found_past_a_shebang_and_a_licence_header() {
        // Comments and blanks are not statements, so the string is still the
        // first one — which is what makes the standard head of a script work.
        let src = "#!/usr/bin/env python\n# -*- coding: utf-8 -*-\n\n\"\"\"What this does.\"\"\"\nimport os\n";
        let got = scan(src, "a.py");
        assert_eq!(docs_of(&got), ["What this does."]);
        assert_eq!(got[0], Region::code(0, 3));
        assert_eq!(got[1].start, 3);
        assert_eq!(got[1].end, 4);
    }

    #[test]
    fn a_method_docstring_is_dedented_to_its_own_left_edge() {
        // Eight columns of Python indentation is not eight columns of markdown
        // quotation, and CommonMark cannot tell the difference — so it comes off
        // before the parser is allowed an opinion.
        let src = "class A:\n    def f(self):\n        \"\"\"Summary.\n\n        Detail.\n        \"\"\"\n        return 1\n";
        let got = scan(src, "a.py");
        assert_eq!(docs_of(&got), ["Summary.\n\nDetail."]);
        let Kind::Doc { indent, .. } = &got[1].kind else {
            panic!("the docstring is a doc region");
        };
        assert_eq!(*indent, 8, "and it is drawn back at the column it sits at");
        assert_eq!(got[2], Region::code(6, 8));
    }

    #[test]
    fn a_one_line_docstring_is_one_region_and_keeps_its_line_number() {
        let src = "def f():\n    r'''One line.'''\n    return 1\n";
        let got = scan(src, "a.py");
        assert_eq!(docs_of(&got), ["One line."]);
        assert_eq!(got[1].start, 1);
        assert_eq!(got[1].end, 2, "the closing quotes are on the same line");
    }

    #[test]
    fn a_header_split_over_several_lines_still_has_its_docstring_found() {
        // A `def` is a logical line and can be three physical ones, so the
        // scanner waits for the colon rather than looking one line back. The
        // trailing comment is there because `# noqa` after the colon is normal.
        let src = "def f(\n    a,\n    b,\n):  # noqa\n    \"\"\"Doc.\"\"\"\n";
        assert_eq!(docs_of(&scan(src, "a.py")), ["Doc."]);
    }

    #[test]
    fn a_google_style_block_is_pulled_off_the_indented_code_threshold() {
        // The wart, and the reason this normalisation exists at all: four spaces
        // under `Args:` is CommonMark's indented-code marker, so the commonest
        // docstring shape in Python would render as a grey monospace slab.
        let src = "def f(x):\n    \"\"\"Do it.\n\n    Args:\n        x: the thing.\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(src, "a.py")),
            ["Do it.\n\nArgs:\n  x: the thing."]
        );
    }

    #[test]
    fn a_docstring_with_a_fence_in_it_keeps_every_column_of_its_sample() {
        // The exemption, and it is not symmetry for its own sake: inside a fence
        // the indentation is the *code's*, and reducing it would be rewriting
        // somebody's example rather than un-indenting somebody's prose.
        let src = "def f():\n    \"\"\"Use it.\n\n    ```python\n    if x:\n        f()\n    ```\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(src, "a.py")),
            ["Use it.\n\n```python\nif x:\n    f()\n```"]
        );
    }

    #[test]
    fn a_string_that_is_not_the_first_statement_is_not_a_docstring() {
        let src = "import os\n\n\"\"\"Just a string expression.\"\"\"\n";
        assert!(docs_of(&scan(src, "a.py")).is_empty());
        let body = "def f():\n    x = 1\n    \"\"\"Not a docstring either.\"\"\"\n";
        assert!(docs_of(&scan(body, "a.py")).is_empty());
    }

    #[test]
    fn the_false_positive_this_heuristic_is_allowed_to_have() {
        // Pinned rather than lamented. Nothing here knows what is inside a
        // string, so Python at the start of a line inside a template arms the
        // scanner and the quotes under it read as a docstring. The cost is a
        // passage drawn as prose in a file where `t` shows the source, which is
        // the trade the module doc makes; the test exists so that a future
        // change to the rules has to notice it is changing this.
        let src = "SNIPPET = '''\nclass Example:\n    \"\"\"Not a docstring.\"\"\"\n'''\n";
        assert_eq!(docs_of(&scan(src, "a.py")), ["Not a docstring."]);

        // And the guard that keeps the envelope this small: a fake header and a
        // triple quote at the *same* column is the far commoner shape, and a
        // body has to be indented further than the header that opened it.
        let flat = "SNIPPET = '''\nclass Example:\n\"\"\"Not a docstring.\"\"\"\n'''\n";
        assert!(docs_of(&scan(flat, "a.py")).is_empty());
    }

    #[test]
    fn an_unterminated_docstring_is_left_as_code_rather_than_eating_the_file() {
        // The file that `load::MAX_BYTES` cut in half, and the guess that went
        // wrong, arrive here as the same shape — and in both cases turning
        // everything below into prose is the worst answer available.
        let src = "def f():\n    \"\"\"Started and never finished.\n    x = 1\n";
        assert!(docs_of(&scan(src, "a.py")).is_empty());
    }

    #[test]
    fn a_pyi_stub_is_python_and_a_hash_inside_a_string_is_not_a_comment() {
        let src = "def f(sep: str = \"#\") -> None:\n    \"\"\"Split on it.\"\"\"\n";
        assert_eq!(docs_of(&scan(src, "a.pyi")), ["Split on it."]);
    }

    #[test]
    fn tabs_are_columns_before_an_indent_is_measured() {
        // The code rows go through `source::expand_tabs`, so an indent measured
        // in raw bytes here would push the prose to a different column from the
        // code it belongs to.
        let got = scan("\t/// Doc.\n\tfn f() {}\n", "a.rs");
        let Kind::Doc { indent, .. } = &got[0].kind else {
            panic!("a doc region");
        };
        assert_eq!(*indent, 4, "one tab is four columns, as everywhere else");
    }
}
