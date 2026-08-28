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
//! ## Two languages, and no third
//!
//! `.rs`, `.py` and `.pyi` are the whole of the list — [`language`] is where
//! it is written down — so a `.ts` full of JSDoc, a `.go` with a `//` comment
//! over every exported name, a `.java` with `/** */` blocks: every one of them
//! opens as plain highlighted source and `t` does nothing, because there is no
//! second form for it to toggle to. That is scope rather than difficulty, and
//! the `/** */` reader below would already parse most of them unchanged.
//!
//! What keeps the list from growing by guess is that a language costs more
//! than a comment syntax: it costs a rule for where a doc comment is *allowed*
//! to be, which is the only thing keeping the false-positive envelope small.
//! An extension added without one widens that envelope silently, in a file
//! nobody was watching.
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
//! prose and one keystroke; there is no state to lose, because a doc region is
//! always a whole number of lines and the source is always one key away.
//!
//! What is deliberately *not* claimed — because it was claimed here once and
//! was not true — is that code can never end up inside a region.
//! [`tail_is_clear`] guards the **closing** line and nothing else, so a block
//! opened by a false positive runs to whatever delimiter matches next and
//! every line it passes on the way is handed to the renderer as prose. Prose
//! is not a neutral place to put a statement: a line of `---` becomes a
//! horizontal rule, a `# ...` becomes a heading, and a run of blanks collapses
//! — so those lines can be not merely misdrawn but absent. Two rules narrow
//! that hole rather than close it:
//!
//! - a block that never finds its closing delimiter is refused outright, so
//!   the worst case is bounded by the next matching quote instead of by the
//!   end of the file, and
//! - a Python docstring whose body leaves the indentation of the header that
//!   opened it is refused — see [`docstring`]. A real body cannot do that; a
//!   template string holding a fake `class` almost always does, because the
//!   string's own closing quotes are back at the outer column.
//!
//! What is left is Rust's `/**` at the start of a line inside a raw string,
//! where there is no enclosing indent to measure against and so no equivalent
//! rule to apply. `t` is the answer to it, and the title says which form is
//! up — which is the same answer as for every other mistake here, and the
//! reason the whole heuristic is allowed to exist.
//!
//! The size of all of that, measured rather than asserted. Against `ast` over
//! the 4,606 Python files of a 3.12 install: 34,932 regions found, 55 of
//! `ast`'s 34,953 docstrings missed (0.16%), and **two** regions covering a
//! line `ast` calls a statement — both of them f-strings, which [`opener`]
//! takes on purpose. Rerun that before believing any change to the rules
//! below; it is the only thing here that is evidence rather than argument.
//!
//! ## Dedenting, which is the only rewriting done
//!
//! [`dedent`] takes the block's own Python indentation off. A docstring inside
//! a method sits four or eight columns in, CommonMark reads four spaces as an
//! *indented code block*, and the prose of every method in the file would
//! therefore arrive at the parser as a grey slab of monospace. That indent
//! belongs to the language rather than to the author, which is what makes
//! taking it off a translation rather than an edit — `inspect.cleandoc` and
//! PEP 257 take the same columns off for the same reason, and this matches
//! them.
//!
//! What is left is the indentation the *author* chose, and it is now left
//! alone. There was a second pass here that pulled any surviving indent of
//! four or more columns down to two, justified by a Google-style `Args:` block
//! being four spaces under a label and therefore "a grey slab". That
//! justification misread CommonMark. `Args:` is a paragraph, the four-space
//! lines under it are *lazy continuations*, and an indented code block *cannot
//! interrupt a paragraph* — so the shape the pass existed for never needed it.
//! Measured over this repository's own 1679 doc blocks it changed the rendered
//! output of exactly none of them.
//!
//! It was not free, which is why it is not coming back. Four spaces pulled to
//! two drops a Python doctest's `>>> ` below CommonMark's block-start
//! threshold, so every prompt is eaten as three nested blockquote markers, the
//! input merges with its expected output and the sample is drawn as a
//! quotation. In the same pass a `Returns:` literal block loses its `│ `
//! gutter to become unmarked prose, and a three-deep list comes out two deep
//! with a grandchild promoted to a sibling. `docs/design.md` names that as the
//! fourth thing: the source is always true, and a rendering that has quietly
//! lost a node is worse than one nobody drew.
//!
//! numpydoc needs nothing either way: `Parameters` over a row of `-` is a
//! setext H2 and renders as a heading for free.
//!
//! ## Markup put round a doctest, and nothing moved
//!
//! Dedent is still the only pass that *takes anything off*, and the paragraph
//! above is still the whole of what it takes. [`doctests`] is a different
//! shape of thing and the difference is the point: it inserts lines — a fence,
//! or the blank one a run needs to stop being a paragraph's lazy continuation —
//! and leaves every line the author wrote byte for byte where it was.
//!
//! It exists because a doctest written the way PEP 257, CPython, numpy and
//! essentially every real docstring writes it — flush with the prose around it
//! — arrives at CommonMark, after dedent, with its `>>> ` at column zero, and
//! at column zero that is three nested blockquote markers rather than a
//! prompt. The prompts are eaten, consecutive statements merge into one
//! reflowed paragraph and the sample is drawn as a quotation. That is the
//! failure the deleted pass was charged with, reached by the *common* shape
//! rather than by a rewriting, and it was on this page all along: the
//! regression test that guards it indents its fixture four columns further
//! than the docstring body, which is an indented code block and renders
//! correctly for reasons that have nothing to do with the bug.
//!
//! Why this is allowed where the deleted pass was not, in one sentence: that
//! pass had to decide of an *arbitrary* indented line whether the author meant
//! the indentation, with nothing in the line to decide it from, and this one
//! acts on a marker — `>>> ` is `doctest.PS1`, the one construct in a
//! docstring that is executed rather than read, and where the marker is absent
//! this pass does nothing whatever.
//!
//! Measured the same way as everything else here, over the 4,610 `.py` files of
//! a 3.12 install: 34,933 doc blocks, of which **753 gain a fence** and 631
//! more hold a prompt the blank-line branch answered instead. Not one prompt
//! below the threshold came out unfenced, not one region moved an edge, and
//! `first`/`last` stayed inside their region in every block — which is the
//! ordering inside [`doc`] holding, not a coincidence. The one shape the guard
//! turns away in numbers is an RST section underline: `~~~~~~~~` is a tilde
//! fence to CommonMark, so a docstring ruled that way is already a code block
//! from the underline down and is left exactly as it is found, which is the
//! only answer that does not put a stray ``` on the page.
//!
//! ## What a Google-style `Args:` block still costs, and why
//!
//! It reflows into one sentence — `Args: name: what to call it. size: how many
//! cells wide.` — and that is asked about often enough to be worth writing down
//! rather than rediscovering. Three transforms were tried against
//! `markdown::render` and all three were refused:
//!
//! - **A bullet at each entry's own column** — the additive one, and the one
//!   that sounds right — renders `Args: - name: x - size: y`. A list marker
//!   four columns in cannot interrupt a paragraph any more than an indented
//!   code block can, so the run-on survives with two characters of noise per
//!   entry added to it. That is `normalise`'s exact failure, paying for a
//!   benefit it does not deliver, and it is why the pass above was measured
//!   before it was written.
//! - **A bullet within three columns of the label**, which does make a list,
//!   has to pull each entry left and re-indent its continuation lines to keep
//!   them in the item. That is moving the author's text by an amount read off
//!   its surroundings, which is the whole of what this module says it does not
//!   do.
//! - **A blank line under the label**, which is additive and does separate the
//!   entries, makes the block indented code and hangs a `│ ` gutter on it —
//!   and `markdown` says in as many words that that gutter is a claim the lines
//!   behind it are code. A parameter list is not code. It is also the demotion
//!   this module already charges `normalise` with, run the other way.
//!
//! So the reader gets a run-on with every parameter present, in order, in the
//! author's words, on the lines the gutter names, and `t` one key away. Bad
//! reading and nothing lost, which is the side of the line this pane can live
//! on; the doctest above was on the other side of it, with two statements
//! merged into one row and both prompts deleted.

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
        /// The file line the first word of `text` is actually on, and the line
        /// the last word is on. Both are indexed exactly as `start` and `end`
        /// are, and both are inside `start..end`.
        ///
        /// Not the same as `start` and `end - 1`, and the difference is the
        /// whole reason they are carried. `start` is the line the `/**` or the
        /// `"""` is on and `end - 1` is the line the closing delimiter is on;
        /// neither of those puts a word on the page. The gutter numbers the
        /// rows it draws, so it has to be handed the lines those rows came
        /// from — otherwise the first rendered row wears the number of a line
        /// the reader can see is a bare marker, which is a gutter that lies
        /// about a file it is claiming to index.
        ///
        /// Equal to each other whenever the block's words are all on one line.
        first: usize,
        last: usize,
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

/// `Copy` because [`doc`] takes one: the rules for turning a block's lines into
/// the text a renderer sees are not the same in the two languages, and a `bool`
/// standing in for "is this Python" would have to be read twice — once as PEP
/// 257's first-line rule and once as [`doctests`] — with nothing saying they are
/// the same question.
#[derive(Clone, Copy, PartialEq, Eq)]
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
            out.push(doc(start, i, indent, body, Lang::Rust));
            continue;
        }

        if is_block_open(rest)
            && let Some((end, body)) = rust_block(lines, i, indent)
        {
            out.push(doc(i, end, indent, body, Lang::Rust));
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
/// `t` shows the truth. Two guards bound it, and they are the same fact asked
/// at two moments. The opening quotes must be indented *further* than the
/// header, which throws out the far commoner shape where the fake header and
/// the quotes sit at the same column; and every line of the body must stay
/// there too — see [`docstring`] — which is what stops a block armed this way
/// from running past real statements to reach a quote that matches.
///
/// ## Both spellings of the one-line form
///
/// `"""Doc."""` and `"Doc."` are both docstrings and both are taken. The second
/// is [`short_string`] and it is worth its dozen lines: 1,325 of a Python 3.12
/// install's docstrings are written that way and nothing else, and missing one
/// means a `def` whose documentation the pane can see and will not draw.
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
            // The same fact asked twice: the opening quotes have to be inside
            // the suite, and so does every line under them. The first question
            // is this module's oldest guard and the second is [`docstring`]'s.
            let outer = match want {
                Expect::Module => None,
                Expect::Body { indent: header } => Some(header),
            };
            let allowed = match outer {
                None => indent == 0,
                Some(header) => indent > header,
            };
            if allowed && let Some((end, body)) = docstring(lines, i, rest, outer) {
                out.push(doc(i, end, indent, body, Lang::Python));
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
///
/// Only the triple forms, because only they can open a block that spans lines.
/// The one-quote spelling is [`short_string`]'s, which can answer for the whole
/// of itself on the line it is on.
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
///
/// `outer` is the indent of the `def` or `class` this is supposed to be the
/// body of, and `None` for a module docstring, which has no header above it.
/// **It is the rule that keeps a false opener from swallowing real code.**
/// [`tail_is_clear`] only ever looks at the closing line, so without this a
/// block armed by a template string runs to the next matching quote and takes
/// whatever is between with it:
///
/// ```text
/// CODE = '''
/// class A:
///     """
/// '''
///
/// class B:
///     """
///     Real docstring.
///     """
/// ```
///
/// The `"""` under the fake `class A:` finds its closer four lines down and
/// `class B:` is inside the region — drawn as a markdown heading, which is to
/// say deleted, in a file the reader has no reason to distrust. What gives it
/// away is that a docstring is *inside a suite*: every line of it is indented
/// past the header, and the moment a line comes back to the header's own column
/// or further left, the block has left the body it claimed to be in. Both of
/// the fixtures above are caught by that at their first line — the closing `'''`
/// of the outer string, sitting at column zero.
///
/// The cost is a real docstring with a line deliberately pushed out to column
/// zero, which is legal Python — `os.spawnv` and `posixpath.realpath` both do
/// it — and which now shows as source instead of prose. Measured against `ast`
/// over the 4,606 files of a Python 3.12 install, that is **14 docstrings in
/// 34,946**: 0.04% declined to close a hole that could take a `class` off the
/// page. That is the trade this module makes everywhere, and it is the cheap
/// side of it.
fn docstring(
    lines: &[&str],
    at: usize,
    rest: &str,
    outer: Option<usize>,
) -> Option<(usize, Vec<String>)> {
    let Some((delim, open)) = opener(rest) else {
        return short_string(rest).map(|body| (at + 1, vec![body]));
    };
    let after = &rest[open..];

    if let Some(p) = after.find(delim) {
        return tail_is_clear(&after[p + 3..], "#")
            .then(|| (at + 1, vec![after[..p].trim_end().to_string()]));
    }

    let mut body = vec![after.to_string()];
    let mut i = at + 1;
    while i < lines.len() {
        let line = cols(lines[i]);
        let end = line.find(delim);
        if !inside(&line[..end.unwrap_or(line.len())], outer) {
            return None;
        }
        match end {
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

/// Is this line still inside the suite the docstring's header opened?
///
/// Blank is always inside — a docstring is full of blank lines and none of them
/// says anything about indentation. Anything else has to be indented strictly
/// past the header, which is what being in its body means. `None` is the module
/// docstring, which has no header and so nothing to be outside of.
fn inside(line: &str, outer: Option<usize>) -> bool {
    let Some(outer) = outer else {
        return true;
    };
    line.trim().is_empty() || indent_of(line) > outer
}

/// `"One-line doc."` — a docstring written with one pair of quotes instead of
/// three, which is what the whole first line of a `def` body is when the
/// summary fits on it.
///
/// Worth the extra rule because it is not rare. Measured against `ast` over the
/// 4,606 files of a Python 3.12 install, this shape is **1,325 real
/// docstrings** — the scanner misses 3.9% of everything `ast` calls a docstring
/// without it and 0.16% with it.
///
/// Cheap because it cannot hide anything. A single-quoted string cannot span a
/// line without a backslash, so this only ever claims the one line it is
/// looking at, and only when [`tail_is_clear`] says there is nothing else on
/// it: `"a" \` continuing onto the next line fails that check and is left as
/// code, as does `"a" + x`.
///
/// The backslash skip is right for `r'...'` too: a raw string still cannot be
/// closed by a quote that a backslash precedes, which is the only thing being
/// asked here.
fn short_string(rest: &str) -> Option<String> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < 2 && i < bytes.len() && matches!(bytes[i] | 0x20, b'r' | b'f' | b'b' | b'u') {
        i += 1;
    }
    let quote = *bytes.get(i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' {
            j += 2;
            continue;
        }
        if bytes[j] == quote {
            return tail_is_clear(&rest[j + 1..], "#").then(|| rest[i + 1..j].trim().to_string());
        }
        j += 1;
    }
    None
}

// --- shared ---------------------------------------------------------------

/// Is there nothing but whitespace or a comment left on the closing line?
///
/// A region is a whole number of lines, so a closing delimiter with a statement
/// after it means the block is declined outright rather than cut short.
///
/// **The closing line, and only the closing line.** This was once described
/// here as the one rule that keeps the scanner from ever hiding code, and it is
/// not: a block opened by mistake passes over every line between the false
/// opener and the delimiter that finally matches, and this is never asked about
/// any of them. What guards those is [`inside`], on the Python side where there
/// is a suite to be inside of. See the module doc for what is left over.
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
/// Two of the three things done here are Python's. The first line of a
/// docstring sits on the same physical line as its opening quotes, so it has no
/// indentation to share with the rest and PEP 257 leaves it out of the
/// common-indent calculation; Rust's line comments all start at the same column,
/// so that does not apply. And [`doctests`] is a Python construct by
/// definition. Rust gets `dedent` and nothing else.
fn doc(start: usize, end: usize, indent: usize, mut body: Vec<String>, lang: Lang) -> Region {
    let python = lang == Lang::Python;
    dedent(&mut body, python);
    // **`body[n]` is what file line `start + n` contributed**, and that holds
    // for every shape this module builds one out of: a run of `///` lines is
    // one entry per line, a `/** … */` block opens with the text after the
    // marker and closes with the text before it, and a docstring does the same
    // with its quotes. Nothing above drops a line or adds one, which is what
    // makes the two indices below a line number rather than a guess. A future
    // rule that folded two source lines into one entry would break that
    // quietly, so it would have to carry the offsets itself.
    //
    // [`doctests`] adds lines and therefore breaks that mapping outright, which
    // is why it runs *below* this and not above it. The ordering is the whole
    // guarantee: `first` and `last` are read off the body while it is still one
    // entry per source line, and what the pass does afterwards is invisible to
    // them. Move the call up and the gutter starts numbering rows by a fence.
    let first = body.iter().position(|l| !l.is_empty()).unwrap_or(0);
    let last = body.iter().rposition(|l| !l.is_empty()).unwrap_or(first);
    // A docstring's closing `"""` sits on its own line and leaves an empty one
    // behind; a `///` block often opens or closes with a bare marker. Neither is
    // a blank line the author wrote, and `markdown::render` drops a leading or
    // trailing run of them anyway — so this is not what puts the first row at
    // the top of the block, and it was documented here as if it were.
    //
    // What it does is keep `text` describing exactly the span `first..=last`
    // names. The two have to agree: the gutter puts `first` beside the first
    // row it draws, so a `text` that still began with the blank line under the
    // opening `"""` would be one renderer change away from putting that number
    // beside a blank row. Trimming here is how the value and the numbers stay
    // one fact instead of two.
    body.truncate(last + 1);
    body.drain(..first.min(body.len()));
    if python {
        doctests(&mut body);
    }
    Region {
        start,
        end,
        kind: Kind::Doc {
            text: body.join("\n"),
            indent,
            first: start + first,
            last: start + last,
        },
    }
}

/// Take the block's own indentation off before the markdown parser sees it.
///
/// A docstring in a method starts four or eight columns in and every line
/// under its first one carries that; hand it to CommonMark unchanged and
/// everything below the summary is an indented code block. What is left
/// afterwards is the indentation the *author* chose, which is the only kind
/// markdown should be given a say over.
///
/// The block's *common* indent and not a fixed four, and never more than the
/// block has: this is `inspect.cleandoc`'s rule, so a docstring reads here the
/// way `help()` prints it. Any further rewriting of what survives — pulling a
/// four-space indent down to two, say — is what the module doc argues against
/// at length, on the evidence of a doctest whose `>>> ` prompts it deleted.
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

/// CommonMark's block-start threshold: four columns is an indented code block,
/// and anything less is read as whatever the line says it is.
const CODE_INDENT: usize = 4;

/// Give a doctest the markup that makes CommonMark draw it as the code it is.
///
/// **The failing shape is the common one.** A doctest is written flush with the
/// prose around it — PEP 257's own example, CPython's, numpy's — so after
/// [`dedent`] its `>>> ` is at column zero, where CommonMark reads three nested
/// blockquote markers. The prompts are eaten, `>>> a()` and `>>> b()` reflow
/// into one paragraph, and the sample is drawn behind a `▏ ` quote gutter. A
/// doctest indented *further* than the body around it is the one shape that
/// works already, because four columns is an indented code block; that is the
/// shape the regression test in `viewer` was written with, which is why it went
/// on passing over the top of this.
///
/// **Why acting on `>>> ` is not the deleted `normalise` in another coat.** That
/// pass was asked, of an arbitrary line indented four columns, whether the
/// author meant the indentation — and nothing in such a line answers it. Four
/// columns is a Google-style `Args:` entry, a doctest, a literal block and a
/// nested list, and the pass moved all four the same way because it could not
/// tell them apart. `>>> ` at the start of a line is not that kind of evidence.
/// It is `doctest.PS1`, spelled with its trailing space, and it marks the one
/// thing in a docstring that is *executed* rather than read; CommonMark has no
/// construct that produces it, because three levels of quotation are written
/// `> > >`. Where the marker is absent this function does nothing at all, which
/// is the property the deleted pass could not have.
///
/// And nothing here moves a character. The two branches insert lines:
///
/// - **Below the threshold** — the broken case — a fence at the run's own
///   indent. A fenced block *can* interrupt a paragraph, so `Example:` on the
///   line above needs no blank line inserted under it, and CommonMark strips
///   exactly the fence's own indent off the lines inside, so the sample reaches
///   the page at the column the author put it at relative to its label.
/// - **At or past it** the run is already a code block *unless* a paragraph is
///   running into it, in which case it is that paragraph's lazy continuation.
///   The one thing missing is the blank line that lets the code block start, so
///   that is the one thing added — no fence, so this branch and the branch
///   above draw the same, which they should: `>>> f()` indented and `>>> f()`
///   flush are the same doctest, and `doctest` itself says so by stripping the
///   indentation before it compiles the example.
///
/// The fence carries no info string on purpose. Handing syntect `python` for a
/// block whose every line begins with a prompt colours the prompts as operators
/// and the expected output as whatever it parses to; and the indented branch
/// above has no way to carry a language at all, so a language here would make
/// the two branches differ in the one respect they must not.
///
/// **Guarded like the rest of this module: all or nothing.** A block with a
/// fence anywhere in it is left entirely alone. An author who has written a
/// fence is writing markdown deliberately, their sample is already code, and a
/// second opinion inserted into a block that has one can only be inserted
/// somewhere it does not belong — inside their fence, where it is not markup
/// but text. This is [`undecorate`]'s rule and it is refused for the same
/// reason.
///
/// What it does not reach is a doctest nested inside a list item, where the
/// column that decides these questions is the item's content column rather than
/// zero. The fence branch survives that by accident and the blank-line branch
/// does not. That is a line scanner's limit rather than an oversight — this
/// module is deliberately not a block parser — and the cost is one sample that
/// draws as it draws today.
fn doctests(body: &mut Vec<String>) {
    if body.iter().any(|l| {
        let t = l.trim_start_matches(' ');
        t.starts_with("```") || t.starts_with("~~~")
    }) {
        return;
    }
    let mut i = 0;
    while i < body.len() {
        if !is_prompt(&body[i]) {
            i += 1;
            continue;
        }
        let base = indent_of(&body[i]);
        // `doctest.DocTestParser`'s own rule for where an example ends: the
        // expected output runs to a blank line, and a line that comes back left
        // of the prompt was never part of it. Both are needed — the blank is
        // what separates a sample from the prose under it, and the indent is
        // what stops a sample indented under `Examples:` from swallowing the
        // paragraph that follows the section.
        let mut end = i + 1;
        while end < body.len() {
            let line = &body[end];
            if line.trim().is_empty() || indent_of(line) < base {
                break;
            }
            end += 1;
        }
        if base < CODE_INDENT {
            let pad = " ".repeat(base);
            body.insert(end, format!("{pad}```"));
            body.insert(i, format!("{pad}```"));
            i = end + 2;
        } else if i > 0 && !body[i - 1].trim().is_empty() {
            body.insert(i, String::new());
            i = end + 1;
        } else {
            i = end;
        }
    }
}

/// `doctest.PS1`, at the start of a line and with the space that is part of it.
///
/// The space is not pedantry. It is what tells a prompt from `>>>` used as an
/// arrow or as the tail of a `<<<<<<<` conflict marker, and it costs nothing:
/// a prompt with no space after it is a line with no statement on it, which is
/// not what any of this is here to keep.
fn is_prompt(line: &str) -> bool {
    line.trim_start_matches(' ').starts_with(">>> ")
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
    fn the_only_indentation_taken_off_is_the_blocks_own() {
        // `dedent` removes the columns the *language* put there and stops. The
        // four spaces under `Args:` are the author's, and they survive — which
        // is what a pass that pulled them to two used to prevent, on the
        // argument that CommonMark would otherwise read them as an indented
        // code block. It would not: `Args:` is a paragraph and the line under
        // it is a lazy continuation, and an indented code block cannot
        // interrupt a paragraph. See this module's doc, and
        // `viewer::tests::a_doctest_keeps_its_prompts_and_the_shape_they_give_the_sample`
        // for what the pass cost when it was believed.
        let src = "def f(x):\n    \"\"\"Do it.\n\n    Args:\n        x: the thing.\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(src, "a.py")),
            ["Do it.\n\nArgs:\n    x: the thing."]
        );
    }

    #[test]
    fn a_doctest_written_flush_with_its_prose_is_fenced_rather_than_quoted() {
        // **The shape every real docstring uses.** PEP 257 writes a doctest
        // level with the paragraph above it, so after `dedent` the `>>> ` is at
        // column zero — where CommonMark reads three nested blockquote markers,
        // eats every prompt, reflows the statements into one paragraph and
        // draws the lot as a quotation. A fence at the run's own indent is what
        // stops that, and it adds two lines without moving a character of the
        // sample.
        let flush = "def f():\n    \"\"\"Do it.\n\n    >>> f()\n    >>> g()\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(flush, "a.py")),
            ["Do it.\n\n```\n>>> f()\n>>> g()\n```"]
        );

        // The expected output belongs to the sample and is fenced with it: it
        // runs to the blank line, which is `doctest`'s own rule for where an
        // example ends.
        let output = "def f():\n    \"\"\"Do it.\n\n    >>> f()\n    1\n\n    And that is all.\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(output, "a.py")),
            ["Do it.\n\n```\n>>> f()\n1\n```\n\nAnd that is all."]
        );

        // Two examples separated by a blank line are two samples, not one with a
        // hole in it.
        let two =
            "def f():\n    \"\"\"Do it.\n\n    >>> f()\n    1\n\n    >>> g()\n    2\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(two, "a.py")),
            ["Do it.\n\n```\n>>> f()\n1\n```\n\n```\n>>> g()\n2\n```"]
        );
    }

    #[test]
    fn a_doctest_indented_under_its_label_gets_the_blank_line_and_not_a_fence() {
        // At four columns the run is already an indented code block *unless* a
        // paragraph is running into it — and `Example:` on the line above is
        // exactly that, which makes the sample a lazy continuation of the label
        // and the whole section one run-on line. One blank line is the entire
        // repair, and it leaves the sample looking the same as the flush one
        // does, which is right: they are the same doctest.
        let tight =
            "def f():\n    \"\"\"Do it.\n\n    Example:\n        >>> f()\n        1\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(tight, "a.py")),
            ["Do it.\n\nExample:\n\n    >>> f()\n    1"]
        );

        // With the blank line already there, nothing is added: CommonMark was
        // going to draw this as code anyway, and a second opinion could only
        // change it.
        let loose = "def f():\n    \"\"\"Do it.\n\n    Example:\n\n        >>> f()\n        1\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(loose, "a.py")),
            ["Do it.\n\nExample:\n\n    >>> f()\n    1"]
        );

        // And the paragraph under a section is not swallowed as expected
        // output: a line back at the label's own column is left of the prompt,
        // which is where `doctest` ends an example too. Nothing is inserted
        // *after* the run because nothing needs to be — a line back at column
        // zero ends an indented code block by itself.
        let after = "def f():\n    \"\"\"Do it.\n\n    Example:\n        >>> f()\n    Prose again.\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(after, "a.py")),
            ["Do it.\n\nExample:\n\n    >>> f()\nProse again."]
        );
    }

    #[test]
    fn a_block_that_already_has_a_fence_is_left_entirely_alone() {
        // All or nothing, which is [`undecorate`]'s rule and is refused for the
        // same reason: an author who has written a fence is writing markdown
        // deliberately, and the only places left to insert a second opinion are
        // places it would arrive as text rather than as markup — inside theirs.
        let fenced =
            "def f():\n    \"\"\"Do it.\n\n    ```python\n    >>> f()\n    ```\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(fenced, "a.py")),
            ["Do it.\n\n```python\n>>> f()\n```"]
        );

        // A fence anywhere in the block, not merely one round the sample: this
        // is asked of the whole block precisely so it cannot be asked wrongly of
        // a part of it.
        let elsewhere = "def f():\n    \"\"\"Do it.\n\n    ```\n    x = 1\n    ```\n\n    >>> f()\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(elsewhere, "a.py")),
            ["Do it.\n\n```\nx = 1\n```\n\n>>> f()"]
        );
    }

    #[test]
    fn a_quotation_stays_a_quotation_and_rust_is_never_touched() {
        // The marker is `>>> ` and nothing near it. One `>` is a block quote the
        // author wrote and means, two is a nested one, and `>>>` with no space
        // is not a prompt — none of them is a doctest and none of them is
        // rewritten.
        let quoted = "def f():\n    \"\"\"Do it.\n\n    > Somebody said this.\n    >> And this.\n    >>>not this either.\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(quoted, "a.py")),
            ["Do it.\n\n> Somebody said this.\n>> And this.\n>>>not this either."]
        );

        // Rust has no doctests to find. `/// >>> f()` in a Rust doc comment is
        // somebody quoting a Python session, and this pass never runs on it.
        assert_eq!(
            docs_of(&scan("/// Do it.\n///\n/// >>> f()\nfn f() {}\n", "a.rs")),
            ["Do it.\n\n>>> f()"]
        );
    }

    #[test]
    fn fencing_a_doctest_moves_neither_the_regions_edges_nor_its_line_numbers() {
        // The pass adds lines to the *text handed to the renderer*, and the
        // gutter numbers rows from `first` and `last`, which are read off the
        // body while it is still one entry per source line. If that ordering
        // ever reverses, the first row of a sample wears the number of a fence
        // that is not in the file — so it is pinned here rather than left to
        // `doc`'s comment.
        let src = "def f():\n    \"\"\"Do it.\n\n    >>> f()\n    1\n    \"\"\"\n    return 1\n";
        let got = scan(src, "a.py");
        let Kind::Doc { first, last, .. } = got[1].kind else {
            panic!("the docstring is a doc region");
        };
        assert_eq!((got[1].start, got[1].end), (1, 6));
        assert_eq!((first, last), (1, 4), "the summary's line and the sample's");
        assert_eq!(got[2], Region::code(6, 8));

        // And the partition still puts the file back, which is the property the
        // pane leans its weight on.
        let lines: Vec<&str> = src.split('\n').collect();
        let back: Vec<&str> = got
            .iter()
            .flat_map(|r| &lines[r.start..r.end])
            .copied()
            .collect();
        assert_eq!(back.join("\n"), src);
    }

    #[test]
    fn a_docstring_with_a_fence_in_it_keeps_every_column_of_its_sample() {
        // Inside a fence the indentation is the *code's*. `dedent` takes the
        // block's common indent off every line alike, so the nesting inside the
        // sample survives the way it survives in `inspect.cleandoc` — and
        // nothing else touches it, which is why there is no fence exemption
        // here any more for there to be a hole in.
        let src = "def f():\n    \"\"\"Use it.\n\n    ```python\n    if x:\n        f()\n    ```\n    \"\"\"\n";
        assert_eq!(
            docs_of(&scan(src, "a.py")),
            ["Use it.\n\n```python\nif x:\n    f()\n```"]
        );
    }

    #[test]
    fn a_block_reports_the_lines_its_words_are_actually_on() {
        // The gutter numbers rendered rows from these, so they are the lines
        // that carry text and not the lines that carry delimiters. `start` is
        // the `/**`; `first` is the sentence under it.
        let numbers = |text: &str, name: &str| -> Vec<(usize, usize, usize, usize)> {
            scan(text, name)
                .into_iter()
                .filter_map(|r| match r.kind {
                    Kind::Doc { first, last, .. } => Some((r.start, r.end, first, last)),
                    Kind::Code => None,
                })
                .collect()
        };

        // Lines 0..3 are `/**`, the sentence, `*/` — and only line 1 has a word.
        assert_eq!(
            numbers("/**\nOne.\n*/\nfn f() {}\n", "a.rs"),
            [(0, 3, 1, 1)]
        );
        // A `///` run opening and closing on a bare marker, spanning 1..=2.
        assert_eq!(
            numbers("///\n/// One.\n/// Two.\n///\nfn f() {}\n", "a.rs"),
            [(0, 4, 1, 2)]
        );
        // Python: the summary is on the opening line, so `first` is `start`, and
        // the closing `\"\"\"` on its own line is not the last word.
        assert_eq!(
            numbers("def f():\n    \"\"\"One.\n\n    Two.\n    \"\"\"\n", "a.py"),
            [(1, 5, 1, 3)]
        );
        // A block with no words at all still names one line rather than none.
        assert_eq!(numbers("///\n///\nfn f() {}\n", "a.rs"), [(0, 2, 0, 0)]);
    }

    #[test]
    fn a_false_opener_may_not_swallow_the_code_between_it_and_its_closer() {
        // `tail_is_clear` guards the closing line and nothing else, so both of
        // these used to be one region running from a fake docstring down to a
        // quote that matched — with a real statement inside it, drawn as prose
        // and in the first case drawn as a markdown heading, which is to say not
        // drawn at all. What refuses them is that a docstring is inside a suite:
        // the outer string's own closing quotes are back at column zero, and
        // that is a line the body of a `class A:` cannot contain.
        let hidden_class = "CODE = '''\nclass A:\n    \"\"\"\n'''\n\nclass B:\n    \"\"\"\n    Real docstring.\n    \"\"\"\n";
        let got = scan(hidden_class, "a.py");
        assert!(
            got.iter().all(|r| !r.is_doc() || r.start >= 5),
            "a region opened above `class B:`: {got:?}"
        );
        assert_eq!(
            docs_of(&got),
            ["Real docstring."],
            "and the real one below it is still found"
        );

        let hidden_call =
            "TEMPLATE = \"\"\"\ndef f():\n    '''\n\"\"\"\nprint(\"hello\")\nX = '''\nend'''\n";
        assert!(
            docs_of(&scan(hidden_call, "a.py")).is_empty(),
            "`print(\"hello\")` is code and must stay on the page"
        );
    }

    #[test]
    fn a_line_pushed_out_to_the_margin_is_what_that_guard_costs() {
        // Named rather than hidden. A docstring may legally contain a line at
        // column zero, and the rule above cannot tell that from a template
        // string's closing quotes — so it is declined and shown as source. The
        // trade is a rendering occasionally refused against a rendering that
        // occasionally eats a statement, and this module takes the first every
        // time.
        let src = "def f():\n    \"\"\"Doc.\n\nFlush left on purpose.\n    \"\"\"\n";
        assert!(docs_of(&scan(src, "a.py")).is_empty());
        // Blank lines are not that, and a docstring is full of them.
        let blanks = "def f():\n    \"\"\"Doc.\n\n    More.\n    \"\"\"\n";
        assert_eq!(docs_of(&scan(blanks, "a.py")), ["Doc.\n\nMore."]);
    }

    #[test]
    fn a_summary_in_one_pair_of_quotes_is_a_docstring_too() {
        // 1366 of them in the standard library and the packages beside it, and
        // every one was invisible to this scanner. Safe to take because it can
        // only ever claim the line it is on: a single-quoted string cannot span
        // one.
        assert_eq!(docs_of(&scan("def f():\n    \"Doc.\"\n", "a.py")), ["Doc."]);
        assert_eq!(
            docs_of(&scan("'Module doc.'\nimport os\n", "a.py")),
            ["Module doc."]
        );
        assert_eq!(
            docs_of(&scan("def f():\n    r'Say \\'hi\\'.'\n", "a.py")),
            ["Say \\'hi\\'."]
        );
        // And it declines everything it cannot answer for on that one line: an
        // implicit continuation, and anything with code after the closing quote.
        assert!(docs_of(&scan("def f():\n    \"a\" \\\n    \"b\"\n", "a.py")).is_empty());
        assert!(docs_of(&scan("def f():\n    \"a\" + x\n", "a.py")).is_empty());
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
