//! Markdown to styled rows, laid out for a known column count.
//!
//! This renders *for a width*, not to an abstract document, because the pane
//! scrolls by physical row. Every block therefore commits to its own wrapping
//! here — bullets hang under their marker, quote gutters continue through
//! blank lines, code breaks at the column rather than at a space — and the
//! result is a flat `Vec<Line>` the pane can slice.
//!
//! It is a state machine over `pulldown_cmark` events rather than a call into
//! `tui-markdown`, for one reason: prefixes. A renderer that produces a `Text`
//! and leaves wrapping to the widget cannot indent a continuation line under
//! its own bullet, and cannot keep a quote gutter unbroken. In a pane that is
//! forty columns wide, almost every list item wraps.

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd,
};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::theme::{Mode, Theme};
use super::{mermaid, source};
use crate::text::wrap::{self, pad_to, spans_width};

/// Two cells, so nested quotes stay legible without a separator.
const QUOTE_GUTTER: &str = "▏ ";
/// A fresh source line inside a fenced block...
const CODE_GUTTER: &str = "│ ";
/// ...and the continuation of one that was too wide. Distinguishing them is
/// the only way to tell a wrapped line from the next statement.
const CODE_WRAP_GUTTER: &str = "┆ ";
/// Below this many columns of usable text, decoration costs more than it says.
const CRAMPED: usize = 12;
/// The rule under an H1, spanning the whole block. Heavy rather than light
/// because a full-width `─` is already what a thematic break draws, and a title
/// that renders as a horizontal rule is a worse lie than a title with no rule
/// under it at all.
const H1_RULE: &str = "━";
/// The rule under an H2, which is light but stops at the end of the heading's
/// own text. Short *and* light, so neither half of it can be read as a break
/// either — the two rules differ in stroke and in extent, not in colour, both
/// being the same `dim` as every other piece of chrome on the page.
const H2_RULE: &str = "─";
/// Cells an H4 and below is indented per level past three. Two, matching the
/// quote gutter and the bullet, so a deep heading steps sideways by the same
/// amount as everything else in this file that steps sideways.
const HEADING_STEP: usize = 2;
/// What an image is reduced to, since a terminal cannot show one. Not an emoji:
/// every glyph this module draws is a BMP symbol that a fixed-width font has an
/// opinion about, and an emoji is two cells wide in some terminals and one in
/// others — which is the wrap arithmetic here going wrong on somebody else's
/// machine. A hatched square rather than `▪`, which is already the bullet three
/// list levels down.
const IMAGE_GLYPH: &str = "▨ ";
/// The share of the pane a link's destination may take before it is cut down to
/// a host. A third: at the forty-six columns this pane routinely is that is
/// fifteen cells, which `x.dev` and `./plan.md` clear and a campaign URL with a
/// query string on it does not. See [`elide_url`].
const URL_SHARE: usize = 3;
/// Narrowest a table column may be and still hold a word rather than a
/// syllable. Below `MIN_COL * columns` the grid is abandoned entirely.
const MIN_COL: usize = 8;
/// Floor for an individual column once the grid is being squeezed.
const FLOOR_COL: usize = 4;

pub fn options() -> Options {
    // Everything here is something an agent actually writes: tables in design
    // docs, task lists in plans, `> [!NOTE]` alerts, YAML front matter. Left
    // off: smart punctuation, which silently rewrites the author's text.
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

pub fn render(source: &str, width: usize, mode: Mode) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let mut r = Renderer::new(width, mode);
    for event in Parser::new_ext(source, options()) {
        r.event(event);
    }
    r.finish()
}

struct ListLevel {
    /// `Some(n)` for an ordered list, holding the *next* number.
    next: Option<u64>,
}

/// What a raw block is being captured *as*, which is what decides how it comes
/// back out. All three arrive as unparsed text and used to be told apart by a
/// bare `dim: bool`; front matter now has a rendering of its own, and a third
/// state spelled as a second boolean is how the third state gets forgotten.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Raw {
    /// A fence or an indented block: highlighted, behind a gutter.
    Code,
    /// An HTML block. Dim rather than highlighted, because it is the document's
    /// own plumbing rather than something the author was writing to be read.
    Html,
    /// YAML front matter, which is tried as a key/value header first and falls
    /// back to the same dim block as `Html`. See [`front_matter`].
    Meta,
}

#[derive(Default)]
struct TableAcc {
    align: Vec<Alignment>,
    header: Vec<Vec<Span<'static>>>,
    rows: Vec<Vec<Vec<Span<'static>>>>,
    row: Vec<Vec<Span<'static>>>,
    in_head: bool,
}

struct Renderer {
    width: usize,
    /// The palette every span here is coloured from. Held as a `&'static`
    /// rather than threaded per call because a document is rendered as one
    /// unit — a theme that could change halfway down would be a bug, not a
    /// feature — and both palettes are statics anyway.
    theme: &'static Theme,
    /// The same choice, in the form `source` wants it: syntect's themes are
    /// held separately and picked by name.
    mode: Mode,
    out: Vec<Line<'static>>,

    /// Inline spans of the block being built, not yet wrapped.
    spans: Vec<Span<'static>>,
    style: Style,
    styles: Vec<Style>,
    /// `(index into spans where the link text started, destination)`.
    link: Option<(usize, String)>,

    quote_depth: usize,
    lists: Vec<ListLevel>,
    /// Total content indent inside the current list item, in cells.
    indent: usize,
    /// Marker widths, so leaving an item unwinds exactly what it added.
    indents: Vec<usize>,
    /// Drawn on the first line of an item's first block, then gone.
    marker: Option<(Vec<Span<'static>>, usize)>,

    /// `(language, text, kind)` while inside a fenced block, an indented block,
    /// an HTML block or a front-matter block.
    code: Option<(String, String, Raw)>,
    table: Option<TableAcc>,

    need_blank: bool,
    suppress_blank: bool,
}

impl Renderer {
    fn new(width: usize, mode: Mode) -> Self {
        Self {
            width,
            theme: mode.theme(),
            mode,
            out: Vec::new(),
            spans: Vec::new(),
            style: Style::default(),
            styles: Vec::new(),
            link: None,
            quote_depth: 0,
            lists: Vec::new(),
            indent: 0,
            indents: Vec::new(),
            marker: None,
            code: None,
            table: None,
            need_blank: false,
            suppress_blank: false,
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_inline();
        self.out
    }

    // --- events ----------------------------------------------------------

    fn event(&mut self, event: Event<'_>) {
        // Raw block content is captured verbatim; none of the inline handling
        // below applies inside a fence.
        if self.code.is_some() {
            match event {
                Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                    if let Some((_, buf, _)) = self.code.as_mut() {
                        buf.push_str(&t);
                    }
                }
                Event::End(TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_)) => {
                    if let Some((lang, text, kind)) = self.code.take() {
                        self.emit_code(&lang, &text, kind);
                    }
                }
                _ => {}
            }
            return;
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.push_text(&t),
            Event::Code(t) => {
                let style = self.style.fg(self.theme.code);
                self.spans.push(Span::styled(t.to_string(), style));
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                self.spans.push(Span::styled(t.to_string(), self.theme.dim()))
            }
            Event::FootnoteReference(label) => {
                self.spans.push(Span::styled(
                    format!("[^{label}]"),
                    self.theme.dim().add_modifier(Modifier::BOLD),
                ));
            }
            Event::SoftBreak => self.spans.push(Span::raw(" ")),
            Event::HardBreak => self.flush_inline(),
            Event::Rule => {
                self.start_block();
                let (first, _) = self.prefixes();
                let avail = self.width.saturating_sub(spans_width(&first)).max(1);
                let mut line = first;
                line.push(Span::styled("─".repeat(avail), self.theme.dim()));
                self.out.push(Line::from(line));
                self.need_blank = true;
            }
            Event::TaskListMarker(done) => {
                let (glyph, colour) = if done {
                    ("✔ ", self.theme.ok)
                } else {
                    ("☐ ", self.theme.dim)
                };
                self.spans.push(Span::styled(glyph, Style::default().fg(colour)));
            }
            // Maths is not enabled, so these cannot arrive; showing the source
            // is still better than dropping it if that ever changes.
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.spans.push(Span::styled(t.to_string(), self.theme.dim()));
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.start_block(),
            Tag::Heading { .. } => self.start_block(),

            Tag::BlockQuote(kind) => {
                self.start_block();
                self.quote_depth += 1;
                if let Some(kind) = kind {
                    self.alert_label(kind);
                }
            }

            Tag::CodeBlock(kind) => {
                self.start_block();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new(), Raw::Code));
            }
            // Neither of these is highlighted. HTML is the document's own
            // plumbing, and front matter is metadata *about* the document that
            // sits at the top, where highlighting it would make the loudest
            // thing on the page the one thing the reader did not open the file
            // for. They part company in `emit_code`, not here.
            Tag::HtmlBlock => {
                self.start_block();
                self.code = Some((String::new(), String::new(), Raw::Html));
            }
            Tag::MetadataBlock(_) => {
                self.start_block();
                self.code = Some((String::new(), String::new(), Raw::Meta));
            }

            Tag::List(start) => {
                self.start_block();
                self.lists.push(ListLevel { next: start });
            }
            Tag::Item => {
                let marker = self.next_marker();
                let w = spans_width(&marker);
                self.marker = Some((marker, w));
                self.indent += w;
                self.indents.push(w);
                // The item's own first block must not open with a blank line;
                // blocks *after* it inside the same item still may.
                self.suppress_blank = true;
            }
            Tag::FootnoteDefinition(label) => {
                self.start_block();
                let marker = vec![Span::styled(
                    format!("[^{label}] "),
                    self.theme.dim().add_modifier(Modifier::BOLD),
                )];
                let w = spans_width(&marker);
                self.marker = Some((marker, w));
                self.indent += w;
                self.indents.push(w);
                self.suppress_blank = true;
            }

            Tag::Table(align) => {
                self.start_block();
                self.table = Some(TableAcc {
                    align,
                    ..TableAcc::default()
                });
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = true;
                }
            }
            Tag::TableRow | Tag::TableCell => self.spans.clear(),

            Tag::Emphasis => self.push_style(self.style.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(self.style.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(self.style.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Superscript | Tag::Subscript => self.push_style(self.style),

            Tag::Link { dest_url, .. } => {
                self.link = Some((self.spans.len(), dest_url.to_string()));
                self.push_style(
                    Style::default()
                        .fg(self.theme.link)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            // The alt text is the whole of what a terminal can show of an image,
            // so it is the whole of what is drawn — the brackets around it were
            // three cells of punctuation announcing something the glyph now says
            // in one. Dim, because an alt is a stand-in for something that is
            // not on the page rather than a sentence of the document; the
            // destination is not shown at all, since unlike a link there is
            // nothing the reader could do with it here. `t` has the source.
            Tag::Image { .. } => {
                self.spans.push(Span::styled(IMAGE_GLYPH, self.theme.dim()));
                self.push_style(self.theme.dim());
            }

            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_inline();
                self.need_blank = true;
            }
            // A heading used to be prefixed with its own `#`s, dimmed, on the
            // argument that a terminal has no font size so the level had to be
            // readable as text. The reasoning was right and the answer was
            // wrong: it left the last literal markdown syntax on an otherwise
            // fully rendered page, and it spent up to seven cells of a
            // forty-column pane saying something the shape of the block can say
            // for free. So the level is still carried by something other than
            // hue — `theme`'s rule, the same one that underlines every link —
            // but that something is now structural. A rule under the title, a
            // shorter rule under a section, nothing under a sub-section, and an
            // indent below that. Read down a page, those four are as distinct
            // in greyscale as they are in colour.
            TagEnd::Heading(level) => {
                let level = level as usize;
                let (mut first, mut rest) = self.prefixes();
                // Measured off the *continuation* prefix, because that is the
                // one every line but the first pays and the one the rule below
                // sits behind. Inside a quote or a list item it is already
                // several cells wide, which is what keeps the decoration here
                // honest about the room it actually has.
                let avail = self.width.saturating_sub(spans_width(&rest));
                // The threshold the code gutter and the table grid already
                // answer to: under it, decoration is spending cells the words
                // needed. A cramped heading is bold and coloured and nothing
                // else, which is exactly what it was before this pane learned
                // to draw rules.
                let roomy = avail >= CRAMPED;

                // H4 and below share a colour — `Theme::heading` gives all of
                // them `warn` — and get no rule, so the indent is the only
                // thing left that says which of them you are looking at. It
                // goes on *both* prefixes: a wrapped H5 whose second line
                // straightened up under the body text would be reading as a
                // paragraph from there on. Clamped the way `prefixes` clamps
                // the list indent, and for the same reason — nesting must never
                // eat the whole pane.
                let step =
                    (HEADING_STEP * level.saturating_sub(3)).min(avail.saturating_sub(CRAMPED));
                if step > 0 {
                    first.push(Span::raw(" ".repeat(step)));
                    rest.push(Span::raw(" ".repeat(step)));
                }

                let content = std::mem::take(&mut self.spans);
                let content = restyle(content, self.theme.heading(level as u8));
                let rows = wrap::wrap_spans(content, self.width, &first, &rest);
                // The widest row this heading actually drew, which is what an
                // H2's rule is measured against. Taken before the rows are
                // handed over, since that is the only moment it is knowable —
                // the wrap decides it, not the text.
                let drawn = rows
                    .iter()
                    .map(|l| spans_width(&l.spans))
                    .max()
                    .unwrap_or(0);
                self.out.extend(rows);

                let rule = match level {
                    1 if roomy => Some((H1_RULE, avail)),
                    // Only as wide as the heading, so a short H2 is visibly not
                    // a break and a long one is visibly not an H1. `clamp`'s
                    // lower bound cannot invert here: `roomy` has already put
                    // `avail` at CRAMPED or more.
                    2 if roomy => Some((
                        H2_RULE,
                        drawn.saturating_sub(spans_width(&rest)).clamp(1, avail),
                    )),
                    _ => None,
                };
                if let Some((glyph, cells)) = rule {
                    // Dim, like the quote and code gutters: a rule is the pane
                    // talking about the document, not part of it.
                    let mut line = rest;
                    line.push(Span::styled(glyph.repeat(cells), self.theme.dim()));
                    self.out.push(Line::from(line));
                }
                self.need_blank = true;
            }

            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.need_blank = true;
            }

            TagEnd::List(_) => {
                self.lists.pop();
                self.need_blank = true;
            }
            TagEnd::Item | TagEnd::FootnoteDefinition => {
                self.flush_inline();
                if let Some(w) = self.indents.pop() {
                    self.indent = self.indent.saturating_sub(w);
                }
                self.marker = None;
                // Items butt up against each other. In a forty-column pane a
                // loose list rendered loosely is mostly blank lines.
                self.need_blank = false;
                self.suppress_blank = false;
            }

            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.spans);
                if let Some(t) = self.table.as_mut() {
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.header = std::mem::take(&mut t.row);
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.emit_table(t);
                }
                self.need_blank = true;
            }

            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript => self.pop_style(),

            TagEnd::Link => {
                self.pop_style();
                // The destination is worth showing only when it is not already
                // on screen; an autolink would otherwise print twice.
                if let Some((start, url)) = self.link.take() {
                    let text: String = self.spans[start.min(self.spans.len())..]
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    if text.trim() != url.trim() && !url.is_empty() {
                        let shown = elide_url(&url, self.width);
                        self.spans
                            .push(Span::styled(format!(" ({shown})"), self.theme.dim()));
                    }
                }
            }
            TagEnd::Image => self.pop_style(),

            TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {
                // Handled by the raw-capture branch in `event`; only reachable
                // if a block opened and closed with nothing in between.
                if let Some((lang, text, kind)) = self.code.take() {
                    self.emit_code(&lang, &text, kind);
                }
            }

            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    // --- inline ----------------------------------------------------------

    fn push_text(&mut self, text: &str) {
        self.spans.push(Span::styled(text.to_string(), self.style));
    }

    fn push_style(&mut self, style: Style) {
        self.styles.push(self.style);
        self.style = style;
    }

    fn pop_style(&mut self) {
        self.style = self.styles.pop().unwrap_or_default();
    }

    // --- blocks ----------------------------------------------------------

    fn start_block(&mut self) {
        // A block can open while the previous one is still unflushed — a list
        // nested directly under an item's text is the everyday case. Flushing
        // here is what keeps that text attached to its own marker.
        self.flush_inline();
        if std::mem::take(&mut self.suppress_blank) {
            self.need_blank = false;
            return;
        }
        if std::mem::take(&mut self.need_blank) && !self.out.is_empty() {
            // A blank line inside a quote still draws the gutter, or one quote
            // renders as two.
            self.out.push(Line::from(self.quote_prefix()));
        }
    }

    fn flush_inline(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let content = std::mem::take(&mut self.spans);
        let (first, rest) = self.prefixes();
        self.out
            .extend(wrap::wrap_spans(content, self.width, &first, &rest));
    }

    fn quote_prefix(&self) -> Vec<Span<'static>> {
        (0..self.quote_depth)
            .map(|_| Span::styled(QUOTE_GUTTER, self.theme.dim()))
            .collect()
    }

    /// `(first line, continuation)` prefixes for the block about to be drawn.
    /// Consumes any pending list marker, which is what makes it appear once.
    fn prefixes(&mut self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let quote = self.quote_prefix();
        // Nesting must never eat the whole pane: past this, indentation is
        // dropped and the structure has to come from the markers alone.
        let room = self
            .width
            .saturating_sub(spans_width(&quote))
            .saturating_sub(CRAMPED);
        let indent = self.indent.min(room);

        let mut rest = quote.clone();
        if indent > 0 {
            rest.push(Span::raw(" ".repeat(indent)));
        }

        let first = match self.marker.take() {
            Some((marker, w)) => {
                let mut first = quote;
                let pad = indent.saturating_sub(w);
                if pad > 0 {
                    first.push(Span::raw(" ".repeat(pad)));
                }
                first.extend(marker);
                first
            }
            None => rest.clone(),
        };
        (first, rest)
    }

    fn next_marker(&mut self) -> Vec<Span<'static>> {
        let depth = self.lists.len();
        // Read before the list below is borrowed mutably.
        let bullet = self.theme.accent;
        match self.lists.last_mut() {
            Some(ListLevel { next: Some(n) }) => {
                let text = format!("{n}. ");
                *n += 1;
                vec![Span::styled(text, Style::default().fg(bullet))]
            }
            _ => {
                // Bullet by depth, so a nested list is distinguishable when it
                // has wrapped and the indentation is no longer obvious.
                let glyph = match depth {
                    0 | 1 => "• ",
                    2 => "◦ ",
                    _ => "▪ ",
                };
                vec![Span::styled(glyph, Style::default().fg(bullet))]
            }
        }
    }

    /// GFM alerts (`> [!WARNING]`) get a coloured label line, because the
    /// marker itself is swallowed by the parser.
    fn alert_label(&mut self, kind: BlockQuoteKind) {
        let (label, colour) = match kind {
            BlockQuoteKind::Note => ("ⓘ Note", self.theme.info),
            BlockQuoteKind::Tip => ("✱ Tip", self.theme.ok),
            BlockQuoteKind::Important => ("‼ Important", self.theme.special),
            BlockQuoteKind::Warning => ("⚠ Warning", self.theme.warn),
            BlockQuoteKind::Caution => ("⚠ Caution", self.theme.danger),
        };
        let mut line = self.quote_prefix();
        line.push(Span::styled(
            label,
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ));
        self.out.push(Line::from(line));
    }

    fn emit_code(&mut self, lang: &str, text: &str, kind: Raw) {
        // Fences arrive with a trailing newline that is fence syntax, not a
        // blank line the author wrote.
        let body = text.strip_suffix('\n').unwrap_or(text);
        // Taken before any branch draws, because it consumes the pending list
        // marker and the marker may only be spent once.
        let (pfirst, prest) = self.prefixes();
        let avail = self.width.saturating_sub(spans_width(&prest));

        // A mermaid fence is the one fenced language whose source is not the
        // point of it, so it is drawn rather than reproduced — when it can be.
        // `mermaid::render` declines every diagram type it does not draw and
        // every one it cannot fit, and declining lands here, in the code block
        // the fence has always had.
        if mermaid::is_mermaid(lang)
            && let Some(rows) = mermaid::render(body, avail, self.theme)
        {
            return self.emit_diagram(rows, &pfirst, &prest);
        }

        // Front matter gets the same bargain the mermaid fence above just made:
        // rendered when it can be rendered faithfully, and otherwise handed
        // back to the block it has always had. `front_matter` is the one that
        // decides, and it declines far more than it accepts.
        if kind == Raw::Meta
            && let Some(pairs) = front_matter(body)
        {
            return self.emit_front_matter(pairs, &pfirst, &prest);
        }

        let rows = match kind {
            Raw::Code => source::highlight_code(body, lang, self.mode),
            Raw::Html | Raw::Meta => source::plain(body, self.theme.dim()),
        };

        // The gutter is the only thing marking the block's extent, so it is the
        // last decoration to go — but at eight usable columns it has to.
        let gutter = avail >= 8;

        for (i, row) in rows.into_iter().enumerate() {
            let mut first = if i == 0 { pfirst.clone() } else { prest.clone() };
            let mut cont = prest.clone();
            if gutter {
                first.push(Span::styled(CODE_GUTTER, self.theme.dim()));
                cont.push(Span::styled(CODE_WRAP_GUTTER, self.theme.dim()));
            }
            self.out
                .extend(wrap::hard_wrap(row, self.width, &first, &cont));
        }
        self.need_blank = true;
    }

    /// A drawn diagram, which gets no code gutter.
    ///
    /// The gutter exists to mark where a code block starts and stops, and box
    /// art already says that about itself — while two of the cells it would
    /// cost are two the drawing was measured for and is now short of. The
    /// outline fallback goes without one for the same reason: its arrows are
    /// not going to be mistaken for a bullet list.
    fn emit_diagram(&mut self, rows: mermaid::Rows, pfirst: &[Span<'static>], prest: &[Span<'static>]) {
        for (i, row) in rows.into_iter().enumerate() {
            let first = if i == 0 { pfirst.to_vec() } else { prest.to_vec() };
            // Hard-wrapped as insurance rather than as layout. `mermaid` was
            // given the column count and commits to it, so a row that overflows
            // is a bug in there — this is what keeps such a bug inside the pane
            // while the tests are what find it.
            self.out.extend(wrap::hard_wrap(row, self.width, &first, prest));
        }
        self.need_blank = true;
    }

    /// Front matter as a header rather than as a block of source.
    ///
    /// A `│ ` gutter is a claim that the lines behind it are code, and the
    /// moment the pairs are being read as pairs that claim is false — it was
    /// also, in an agent-written plan, the loudest thing on the page before the
    /// reader had reached a word of the plan. So the gutter goes and the shape
    /// carries it instead: one row per key, the key bold and the value not,
    /// both dim, because all of it is still metadata *about* the document.
    ///
    /// The key is clipped rather than allowed to run, because it is a wrap
    /// *prefix* and a prefix is the one thing `wrap_spans` will let overflow the
    /// pane — a forty-cell `implementation_strategy: ` at eighteen columns is
    /// not a hypothetical in this codebase. That is the same trade, and the same
    /// `clip_label`, the narrow-table records already make one method below.
    fn emit_front_matter(
        &mut self,
        pairs: Vec<(String, String)>,
        pfirst: &[Span<'static>],
        prest: &[Span<'static>],
    ) {
        // Below two cells there is no room for a key and its colon, so the
        // values go out bare and in order. Ugly, and it is still the data.
        let room = self.width.saturating_sub(spans_width(prest) + 2);
        let key_style = self.theme.dim().add_modifier(Modifier::BOLD);

        for (i, (key, value)) in pairs.into_iter().enumerate() {
            let mut first = if i == 0 {
                pfirst.to_vec()
            } else {
                prest.to_vec()
            };
            if room >= 2 {
                first.push(Span::styled(
                    format!("{}: ", clip_label(&key, room)),
                    key_style,
                ));
            }
            // Hanging by two, as the table records hang: a wrapped value that
            // came back to column zero would read as the next key.
            let mut cont = prest.to_vec();
            cont.push(Span::raw("  "));
            self.out.extend(wrap::wrap_spans(
                vec![Span::styled(value, self.theme.dim())],
                self.width,
                &first,
                &cont,
            ));
        }
        self.need_blank = true;
    }

    fn emit_table(&mut self, t: TableAcc) {
        let cols = t
            .rows
            .iter()
            .map(Vec::len)
            .chain([t.header.len(), t.align.len()])
            .max()
            .unwrap_or(0);
        if cols == 0 {
            return;
        }

        let (pfirst, prest) = self.prefixes();
        let avail = self.width.saturating_sub(spans_width(&prest));
        // Every column pays for a ` │ ` before it except the first.
        let content = avail.saturating_sub(3 * (cols - 1));
        if content < MIN_COL * cols {
            return self.emit_table_as_records(t, cols, &pfirst, &prest);
        }

        let widths = fit(&t, cols, content);
        let sep = Span::styled(" │ ", self.theme.dim());
        let mut first_line = true;

        let mut emit_row = |out: &mut Vec<Line<'static>>, row: &[Vec<Span<'static>>], head: bool| {
            let cells: Vec<Vec<Line<'static>>> = (0..cols)
                .map(|c| {
                    let cell = row.get(c).cloned().unwrap_or_default();
                    let cell = if head { restyle_bold(cell) } else { cell };
                    wrap::wrap_spans(cell, widths[c], &[], &[])
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);

            for r in 0..height {
                let mut line = if first_line {
                    first_line = false;
                    pfirst.clone()
                } else {
                    prest.clone()
                };
                for c in 0..cols {
                    if c > 0 {
                        line.push(sep.clone());
                    }
                    let piece = cells[c].get(r).map(|l| l.spans.clone()).unwrap_or_default();
                    line.extend(align(piece, widths[c], t.align.get(c).copied()));
                }
                out.push(Line::from(line));
            }
        };

        if !t.header.is_empty() {
            emit_row(&mut self.out, &t.header, true);
            let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            let mut line = prest.clone();
            line.push(Span::styled(rule.join("─┼─"), self.theme.dim()));
            self.out.push(Line::from(line));
        }
        for row in &t.rows {
            emit_row(&mut self.out, row, false);
        }
    }

    /// A grid needs columns the pane does not have. One field per line keeps
    /// the data readable; keeping the grid would not.
    fn emit_table_as_records(
        &mut self,
        t: TableAcc,
        cols: usize,
        pfirst: &[Span<'static>],
        prest: &[Span<'static>],
    ) {
        // Below this there is no room for a label and a value, so the cells go
        // out bare and in column order. Ugly, but it is still the data.
        let room = self.width.saturating_sub(spans_width(prest) + 2);
        let mut first_line = true;

        for (i, row) in t.rows.iter().enumerate() {
            if i > 0 {
                self.out.push(Line::from(prest.to_vec()));
            }
            for c in 0..cols {
                let label: String = t
                    .header
                    .get(c)
                    .map(|h| h.iter().map(|s| s.content.as_ref()).collect())
                    .unwrap_or_else(|| format!("col {}", c + 1));

                let mut first = if first_line {
                    first_line = false;
                    pfirst.to_vec()
                } else {
                    prest.to_vec()
                };
                if room >= 2 {
                    first.push(Span::styled(
                        format!("{}: ", clip_label(&label, room)),
                        self.theme.dim().add_modifier(Modifier::BOLD),
                    ));
                }
                let mut cont = prest.to_vec();
                cont.push(Span::raw("  "));
                let cell = row.get(c).cloned().unwrap_or_default();
                self.out
                    .extend(wrap::wrap_spans(cell, self.width, &first, &cont));
            }
        }
    }
}

/// Column widths that add up to `content`, shrinking the greediest columns
/// first and never below three cells.
fn fit(t: &TableAcc, cols: usize, content: usize) -> Vec<usize> {
    let mut widths = vec![0usize; cols];
    for row in std::iter::once(&t.header).chain(t.rows.iter()) {
        for (c, cell) in row.iter().enumerate().take(cols) {
            widths[c] = widths[c].max(spans_width(cell)).min(content);
        }
    }
    for w in &mut widths {
        *w = (*w).max(FLOOR_COL);
    }

    let total: usize = widths.iter().sum();
    if total <= content {
        return widths;
    }
    // Proportional first so one enormous column does not starve the rest, then
    // a bounded fix-up for the rounding. `content >= MIN_COL * cols` is the
    // caller's precondition, so the floor can always be honoured.
    for w in &mut widths {
        *w = (*w * content / total).max(FLOOR_COL);
    }
    while widths.iter().sum::<usize>() > content {
        let Some(widest) = widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > FLOOR_COL)
            .max_by_key(|(_, w)| **w)
            .map(|(i, _)| i)
        else {
            break;
        };
        widths[widest] -= 1;
    }
    widths
}

/// Front matter as the flat `key: value` pairs it usually is, or `None` if it
/// is anything else at all.
///
/// The bar here is faithfulness, not effort. Nested maps, lists, block scalars
/// and comments would all render as *something* if you squinted, and something
/// is the one outcome this must not produce — `mermaid` states the rule for
/// diagrams (the source is always true, and a diagram that has quietly lost a
/// node is worse than one nobody drew) and metadata is the same bargain, made
/// about a block whose entire content is data. So `None` is not a failure path.
/// It is the dim block this always used to be, showing the YAML exactly as the
/// author wrote it, and it is the answer to every case below.
///
/// The indent check is what does most of the work: `tags:` introducing a list,
/// `owner:` introducing a map and `notes: |` introducing a block scalar are all
/// followed by lines that begin with a space, and YAML has no way to continue a
/// value that does not.
fn front_matter(body: &str) -> Option<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for line in body.lines() {
        // A blank line is spacing in a block that is no longer being laid out
        // as a block. It carries no key and no value, so it is the one thing
        // dropped rather than declined over.
        if line.trim().is_empty() {
            continue;
        }
        // Indented: a continuation, a nested key or a list element. Leading
        // `-`: a top-level sequence. `#`: a comment, which is the author
        // talking and not a datum, and reformatting it is not on offer.
        if line.starts_with([' ', '\t', '#', '-']) {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        // Split at the *first* colon and keep the rest verbatim: `url:
        // https://x.dev` is one pair, not two, and the value is reproduced
        // rather than re-quoted.
        let (key, value) = (key.trim_end(), value.trim());
        // An empty value is `tags:` introducing something on the lines below —
        // which the indent check has already declined — or an explicit null,
        // which has no rendering here that is not an invention.
        if key.is_empty() || value.is_empty() {
            return None;
        }
        pairs.push((key.to_string(), value.to_string()));
    }
    // An empty block has no header to be, and falls through to drawing nothing
    // exactly as it did before.
    (!pairs.is_empty()).then_some(pairs)
}

/// A link's destination, cut down to what the pane can afford to say about it.
///
/// The parenthesised URL is a footnote on the sentence it hangs off, and a
/// footnote longer than the sentence has stopped being one: in a forty-six
/// column pane a URL carrying a query string routinely outruns the clause it
/// annotates, and the reader pays two wrapped rows for a string they cannot
/// click. So an over-budget absolute URL is reduced to its host. `github.com`
/// still answers the question the annotation exists for — *where does this go* —
/// which is what a reader deciding whether to follow it is actually reading.
///
/// Only a URL with an authority can lose anything, and that is the whole of the
/// rule. A relative link is not an address, it is a path inside this repository,
/// and `./plan.md` or `#invariants` *is* the useful half — there is no host to
/// fall back to and no part of it that is noise. `mailto:` the same: an address
/// is the information. Those are kept whole at any length, and `wrap` breaks an
/// over-long one rather than overflowing the pane.
///
/// Nothing is lost in either direction. `t` shows the source, where every
/// destination is written out in full and always has been.
fn elide_url(url: &str, width: usize) -> String {
    // The three cells are the ` (` and `)` the caller is about to wrap around
    // it: the budget is for the annotation, not for the string inside it.
    if url.width() + 3 <= width / URL_SHARE {
        return url.to_string();
    }
    let Some((_, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // `user@host:port`. Userinfo is the one part of an authority that does not
    // say *where*, so it goes with the path; the port stays, because
    // `localhost:3000` and `localhost` are not the same place.
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() {
        // `file:///plans/x.md` has an authority and it is empty, so there is
        // nothing shorter to say than the whole thing.
        return url.to_string();
    }
    host.to_string()
}

/// Cut a label down to `width` cells. Deliberately *not* `text::clip`: this is
/// where truncation goes unmarked, because at the widths a narrow-table label
/// or a front-matter key is squeezed into, the ellipsis is a measurable
/// fraction of the label itself.
fn clip_label(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    wrap::split_to_width(text, width)
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn align(spans: Vec<Span<'static>>, width: usize, how: Option<Alignment>) -> Vec<Span<'static>> {
    let have = spans_width(&spans);
    let pad = width.saturating_sub(have);
    match how {
        Some(Alignment::Right) => {
            let mut out = vec![Span::raw(" ".repeat(pad))];
            out.extend(spans);
            out
        }
        Some(Alignment::Center) => {
            let left = pad / 2;
            let mut out = vec![Span::raw(" ".repeat(left))];
            out.extend(spans);
            out.push(Span::raw(" ".repeat(pad - left)));
            out
        }
        _ => pad_to(spans, width),
    }
}

/// Apply a base style, keeping whatever the inline markup added on top.
fn restyle(spans: Vec<Span<'static>>, base: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|s| {
            let style = base.patch(s.style);
            Span::styled(s.content, style)
        })
        .collect()
}

fn restyle_bold(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|s| {
            let style = s.style.add_modifier(Modifier::BOLD);
            Span::styled(s.content, style)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::theme;
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn styles_of<'a>(line: &'a Line<'a>, needle: &str) -> Option<Style> {
        line.spans
            .iter()
            .find(|s| s.content.contains(needle))
            .map(|s| s.style)
    }

    /// The invariant every test below relies on: nothing the renderer produces
    /// may be wider than the pane it was rendered for.
    fn assert_fits(lines: &[Line<'_>], width: usize) {
        for line in lines {
            assert!(
                spans_width(&line.spans) <= width,
                "{:?} is {} cells, pane is {width}",
                text(std::slice::from_ref(line)),
                spans_width(&line.spans)
            );
        }
    }

    /// This used to assert the opposite, and the reasoning it asserted is still
    /// true: a terminal has no font size, so the level cannot be left to hue —
    /// `theme`'s rule, the one that underlines every link. What changed is the
    /// answer. The hashes were the last literal markdown syntax on a page that
    /// renders everything else, and they cost up to seven cells of a forty
    /// column pane to say something the *shape* of the block says for nothing.
    /// So the non-chromatic signal is now structural: a full-width rule under a
    /// title, a rule the width of its own text under a section, nothing under a
    /// sub-section, an indent below that. Read down a page in greyscale, those
    /// four are as distinct as they were with the hashes on.
    #[test]
    fn headings_are_marked_by_level_and_coloured() {
        let out = render("# One\n\n## Two\n", 40, Mode::Dark);
        assert_eq!(
            text(&out),
            ["One", &"━".repeat(40), "", "Two", "───"].map(String::from)
        );
        assert_eq!(
            styles_of(&out[0], "One").and_then(|s| s.fg),
            theme::DARK.heading(1).fg
        );
        assert_eq!(
            styles_of(&out[3], "Two").and_then(|s| s.fg),
            theme::DARK.heading(2).fg
        );
        // Dim, like every other piece of chrome: a rule is the pane talking
        // about the document rather than part of it.
        assert_eq!(
            styles_of(&out[1], "━").and_then(|s| s.fg),
            Some(theme::DARK.dim)
        );
        // The whole point of the change, stated as the assertion a tidy-up
        // would have to argue with.
        assert!(!text(&out).iter().any(|l| l.contains('#')));
    }

    #[test]
    fn a_sub_section_gets_no_rule_and_the_levels_under_it_get_an_indent() {
        // H4 and below share a colour — `Theme::heading` gives all of them
        // `warn` — so the indent is the only thing left saying which is which,
        // and it has to be a thing a reader who receives no hue can still see.
        let out = render(
            "### Three\n\n#### Four\n\n##### Five\n\n###### Six\n",
            40,
            Mode::Dark,
        );
        assert_eq!(
            text(&out),
            ["Three", "", "  Four", "", "    Five", "", "      Six"]
        );
        assert_eq!(
            styles_of(&out[0], "Three").and_then(|s| s.fg),
            theme::DARK.heading(3).fg
        );
        assert!(
            styles_of(&out[6], "Six")
                .unwrap()
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_cramped_pane_drops_the_rules_and_the_indent_before_it_drops_the_words() {
        // CRAMPED is twelve usable columns, the same threshold the code gutter
        // and the table grid already answer to. Under it the decoration is
        // spending cells the words needed, so a heading falls back to what it
        // has always been at that width: bold, coloured, and nothing else.
        let out = render("# One\n\n###### Six\n", 11, Mode::Dark);
        assert_eq!(text(&out), ["One", "", "Six"]);
        assert_fits(&out, 11);

        // One column more and the rule is affordable again. The indent is
        // clamped the way `prefixes` clamps a list indent, so it is still the
        // last thing to come back rather than the first.
        let out = render("# One\n\n###### Six\n", 12, Mode::Dark);
        assert_eq!(
            text(&out),
            ["One", &"━".repeat(12), "", "Six"].map(String::from)
        );
        assert_fits(&out, 12);
    }

    #[test]
    fn a_heading_rule_stays_behind_a_quote_gutter_and_a_list_marker() {
        // `prefixes` consumes the pending list marker, so the rule has to come
        // out of the same call the heading's own text did — and sit behind the
        // *continuation* prefix, which is where that text starts.
        let out = render("> # Quoted\n", 20, Mode::Dark);
        assert_eq!(
            text(&out),
            ["▏ Quoted".to_string(), format!("▏ {}", "━".repeat(18))]
        );
        assert_fits(&out, 20);

        // An H2's rule is the width of the heading, not of the pane, so inside
        // a list item it stops where the words did.
        let out = render("- ## Head\n", 20, Mode::Dark);
        assert_eq!(text(&out), ["• Head", "  ────"]);
        assert_fits(&out, 20);
    }

    #[test]
    fn a_wrapped_heading_starts_its_continuation_at_its_own_column() {
        // The hashes used to indent the continuation under the text by their
        // own width. With nothing to hang under, a wrapped heading simply keeps
        // its column.
        let out = render("# alpha beta gamma\n", 12, Mode::Dark);
        assert_eq!(
            text(&out),
            ["alpha beta", "gamma", &"━".repeat(12)].map(String::from)
        );
        assert_fits(&out, 12);

        // An H4 and below keeps its indent on every row it drew: a second line
        // that straightened up under the body text would be reading as a
        // paragraph from there on.
        let out = render("##### alpha beta gamma\n", 16, Mode::Dark);
        assert_eq!(text(&out), ["    alpha beta", "    gamma"]);
        assert_fits(&out, 16);
    }

    #[test]
    fn emphasis_becomes_a_modifier_not_asterisks() {
        let out = render("plain **bold** and *italic*", 40, Mode::Dark);
        assert_eq!(text(&out), ["plain bold and italic"]);
        assert!(
            styles_of(&out[0], "bold")
                .unwrap()
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            styles_of(&out[0], "italic")
                .unwrap()
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }

    #[test]
    fn inline_code_is_coloured_and_keeps_its_text() {
        let out = render("call `do_thing()` now", 40, Mode::Dark);
        assert_eq!(text(&out), ["call do_thing() now"]);
        assert_eq!(
            styles_of(&out[0], "do_thing").and_then(|s| s.fg),
            Some(theme::DARK.code)
        );
    }

    #[test]
    fn a_bullet_hangs_its_continuation_under_the_text() {
        let out = render("- alpha beta gamma delta epsilon", 16, Mode::Dark);
        assert_eq!(text(&out), ["• alpha beta", "  gamma delta", "  epsilon"]);
        assert_fits(&out, 16);
    }

    #[test]
    fn ordered_lists_count_and_nested_lists_indent() {
        // The nested bullet changes glyph as well as indent: once a list has
        // wrapped, the indent alone no longer says which level you are on.
        let out = render("1. one\n2. two\n   - deep\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["1. one", "2. two", "   ◦ deep"]);
    }

    #[test]
    fn task_list_markers_render_as_boxes() {
        let out = render("- [x] done\n- [ ] todo\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["• ✔ done", "• ☐ todo"]);
    }

    #[test]
    fn a_block_quote_draws_an_unbroken_gutter_including_blank_lines() {
        let out = render("> first para\n>\n> second para\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["▏ first para", "▏ ", "▏ second para"]);
    }

    #[test]
    fn a_gfm_alert_gets_a_label_because_the_parser_eats_the_marker() {
        let out = render("> [!WARNING]\n> mind the gap\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["▏ ⚠ Warning", "▏ mind the gap"]);
    }

    #[test]
    fn fenced_code_keeps_its_indentation_and_gets_a_gutter() {
        let out = render("```rust\nfn main() {\n    go();\n}\n```\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["│ fn main() {", "│     go();", "│ }"]);
        // Highlighted, not just reproduced: past the gutter there is colour.
        assert_eq!(out[0].spans[0].style.fg, Some(theme::DARK.dim));
        assert!(out[1].spans.iter().skip(1).any(|s| s.style.fg.is_some()));
    }

    #[test]
    fn a_wrapped_code_line_is_marked_as_a_continuation() {
        let out = render("```\nabcdefghijklmnopqrstuvwxyz\n```\n", 12, Mode::Dark);
        assert_eq!(text(&out), ["│ abcdefghij", "┆ klmnopqrst", "┆ uvwxyz"]);
        assert_fits(&out, 12);
    }

    #[test]
    fn a_link_shows_its_destination_unless_that_would_repeat_it() {
        let out = render("see [the docs](http://x.dev) ok", 60, Mode::Dark);
        assert_eq!(text(&out), ["see the docs (http://x.dev) ok"]);
        assert!(
            styles_of(&out[0], "docs")
                .unwrap()
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );

        // An autolink already *is* its destination.
        let out = render("<http://x.dev>", 60, Mode::Dark);
        assert_eq!(text(&out), ["http://x.dev"]);
    }

    #[test]
    fn a_destination_too_long_for_the_pane_is_reduced_to_its_host() {
        // Forty-six columns is a plausible right-hand pane, where the budget is
        // fifteen cells. The tracker URL is forty-one of them, so the reader
        // would pay two wrapped rows for a string they cannot click; the host
        // still answers the question the annotation exists for, which is where
        // this goes. `t` has the whole of it, as it always did.
        let out = render(
            "see [the tracker](https://github.com/o/a/issues/1?utm=x)",
            46,
            Mode::Dark,
        );
        assert_eq!(text(&out), ["see the tracker (github.com)"]);
        assert_fits(&out, 46);

        // Under budget at the same width, and kept whole. The threshold is a
        // share of the pane rather than a fixed length, so a wide pane elides
        // less — which is the behaviour a reader dragging the split expects.
        let out = render("see [docs](http://x.dev)", 46, Mode::Dark);
        assert_eq!(text(&out), ["see docs (http://x.dev)"]);
        let out = render("see [docs](http://x.dev)", 24, Mode::Dark);
        assert_eq!(text(&out), ["see docs (x.dev)"]);
    }

    #[test]
    fn a_relative_destination_is_kept_whole_however_long_it_is() {
        // A path in the repository is not an address, it *is* the information —
        // the thing the reader would open next — and there is no host under it
        // to fall back to. Both of these are over budget at this width and both
        // survive intact; only a URL with an authority can lose anything.
        let out = render(
            "[the plan](./plan.md), [why](#invariants), [mail](mailto:a@b.dev)",
            24,
            Mode::Dark,
        );
        let joined = text(&out).join(" ");
        for kept in ["(./plan.md)", "(#invariants)", "(mailto:a@b.dev)"] {
            assert!(joined.contains(kept), "{joined:?} lost {kept}");
        }
        assert_fits(&out, 24);
    }

    #[test]
    fn front_matter_becomes_a_header_of_keys_and_values() {
        // Still not parsed as a thematic break, which is what this test was
        // originally written to catch. What changed is what it is parsed *as*:
        // a dim block behind a `│ ` gutter was the loudest thing on every
        // agent-written plan before the reader reached a word of the plan, and
        // a gutter is a claim that the lines behind it are source.
        let out = render(
            "---\ntitle: The Plan\nstatus: draft\n---\n\n# Body\n",
            40,
            Mode::Dark,
        );
        assert_eq!(
            text(&out),
            [
                "title: The Plan",
                "status: draft",
                "",
                "Body",
                &"━".repeat(40)
            ]
            .map(String::from)
        );

        // Key bold, value not; both dim, because all of it is still metadata
        // about the document rather than a sentence of it.
        let key = styles_of(&out[0], "title:").expect("the key is one span");
        assert_eq!(key.fg, Some(theme::DARK.dim));
        assert!(key.add_modifier.contains(Modifier::BOLD));
        let value = styles_of(&out[0], "Plan").expect("the value is drawn");
        assert_eq!(value.fg, Some(theme::DARK.dim));
        assert!(!value.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn front_matter_that_is_not_flat_pairs_keeps_the_block_it_had() {
        // The rule `mermaid` states, applied to metadata: a nested map drawn as
        // if it were flat has quietly lost the nesting and the reader cannot
        // tell, which is worse than not drawing it. So one line this cannot
        // render faithfully declines the whole block back to the dim source.
        let out = render(
            "---\ntitle: x\ntags:\n  - a\n  - b\n---\n\n# Body\n",
            40,
            Mode::Dark,
        );
        assert_eq!(
            text(&out),
            [
                "│ title: x",
                "│ tags:",
                "│   - a",
                "│   - b",
                "",
                "Body",
                &"━".repeat(40),
            ]
            .map(String::from)
        );

        // A key with nothing after it is a map, a list or a null waiting on the
        // next line, and none of the three has a value this could draw.
        assert_eq!(
            text(&render("---\nnotes:\n---\n", 40, Mode::Dark)),
            ["│ notes:"]
        );
        // A comment is the author talking rather than a datum, and reformatting
        // it is not on offer.
        assert_eq!(
            text(&render(
                "---\n# owned by nobody\nk: v\n---\n",
                40,
                Mode::Dark
            )),
            ["│ # owned by nobody", "│ k: v"]
        );
    }

    #[test]
    fn a_front_matter_key_is_clipped_rather_than_allowed_to_overflow() {
        // A wrap *prefix* is the one thing `wrap_spans` will let run past the
        // pane, and a front-matter key is a prefix. The same trade, and the
        // same `clip_label`, the narrow-table records make.
        let out = render(
            "---\nimplementation_strategy: two passes\n---\n",
            18,
            Mode::Dark,
        );
        assert_eq!(text(&out), ["implementation_s: ", "  two passes"]);
        assert_fits(&out, 18);
    }

    #[test]
    fn an_image_becomes_a_glyph_and_its_alt_text() {
        // The brackets were three cells of punctuation announcing what one
        // glyph now says, and the alt text is the whole of what a terminal has
        // of the picture, so it is what is left standing. The destination is
        // not drawn at all: unlike a link there is nothing the reader could do
        // with it, and `t` has it.
        let out = render("![the pane at rest](docs/pane.png)", 40, Mode::Dark);
        assert_eq!(text(&out), ["▨ the pane at rest"]);
        assert_eq!(
            styles_of(&out[0], "▨").and_then(|s| s.fg),
            Some(theme::DARK.dim)
        );
        assert_eq!(
            styles_of(&out[0], "pane").and_then(|s| s.fg),
            Some(theme::DARK.dim)
        );

        // No alt text is the common case in an agent-written plan, and the
        // glyph alone still says a picture was here. The trailing space goes
        // with the wrap, which drops a space nothing follows.
        assert_eq!(text(&render("![](x.png)", 40, Mode::Dark)), ["▨"]);
    }

    #[test]
    fn a_table_is_aligned_when_it_fits_and_honours_the_alignment_row() {
        let out = render("| a | bb |\n| --- | ---: |\n| 1 | 2 |\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["a    │   bb", "─────┼─────", "1    │    2"]);
        assert_fits(&out, 40);
    }

    #[test]
    fn a_table_too_wide_for_the_pane_becomes_one_field_per_line() {
        // A grid of four-cell columns is not a grid, it is a puzzle.
        let md = "| name | description |\n| --- | --- |\n| alpha | the first one |\n";
        let out = render(md, 14, Mode::Dark);
        assert_eq!(
            text(&out),
            ["name: alpha", "description: ", "  the first", "  one"]
        );
        assert_fits(&out, 14);
    }

    #[test]
    fn a_rule_spans_the_pane_exactly() {
        let out = render("a\n\n---\n\nb", 10, Mode::Dark);
        assert_eq!(text(&out), ["a", "", "──────────", "", "b"]);
        assert_fits(&out, 10);
    }

    #[test]
    fn an_empty_document_renders_to_nothing_rather_than_a_blank_page() {
        assert!(render("", 40, Mode::Dark).is_empty());
        assert!(render("   \n\n  \n", 40, Mode::Dark).is_empty());
    }

    #[test]
    fn a_zero_width_pane_produces_no_rows_at_all() {
        // The pane guards this too, but a renderer that divides by the width
        // must not be the thing that finds out.
        assert!(render("# hello\n\n- a\n", 0, Mode::Dark).is_empty());
    }

    const KITCHEN_SINK: &str = "\
---
title: the sink
status: draft
---

# Heading here

## Second level

### Third level

###### Sixth level

![the pane at rest](docs/pane.png)

see [the tracker](https://github.com/o/a/issues/1?utm=x) and [the plan](./plan.md)

> quoted text that runs on
>
> > ## a heading two quotes deep

- item one
  - nested item
  - ## a heading inside a list

| a | b |
| --- | --- |
| 1 | 2 |

```rust
let x = 1;
```

```mermaid
graph TD
  A[one] --> B{two}
  B -->|yes| C[three]
```

tail";

    #[test]
    fn nothing_overflows_a_narrow_pane() {
        // Eight columns is where the last decoration gives up. Everything from
        // there to a full-window pane must fit exactly.
        //
        // The heading rules, the H4 indent, the front-matter header and an
        // elided host all add cells to a row *after* its words were measured,
        // which is the single way a renderer that pre-wraps gets a width wrong.
        // So the sweep runs over every one of them at once, and the fixture
        // above carries one of each: the widths a split pane actually lands on
        // — ten, twenty, forty, eighty — are all inside it, and the columns
        // either side of them are for the arithmetic.
        for width in 8..=80 {
            assert_fits(&render(KITCHEN_SINK, width, Mode::Dark), width);
        }
    }

    #[test]
    fn a_heading_in_a_pathological_position_still_fits_and_still_returns() {
        // Three quotes inside a list item is four prefixes deep before a word
        // is drawn, which is where the rule arithmetic runs out of pane first:
        // `avail` reaches zero long before the width does, and a rule sized off
        // a subtraction that went negative is the overflow this guards.
        let md = "- item\n\n  > > > # Buried\n  > > >\n  > > > ## Also buried\n";
        for width in 1..=48 {
            let out = render(md, width, Mode::Dark);
            assert!(!out.is_empty(), "width {width} rendered nothing");
            // Below eight nothing fits by construction — the quote gutters
            // alone are wider than the pane — and ratatui clips. See
            // `absurd_widths_produce_something_rather_than_a_panic`.
            if width >= 8 {
                assert_fits(&out, width);
            }
        }
        // A heading with no room for anything at all, which is the width the
        // `clamp` on an H2's rule would panic at if `roomy` ever stopped
        // guarding it.
        assert!(!render("# x", 3, Mode::Dark).is_empty());
        assert!(!render("###### deep", 1, Mode::Dark).is_empty());
        assert!(!render("---\nk: v\n---\n", 1, Mode::Dark).is_empty());
    }

    #[test]
    fn absurd_widths_produce_something_rather_than_a_panic() {
        // One to seven columns cannot hold a gutter and a character. ratatui
        // clips the overflow; what matters is that nothing here divides by the
        // width, loops on it, or indexes past it.
        for width in 1..8 {
            assert!(!render(KITCHEN_SINK, width, Mode::Dark).is_empty());
        }
    }

    const FLOWCHART: &str = "\
```mermaid
graph TD
  A[Watch the tree] --> B{Markdown?}
  B -->|yes| C[Render it]
  B -->|no| D[Refresh git]
```
";

    #[test]
    fn a_mermaid_fence_is_drawn_rather_than_reproduced() {
        // The whole point of the module: this fence used to arrive as four
        // lines of source behind a code gutter.
        let out = text(&render(FLOWCHART, 60, Mode::Dark));
        assert_eq!(
            out,
            [
                "     ┌─────────────────┐",
                "     │ Watch the tree  │",
                "     └────────┬────────┘",
                "              ▼",
                "        ╔═══════════╗",
                "        ║ Markdown? ║",
                "        ╚═════╤═════╝",
                "      ╭──yes──┤",
                "      │       ╰──no───╮",
                "      ▼               ▼",
                "┌───────────┐  ┌─────────────┐",
                "│ Render it │  │ Refresh git │",
                "└───────────┘  └─────────────┘",
            ]
        );
        // No code gutter, which the rows above say by where they start: a
        // gutter would have pushed every one of them two cells right. Not
        // asserted as "no line begins with `│ `" — a box's own left wall is
        // that exact string, which is the trap this note exists to mark.
        assert!(out.iter().any(|l| l.starts_with('┌')));
    }

    #[test]
    fn a_diagram_the_pane_is_too_narrow_for_becomes_an_outline() {
        // The same fallback tables already make: at this width the boxes would
        // be four cells of frame around two of text.
        let out = text(&render(FLOWCHART, 24, Mode::Dark));
        assert_eq!(out[0], "Watch the tree");
        assert_eq!(out[1], "└─▶ Markdown?");
        assert!(out.iter().any(|l| l.contains("yes")));
        assert!(out.iter().any(|l| l.contains("Refresh")));
        assert_fits(&render(FLOWCHART, 24, Mode::Dark), 24);
    }

    #[test]
    fn a_mermaid_fence_this_cannot_draw_keeps_the_code_block_it_had() {
        // Most of mermaid, by diagram type. Declining has to land exactly where
        // it always did — source, gutter and all — or the reader loses the
        // diagram rather than merely not seeing it drawn.
        let md = "```mermaid\npie title Votes\n  \"yes\" : 10\n```\n";
        assert_eq!(
            text(&render(md, 40, Mode::Dark)),
            ["│ pie title Votes", "│   \"yes\" : 10"]
        );
    }

    #[test]
    fn a_sequence_diagram_draws_its_lifelines() {
        let md = "\
```mermaid
sequenceDiagram
  participant W as Watcher
  participant V as Viewer
  W->>V: file changed
```
";
        assert_eq!(
            text(&render(md, 40, Mode::Dark)),
            [
                "┌─────────┐    ┌─────────┐",
                "│ Watcher │    │ Viewer  │",
                "└────┬────┘    └────┬────┘",
                "     │              │",
                "     │ file changed │",
                "     ├──────────────▶",
                "┌────┴────┐    ┌────┴────┐",
                "│ Watcher │    │ Viewer  │",
                "└─────────┘    └─────────┘",
            ]
        );
    }

    #[test]
    fn a_diagram_inside_a_list_item_indents_under_its_bullet() {
        // The reason `mermaid` returns rows and the prefixes stay here: a
        // diagram is a block like any other and hangs under its marker.
        let md = "- step:\n\n  ```mermaid\n  graph LR\n    a --> b\n  ```\n";
        let out = text(&render(md, 40, Mode::Dark));
        assert_eq!(out[0], "• step:");
        for line in &out[2..] {
            assert!(line.starts_with("  "), "{line:?} is not indented");
        }
    }

    #[test]
    fn a_document_that_is_all_structure_still_terminates() {
        // Pathological nesting in a narrow pane: the indent clamp is what
        // keeps `wrap` from being handed a prefix wider than the line.
        let md = "- a\n  - b\n    - c\n      - d\n        - e\n          - f\n";
        let out = render(md, 8, Mode::Dark);
        assert_fits(&out, 8);
        assert_eq!(out.len(), 6);
    }
}

