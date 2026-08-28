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
    Alignment, BlockQuoteKind, CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag,
    TagEnd,
};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::outline::Entry;
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
///
/// `mermaid` runs its own dim `─`/`┄`/`━` opposition, and states the rule that
/// a reader looking at one document containing both families must not be able
/// to tell who drew what. Two dim oppositions on one page is safe here because
/// they never share a row: a heading rule is a block of its own with nothing
/// attached to either end, and every `━` a diagram draws is a connector between
/// two boxes, inside a drawing the eye has already framed. Both are `dim`,
/// which is the part that has to agree, and does — a heading rule is the pane
/// talking about the document exactly as a lifeline is. What would break the
/// arrangement is a heading rule drawn in anything but `dim`, or a diagram
/// promoted to full-width horizontals; neither is on offer.
const H1_RULE: &str = "━";
/// The rule under an H2, which is light but stops strictly short of the pane.
///
/// A thematic break is a full-width `─` at the same prefix and in the same
/// `dim`, so extent is the whole of what separates the two — and a *wrapping*
/// H2 is the normal case in a forty-column pane, which is what makes the
/// measurement below (widest drawn row) reach the edge. The invariant the code
/// holds is therefore stronger than "short": an H2's rule is capped at one cell
/// under `avail`, so it can never be the width `Event::Rule` would draw at that
/// prefix. Stroke separates it from an H1's `━`; that cell separates it from a
/// break.
const H2_RULE: &str = "─";
/// The mark on an H4, H5 and H6 — the whole of what tells those three apart
/// without hue, since `Theme::heading` deliberately gives all of them `warn`.
///
/// This replaced a per-level indent, which failed in two ways that a pip cannot.
/// The indent collided with ordinary body indent: `- item` followed by
/// `### Inside a list` drew the same two leading spaces a top-level `#### Four`
/// did, so two different levels rendered as structurally identical rows and only
/// the colour was left. And it was clamped to a cell count rather than to a
/// whole number of steps, so anywhere in 13..=17 usable columns two or three of
/// the levels collapsed onto the same indent while `roomy` was still drawing the
/// rules above them. A pip is a fixed width added on top of whatever the
/// heading is nested inside, so nesting shifts a level and its neighbours by the
/// same amount and the mark survives.
///
/// Solid, hollow, hollow-and-square: the same progression `next_marker` makes
/// with `• ◦ ▪` going down list levels, and pointedly not those three glyphs, so
/// a sub-section is never read as a list item. If these are ever changed, the
/// property to keep is that the three differ in *shape* — a reader who receives
/// no hue has nothing else here — and that each is one cell wide, since two
/// would be the wrap arithmetic in this file going wrong on someone else's
/// terminal.
const HEADING_PIPS: [&str; 3] = ["▸ ", "▹ ", "▫ "];
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

/// The pip an H4, H5 or H6 is drawn with, or `None` for H1..H3, which carry
/// their level in the rule under them instead.
///
/// Reached from `theme`'s tests as well as from the heading arm: `Theme::heading`
/// hands H4, H5 and H6 one colour on purpose, and the assertion that this is
/// still safe has to be able to see the thing that makes it safe.
pub(super) fn heading_pip(level: usize) -> Option<&'static str> {
    level
        .checked_sub(4)
        .and_then(|i| HEADING_PIPS.get(i).copied())
}

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

/// The rows, for every caller that only wants rows.
///
/// A wrapper over [`render_outlined`] rather than a second walk of the parser,
/// so there is exactly one state machine and no way for the rows a reader sees
/// and the rows an outline points at to have come from different runs of it.
/// `super::Page::doc` and `crate::panes::ask` are the callers that genuinely
/// have nowhere to put an outline — a doc block is a fragment of a file whose
/// structure the file's own scanner already describes, and the ask pane has no
/// outline view — and neither should have to say so at the call site.
pub fn render(source: &str, width: usize, mode: Mode) -> Vec<Line<'static>> {
    render_outlined(source, width, mode).0
}

/// The rows, and where each heading landed among them.
///
/// The entries are recorded **by the renderer, as it emits the heading**, which
/// is the only moment the row is knowable: how many rows a document has by the
/// time it reaches its third `##` depends on how every paragraph before it
/// wrapped, so anything that worked the answer out afterwards would be
/// re-deriving the layout from the layout. See [`super::outline`] for what an
/// entry's `row` is worth and for how long.
pub(super) fn render_outlined(
    source: &str,
    width: usize,
    mode: Mode,
) -> (Vec<Line<'static>>, Vec<Entry>) {
    if width == 0 {
        return (Vec::new(), Vec::new());
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
    /// Shown as dim source rather than highlighted. An HTML block, because it
    /// is the document's own plumbing rather than something the author was
    /// writing to be read — and a metadata block in a flavour this cannot read
    /// as pairs, which is every flavour but YAML.
    Plain,
    /// YAML front matter, which is tried as a key/value header first and falls
    /// back to the same dim block as `Plain`. See [`front_matter`].
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
    /// Where each heading landed in `out`, recorded as it was emitted. See
    /// [`render_outlined`].
    outline: Vec<Entry>,

    /// Inline spans of the block being built, not yet wrapped.
    spans: Vec<Span<'static>>,
    style: Style,
    styles: Vec<Style>,
    /// `(index into spans where the link text started, destination)`.
    link: Option<(usize, String)>,
    /// The same for an image's alt text, held separately because an image
    /// inside a link is legal and one field would lose the outer one.
    image: Option<(usize, String)>,

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
            outline: Vec::new(),
            spans: Vec::new(),
            style: Style::default(),
            styles: Vec::new(),
            link: None,
            image: None,
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

    fn finish(mut self) -> (Vec<Line<'static>>, Vec<Entry>) {
        self.flush_inline();
        (self.out, self.outline)
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
                self.code = Some((String::new(), String::new(), Raw::Plain));
            }
            // Only the YAML flavour is read as pairs. `front_matter` is a YAML
            // predicate — `key: value`, `#` for a comment, `|` for a block
            // scalar — and TOML answers none of those the same way: a
            // `+++`-delimited `date = 2024-01-01T00:00:00` would split at the
            // colon into a key of `date = 2024-01-01T00`, which is an invented
            // field drawn as if the author had written it. The pluses option is
            // not in `options()` today, so this arm is unreachable; a wildcard
            // here is a trap that springs the day somebody turns it on.
            Tag::MetadataBlock(kind) => {
                self.start_block();
                let raw = match kind {
                    MetadataBlockKind::YamlStyle => Raw::Meta,
                    MetadataBlockKind::PlusesStyle => Raw::Plain,
                };
                self.code = Some((String::new(), String::new(), raw));
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
            // not on the page rather than a sentence of the document; when
            // there *is* an alt the destination is not shown, since unlike a
            // link there is nothing the reader could do with it here.
            //
            // The destination is kept anyway, because an empty alt is the
            // common case in an agent-written plan and a bare glyph says a
            // picture was here without saying which. `TagEnd::Image` is the
            // first moment that is knowable, so the start index is recorded the
            // way a link's is. Not the same field: `[![alt](img)](url)` is legal
            // and one `Option` would lose the outer link.
            Tag::Image { dest_url, .. } => {
                self.spans.push(Span::styled(IMAGE_GLYPH, self.theme.dim()));
                self.image = Some((self.spans.len(), dest_url.to_string()));
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
            // but that something is now structural: a full-width rule under the
            // title, a shorter rule under a section, nothing under a
            // sub-section, and a pip on the three below that. Read down a page
            // in greyscale, all six are as distinct as they are in colour.
            //
            // A pip rather than an indent, which is what this first shipped as
            // and what a reviewer broke twice over — see `HEADING_PIPS`. The
            // short of it: an indent is not the heading's own, because whatever
            // the heading is nested in has already spent some, and it cannot be
            // paid in whole steps at every width.
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
                //
                // One boolean governs the rules *and* the pip, so the invariant
                // is statable in one line: at CRAMPED or wider every level is
                // told apart by something that is not hue, and under it no
                // level is, because the words have taken the cells back. The
                // per-level arithmetic that used to sit here is what made that
                // sentence false across a band of widths.
                let roomy = avail >= CRAMPED;

                // The pip goes on *both* prefixes — a wrapped H5 whose second
                // line straightened up under the body text would be reading as
                // a paragraph from there on — and the continuation pays for it
                // in blanks, exactly as the hashes used to.
                if roomy && let Some(pip) = heading_pip(level) {
                    first.push(Span::styled(pip, self.theme.dim()));
                    rest.push(Span::raw(" ".repeat(pip.width())));
                }

                let content = std::mem::take(&mut self.spans);
                let content = restyle(content, self.theme.heading(level as u8));

                // **Recorded here, and here is the only place it can be.**
                // `self.out.len()` is the row this heading's first line is
                // about to become, which is knowable at this instant and at no
                // other: how many rows the document has by now depends on how
                // every paragraph above it wrapped.
                //
                // Before the rows *and* before the rule, so a jump lands on the
                // words rather than on the `━` under them — a reader taken to a
                // heading's underline sees the section above it filling the
                // pane and the heading itself off the top edge, which is the
                // one landing this feature exists to avoid.
                //
                // The text is taken from `content` rather than from the drawn
                // rows, because those carry the quote gutter, the list marker
                // and the pip that whatever this heading is nested in has
                // already paid for. Those are the pane talking about the
                // document; the outline is a list of the document's own names.
                //
                // A heading with no text at all — `#` on its own, which is
                // legal — is left out. It is the same judgement the rule below
                // makes two dozen lines down: there is nothing to draw and
                // nothing to name, and a blank row in a jump list is a target
                // the reader cannot choose between.
                let text: String = content.iter().map(|s| s.content.as_ref()).collect();
                let text = text.trim();
                if !text.is_empty() {
                    self.outline.push(Entry {
                        row: self.out.len(),
                        level: level as u8,
                        text: text.to_string(),
                    });
                }

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

                // `#` on its own is a heading with no text, and `wrap_spans`
                // still returns the one row every block gets. Ruling under it
                // would draw precisely the "title that renders as a horizontal
                // rule" `H1_RULE` calls the worse lie — with not even a title
                // above it. Nothing was drawn when the widest row is exactly
                // the prefix every row starts with.
                let drew_something = drawn > spans_width(&rest);
                let rule = match level {
                    1 if roomy && drew_something => Some((H1_RULE, avail)),
                    // Only as wide as the heading, so a short H2 is visibly not
                    // a break and a long one is visibly not an H1 — and capped
                    // one cell under `avail`, because a wrapping H2 measured
                    // from its widest drawn row otherwise reaches the pane edge
                    // and draws a thematic break. `clamp`'s bounds cannot
                    // invert: `roomy` has already put `avail` at CRAMPED.
                    2 if roomy && drew_something => Some((
                        H2_RULE,
                        drawn.saturating_sub(spans_width(&rest)).clamp(1, avail - 1),
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
                        let shown = elide_url(&url, self.avail());
                        self.spans
                            .push(Span::styled(format!(" ({shown})"), self.theme.dim()));
                    }
                }
            }
            TagEnd::Image => {
                self.pop_style();
                // An image with no alt is a glyph and nothing else, which says
                // a picture was here without saying which one — and in a file
                // viewer, over a repository, `docs/pane.png` is a path the
                // reader can go and open. So the destination stands in for the
                // alt when there is no alt, through the same elision a link's
                // does. When there *is* one, it still is not drawn: the alt is
                // the better answer and two of them is noise.
                if let Some((start, dest)) = self.image.take() {
                    let alt: String = self.spans[start.min(self.spans.len())..]
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    if alt.trim().is_empty() && !dest.is_empty() {
                        let shown = elide_url(&dest, self.avail());
                        self.spans.push(Span::styled(shown, self.theme.dim()));
                    }
                }
            }

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

    /// Columns the block being built has for its own text, worked out without
    /// consuming the pending list marker the way [`Self::prefixes`] does.
    ///
    /// Every block in this file measures its decoration off a number like this
    /// one — `emit_code`, `emit_table`, the heading arm, which says in its own
    /// comment why it takes the continuation prefix — and an inline elision has
    /// to answer to the same one, or a link six cells inside a nested quote is
    /// budgeted as though it started at column zero. It cannot go through
    /// `prefixes`: that spends the marker, and an inline event routinely
    /// arrives while the marker is still owed to a row that has not been built.
    ///
    /// The list indent is read un-clamped, so this can come out *narrower* than
    /// the prefix eventually drawn — `prefixes` caps the indent so nesting
    /// cannot eat the pane. That is the safe direction: it elides a little
    /// sooner in pathological nesting and never budgets for room the row turns
    /// out not to have.
    fn avail(&self) -> usize {
        self.width
            .saturating_sub(self.indent + QUOTE_GUTTER.width() * self.quote_depth)
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
        //
        // It takes `avail` for the same reason `mermaid::render` does. The
        // bargain is not only about *what* the block says but about whether it
        // fits: a width-blind predicate cannot decline what it cannot draw, and
        // the loss would land downstream in `emit_front_matter`, where a key
        // clipped to the pane is unmarked — two fields sharing a prefix render
        // under one identical invented name and the reader cannot tell.
        if kind == Raw::Meta
            && let Some(pairs) = front_matter(body, avail)
        {
            return self.emit_front_matter(pairs, &pfirst, &prest);
        }

        let rows = match kind {
            Raw::Code => source::highlight_code(body, lang, self.mode),
            Raw::Plain | Raw::Meta => source::plain(body, self.theme.dim()),
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

    /// One `label: value` row, hung by two.
    ///
    /// Front matter and a table too narrow for its grid draw the same shape,
    /// and used to draw it twice: the same `room`, the same `clip_label`, the
    /// same two-cell hang, down to a paraphrase of the same comment. Two copies
    /// of a width calculation is two answers the day one of them is corrected —
    /// which is exactly what happened, since only one of them had a caller that
    /// could decline.
    ///
    /// The label is clipped rather than allowed to run, because it is a wrap
    /// *prefix* and a prefix is the one thing `wrap_spans` will let overflow the
    /// pane — a forty-cell `implementation_strategy: ` at eighteen columns is
    /// not a hypothetical in this codebase. Clipping is unmarked, so it is a
    /// loss the reader is not told about; `front_matter` therefore declines the
    /// whole block before it can happen, and the table records, whose header
    /// cells the reader can still find in the source, accept it. That asymmetry
    /// is the reason this helper takes the label already chosen rather than
    /// deciding anything.
    fn emit_record(
        &mut self,
        label: &str,
        value: Vec<Span<'static>>,
        label_style: Style,
        mut first: Vec<Span<'static>>,
        prest: &[Span<'static>],
    ) {
        // Below two cells there is no room for a label and its colon, so the
        // value goes out bare. Ugly, and it is still the data.
        let room = label_room(self.width.saturating_sub(spans_width(prest)));
        if room >= 2 {
            first.push(Span::styled(
                format!("{}: ", clip_label(label, room)),
                label_style,
            ));
        }
        // A wrapped value that came back to column zero would read as the next
        // label.
        let mut cont = prest.to_vec();
        cont.push(Span::raw("  "));
        self.out
            .extend(wrap::wrap_spans(value, self.width, &first, &cont));
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
    /// Every key here is already known to fit: `front_matter` was given the same
    /// `avail` and declined the block otherwise. So `emit_record`'s clip never
    /// fires on this path, which is what lets the header be read as a header.
    fn emit_front_matter(
        &mut self,
        pairs: Vec<(String, String)>,
        pfirst: &[Span<'static>],
        prest: &[Span<'static>],
    ) {
        let key_style = self.theme.dim().add_modifier(Modifier::BOLD);
        for (i, (key, value)) in pairs.into_iter().enumerate() {
            let first = if i == 0 {
                pfirst.to_vec()
            } else {
                prest.to_vec()
            };
            let value = vec![Span::styled(value, self.theme.dim())];
            self.emit_record(&key, value, key_style, first, prest);
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
    ///
    /// Unlike front matter, this accepts a clipped label rather than declining:
    /// a table has no faithful fallback to decline *to* — the grid is the thing
    /// that did not fit — and a header cell is one word of a document the reader
    /// can read whole with `t`, not the name of the datum beside it.
    fn emit_table_as_records(
        &mut self,
        t: TableAcc,
        cols: usize,
        pfirst: &[Span<'static>],
        prest: &[Span<'static>],
    ) {
        let label_style = self.theme.dim().add_modifier(Modifier::BOLD);
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

                let first = if first_line {
                    first_line = false;
                    pfirst.to_vec()
                } else {
                    prest.to_vec()
                };
                let cell = row.get(c).cloned().unwrap_or_default();
                self.emit_record(&label, cell, label_style, first, prest);
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

/// Cells a `label: ` may spend on the label itself, given the columns the block
/// has. The two are the `: ` that follows it.
///
/// One function because `front_matter` decides whether a key fits and
/// `emit_record` decides what to do about it, and two copies of that
/// subtraction is two answers to the same question the first time one is
/// corrected.
fn label_room(avail: usize) -> usize {
    avail.saturating_sub(2)
}

/// Front matter as the flat `key: value` pairs it usually is, or `None` if it
/// is anything else at all — or anything that will not fit in `avail` columns.
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
/// The width is part of the bargain, not a separate concern, which is why it is
/// an argument here rather than a clip downstream. `mermaid::render` declines a
/// diagram it cannot *fit* as readily as one it cannot draw; a predicate with no
/// width could only make half that promise, and the other half became an
/// unmarked truncation in `emit_record` — `implementation_strategy` and
/// `implementation_status` at nineteen columns both clip to
/// `implementation_st`, which is one invented key drawn over two different
/// fields. Declining is content-aware in a way a blanket `CRAMPED` gate is not:
/// `k: v` still renders at four columns, and a forty-cell key declines at
/// eighty.
///
/// The indent check is what does most of the rest: `tags:` introducing a list,
/// `owner:` introducing a map and `notes: |` introducing a block scalar are all
/// followed by lines that begin with a space, and YAML has no way to continue a
/// value that does not.
fn front_matter(body: &str, avail: usize) -> Option<Vec<(String, String)>> {
    let room = label_room(avail);
    // Under two cells `emit_record` drops the key entirely and draws the value
    // alone, which is a field with its name silently removed. At those widths
    // the source block is both narrower and complete.
    if room < 2 {
        return None;
    }

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
        // The colon has to be *followed by a space*, which is what makes a
        // mapping a mapping in YAML. `url:https://x.dev` is a plain scalar and
        // splitting it invents a pair out of a sentence; `"a:b": v` has its key
        // in quotes and splitting at the first colon cuts it in half. Taking
        // the first `": "` instead gets both right, and `notes:` with nothing
        // after it finds no match at all — which is the decline it wanted
        // anyway, since a key alone is a map, a list or a null on the lines
        // below.
        let (key, value) = line.split_once(": ")?;
        // Keep the rest verbatim past that: `url: https://x.dev` is one pair,
        // not two, and the value is reproduced rather than re-quoted.
        let (key, value) = (key.trim_end(), value.trim());
        // An explicit empty value has no rendering here that is not an
        // invention.
        if key.is_empty() || value.is_empty() {
            return None;
        }
        // `notes: |`, `text: >-`, `blob: |2+`. A block scalar's body is on the
        // lines below — which the indent check declines — so what is left here
        // is the indicator itself, and drawing it as the value puts a piece of
        // YAML syntax on the page dressed as a datum.
        if is_block_scalar(value) {
            return None;
        }
        // ` #` opens a comment in a plain scalar, and YAML drops everything
        // after it: `title: Plan # a note` is the value `Plan`. Drawing
        // `Plan # a note` is the author's aside promoted to data, which is the
        // rule the leading-`#` check one branch up already states. A `#` inside
        // quotes is a datum and is declined too — telling the two apart is a
        // YAML parser, and this is a predicate that would rather show the
        // source than be one.
        if value.contains(" #") {
            return None;
        }
        // The value is about to go through `wrap_spans`, which collapses runs
        // of whitespace — so `a: one    two` would render `one two` and the
        // reader would have no way to know. `hard_wrap` would keep the run, and
        // was rejected: it breaks at the column rather than at a space, so
        // every ordinary title would start splitting mid-word to protect a case
        // that hardly happens. Declining keeps the run exactly, in the source
        // block, and costs the header only when the header would have lied.
        if value.contains('\t') || value.contains("  ") {
            return None;
        }
        // The key is a wrap *prefix*, and `emit_record` clips a prefix that
        // does not fit without marking the cut. See the note above.
        if key.width() > room {
            return None;
        }
        pairs.push((key.to_string(), value.to_string()));
    }
    // An empty block has no header to be, and falls through to drawing nothing
    // exactly as it did before.
    (!pairs.is_empty()).then_some(pairs)
}

/// Whether a value is a YAML block scalar indicator rather than a value:
/// `|`, `>`, and either with a chomping (`-`, `+`) or indentation (a digit)
/// modifier hung off it.
fn is_block_scalar(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('|' | '>')) && chars.all(|c| matches!(c, '-' | '+' | '0'..='9'))
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
/// over-long one rather than overflowing the pane. What makes something an
/// address is a *scheme* in front of the `://`, spelled the way RFC 3986 spells
/// one; `./weird://path/to/a/file` is a filename with a colon in it, and reading
/// `weird` as a scheme would cut a relative path down to `path`.
///
/// **Something is lost when a URL is cut, and it is marked.** An elided
/// destination ends in `/…`, so `(github.com/…)` and `(github.com)` are two
/// different answers — the first a host standing in for an address that
/// continued, the second an address that stopped there. Without the mark those
/// were one string, and a reader had no way to know which. The slash is the
/// authority's own delimiter: everything this drops comes after it, whether the
/// URL spelled it with a `/`, a `?` or a `#`, and userinfo counts as dropped
/// too — `https://user:pw@h.io/a/b` reduced to a bare `(h.io)` is a
/// credentialled URL wearing an ordinary face.
///
/// What is *not* recoverable from the annotation is the scheme: `http://x.dev`
/// and `https://x.dev` both elide to `(x.dev)`, because eight cells of `https://`
/// is most of a fifteen-cell budget. That is the one thing given up here, and
/// `t` has it, along with every destination written out in full.
///
/// `text::clip`'s argument for an unmarked cut — the ellipsis is a measurable
/// fraction of the string — does not reach this far. It is one cell against a
/// budget of about fifteen, and it is buying back a distinction the reader
/// otherwise cannot make at all.
fn elide_url(url: &str, width: usize) -> String {
    // The three cells are the ` (` and `)` the caller is about to wrap around
    // it: the budget is for the annotation, not for the string inside it.
    if url.width() + 3 <= width / URL_SHARE {
        return url.to_string();
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    if !is_scheme(scheme) {
        return url.to_string();
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    // `user@host:port`. Userinfo is the one part of an authority that does not
    // say *where*, so it goes with the path; the port stays, because
    // `localhost:3000` and `localhost` are not the same place.
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() {
        // `file:///plans/x.md` has an authority and it is empty, so there is
        // nothing shorter to say than the whole thing.
        return url.to_string();
    }
    // A bare `https://h.io` and a trailing slash on one say the same thing, so
    // neither is a cut; anything past that is, and so is a `user@` this just
    // dropped in front of the host.
    let cut = !matches!(tail, "" | "/") || authority.contains('@');
    if cut {
        format!("{host}/…")
    } else {
        host.to_string()
    }
}

/// Whether `s` is a URL scheme as RFC 3986 spells one: a letter, then letters,
/// digits, `+`, `-` and `.`. Anything else in front of a `://` is part of a
/// path that happens to contain one.
fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Cut a label down to `width` cells. Deliberately *not* `text::clip`: this is
/// where truncation goes unmarked, because at the widths a narrow-table label
/// is squeezed into, the ellipsis is a measurable fraction of the label itself.
/// The other caller, `front_matter`, declines the block rather than reach here
/// at all — see its note on why an unmarked cut is not on offer for a key.
///
/// Never wider than asked, which is not free: `split_to_width` always emits at
/// least one character, so a two-cell ideograph comes back from a one-cell
/// budget. Nothing reaches that today — both callers hold `width` at two or
/// more, and no single character is three cells — but a helper whose whole job
/// is a width is the wrong place to leave a contract that only happens to hold.
fn clip_label(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let cut = wrap::split_to_width(text, width)
        .into_iter()
        .next()
        .unwrap_or_default();
    if cut.width() > width {
        String::new()
    } else {
        cut
    }
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
    /// sub-section, a pip on the three below that. Read down a page in
    /// greyscale, all six are as distinct as they were with the hashes on.
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
    fn a_sub_section_gets_no_rule_and_the_levels_under_it_get_a_pip() {
        // H4 and below share a colour — `Theme::heading` gives all of them
        // `warn` — so the pip is the only thing left saying which is which,
        // and it has to be a thing a reader who receives no hue can still see.
        let out = render(
            "### Three\n\n#### Four\n\n##### Five\n\n###### Six\n",
            40,
            Mode::Dark,
        );
        assert_eq!(
            text(&out),
            ["Three", "", "▸ Four", "", "▹ Five", "", "▫ Six"]
        );
        assert_eq!(
            styles_of(&out[0], "Three").and_then(|s| s.fg),
            theme::DARK.heading(3).fg
        );
        // The pip is chrome and the words are the document, so they are not the
        // same colour: the pip is `dim` like every rule and gutter, and the
        // text keeps the heading colour and the bold.
        assert_eq!(
            styles_of(&out[2], "▸").and_then(|s| s.fg),
            Some(theme::DARK.dim)
        );
        assert!(
            styles_of(&out[6], "Six")
                .unwrap()
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    /// The failure the pips were written for. An indent is not the heading's
    /// own — whatever it is nested in has already spent some — so `### Inside a
    /// list` inside an item and `#### Four` at the top level drew the same two
    /// leading spaces, in bold, with no rule, and differed in hue and in
    /// nothing else. A pip is added on top of the nesting rather than confused
    /// with it.
    #[test]
    fn a_heading_nested_in_a_list_is_still_told_apart_from_a_deeper_one() {
        let nested = text(&render("- item\n\n  ### Inside a list\n", 40, Mode::Dark));
        let deeper = text(&render("#### Four at top level\n", 40, Mode::Dark));
        assert_eq!(nested, ["• item", "", "  Inside a list"]);
        assert_eq!(deeper, ["▸ Four at top level"]);
        // Stated as the property rather than as the fixture: strip the words
        // and what is left has to differ.
        assert_ne!(
            nested[2].replace("Inside a list", ""),
            deeper[0].replace("Four at top level", "")
        );
    }

    /// The other way the indent failed: it was clamped to a cell count rather
    /// than to a whole number of steps, so across 13..=17 usable columns two or
    /// three levels landed on the same indent — while `roomy` was still drawing
    /// the H1 rule and claiming decoration was affordable. Every width from
    /// where decoration starts to a full window, six levels, all six distinct.
    #[test]
    fn no_two_heading_levels_render_the_same_at_any_width() {
        // Compared as *text*, which is the whole point: this is what a reader
        // who receives no hue is looking at. Compared as whole blocks too,
        // because a rule is a row of its own and an H1, an H2 and an H3 all
        // draw the same first row.
        for width in CRAMPED..=80 {
            let shapes: Vec<Vec<String>> = (1..=6)
                .map(|l| {
                    let md = format!("{} a\n", "#".repeat(l));
                    text(&render(&md, width, Mode::Dark))
                })
                .collect();
            for (i, a) in shapes.iter().enumerate() {
                for (j, b) in shapes.iter().enumerate().skip(i + 1) {
                    assert_ne!(
                        a,
                        b,
                        "H{} and H{} are identical at width {width}",
                        i + 1,
                        j + 1
                    );
                }
            }
        }
    }

    /// The pip is a wrap *prefix*, so the words pay for it and the row does
    /// not grow — which is the one way a renderer that pre-wraps gets a width
    /// wrong, and the reason the indent it replaced needed a clamp at all.
    /// Every level that has one, in every nesting this file draws.
    #[test]
    fn a_pip_is_paid_for_out_of_the_words_and_not_added_to_the_row() {
        for md in [
            "###### Sixth level heading here\n",
            "##### alpha\n",
            "> ##### under a quote gutter\n",
            "- #### inside a list item\n",
            "- item\n\n  > > ###### two quotes inside an item\n",
        ] {
            // From eight, where the last decoration gives up. Under it the
            // quote gutters alone outrun the pane and `wrap` never clips a
            // prefix — pre-existing, and not the pip's doing.
            for width in 8..=48 {
                assert_fits(&render(md, width, Mode::Dark), width);
            }
        }
    }

    #[test]
    fn a_cramped_pane_drops_the_rules_and_the_pip_before_it_drops_the_words() {
        // CRAMPED is twelve usable columns, the same threshold the code gutter
        // and the table grid already answer to. Under it the decoration is
        // spending cells the words needed, so a heading falls back to what it
        // has always been at that width: bold, coloured, and nothing else.
        //
        // One boolean governs the rules and the pip together, which is what
        // makes that statable: at CRAMPED and above every level is told apart
        // by something other than hue, and below it none of them is. The
        // arithmetic this replaced held neither half across 13..=17.
        let out = render("# One\n\n###### Six\n", 11, Mode::Dark);
        assert_eq!(text(&out), ["One", "", "Six"]);
        assert_fits(&out, 11);

        let out = render("# One\n\n###### Six\n", 12, Mode::Dark);
        assert_eq!(
            text(&out),
            ["One", &"━".repeat(12), "", "▫ Six"].map(String::from)
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

        // An H4 and below pays for its pip on every row it drew: a second line
        // that straightened up under the body text would be reading as a
        // paragraph from there on. The pip itself appears once — it is a
        // marker, not a gutter — and the continuation pays in blanks, exactly
        // as the hashes used to.
        let out = render("##### alpha beta gamma\n", 16, Mode::Dark);
        assert_eq!(text(&out), ["▹ alpha beta", "  gamma"]);
        assert_fits(&out, 16);
    }

    /// A wrapping H2 is the normal case in a pane this wide, and its rule is
    /// measured from the widest row it actually drew — which reaches the pane
    /// edge, where a `─` in `dim` at that prefix is exactly what `Event::Rule`
    /// draws. Widest-drawn is still the right measurement (it beats first-row
    /// and last-row, both of which under-report a wrapped heading); the cap is
    /// what keeps it from colliding.
    #[test]
    fn an_h2_rule_is_never_the_width_a_thematic_break_would_draw() {
        // Nineteen columns and four four-letter words: the heading fits on one
        // row of exactly nineteen cells, so an uncapped rule would be nineteen
        // too, and a thematic break here is nineteen.
        let out = render("## aaaa bbbb cccc dddd\n", 19, Mode::Dark);
        assert_eq!(text(&out)[0].width(), 19, "the fixture stopped wrapping");
        let rule = &text(&out)[1];
        assert_eq!(rule.width(), 18);

        let brk = render("a\n\n---\n", 19, Mode::Dark);
        let brk = &text(&brk)[2];
        assert_eq!(brk.width(), 19);
        assert!(rule.width() < brk.width(), "an H2 rule is a thematic break");

        // And behind a prefix, where both are measured off the same `avail`.
        let out = render("> ## aaaa bbbb cccc\n", 19, Mode::Dark);
        let rule = text(&out).pop().expect("a rule was drawn");
        let brk = render("> a\n>\n> ---\n", 19, Mode::Dark);
        let brk = text(&brk).pop().expect("a break was drawn");
        assert!(rule.width() < brk.width(), "{rule:?} is as wide as {brk:?}");
    }

    /// `#` on its own is a legal ATX heading with no text, and it used to draw
    /// a blank row and then a full-width `━` — the "title that renders as a
    /// horizontal rule" `H1_RULE`'s own comment calls the worse lie, with not
    /// even a title above it.
    #[test]
    fn an_empty_heading_draws_no_rule() {
        assert_eq!(text(&render("#\n", 40, Mode::Dark)), [""]);
        assert_eq!(text(&render("##\n", 40, Mode::Dark)), [""]);
        // The prefix is not content: a heading with no text inside a quote has
        // still drawn nothing, however many cells its gutter took.
        assert_eq!(text(&render("> #\n", 40, Mode::Dark)), ["▏ "]);
        // A heading that *did* draw something still gets its rule.
        assert_eq!(
            text(&render("# x\n", 40, Mode::Dark)),
            ["x".to_string(), "━".repeat(40)]
        );
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
        // fifteen cells. The tracker URL is thirty-seven of them, so the reader
        // would pay two wrapped rows for a string they cannot click; the host
        // still answers the question the annotation exists for, which is where
        // this goes. `t` has the whole of it, as it always did.
        let out = render(
            "see [the tracker](https://github.com/o/a/issues/1?utm=x)",
            46,
            Mode::Dark,
        );
        assert_eq!(text(&out), ["see the tracker (github.com/…)"]);
        assert_fits(&out, 46);

        // Under budget at the same width, and kept whole. The threshold is a
        // share of the pane rather than a fixed length, so a wide pane elides
        // less — which is the behaviour a reader dragging the split expects.
        let out = render("see [docs](http://x.dev)", 46, Mode::Dark);
        assert_eq!(text(&out), ["see docs (http://x.dev)"]);
        let out = render("see [docs](http://x.dev)", 24, Mode::Dark);
        assert_eq!(text(&out), ["see docs (x.dev)"]);
    }

    /// An elided host used to be indistinguishable from a URL that *was* a
    /// host: `https://github.com/Open/abeam/issues/42` and `https://github.com`
    /// both drew `(github.com)`, three words apart, with nothing saying one had
    /// been cut. One cell buys the distinction back.
    #[test]
    fn an_elided_destination_says_that_it_was_cut() {
        let shown = |md: &str| {
            let out = text(&render(md, 24, Mode::Dark));
            out.join(" ")
        };
        assert!(shown("[a](https://github.com/o/abeam/issues/42)").contains("(github.com/…)"));
        assert!(shown("[a](https://github.com)").contains("(github.com)"));
        assert_ne!(
            shown("[a](https://github.com/o/abeam/issues/42)"),
            shown("[a](https://github.com)")
        );
        // A query or a fragment on a bare host is a cut too, so `x.dev` and
        // `x.dev?q=1` do not collapse onto each other either.
        assert!(shown("[a](https://x.dev?q=1)").contains("(x.dev/…)"));
        assert!(shown("[a](http://x.dev)").contains("(x.dev)"));
        // Userinfo is dropped on the way to the host, which the mark is the
        // only warning of: a URL carrying credentials must not read as an
        // ordinary one.
        assert!(shown("[a](https://user:pw@h.io/a/b)").contains("(h.io/…)"));
        // A trailing slash is not information, so it is not a cut.
        assert!(shown("[a](https://averylongexample.dev/)").contains("(averylongexample.dev)"));
    }

    /// `://` is not a scheme separator on its own — a path can contain one, and
    /// the doc's promise is that a relative link survives at any length. Taking
    /// `weird` for a scheme cut this down to `path`.
    #[test]
    fn a_relative_path_that_contains_a_scheme_separator_is_still_kept_whole() {
        let out = text(&render("[x](./weird://path/to/a/file)", 20, Mode::Dark));
        assert!(
            out.join("").contains("./weird://path/to/a/file"),
            "{out:?} lost the path"
        );
        // The characters RFC 3986 allows in a scheme still work as one.
        assert_eq!(elide_url("git+ssh://h.io/a/b", 20), "h.io/…");
        assert_eq!(elide_url("x-y.z://h.io/a/b", 20), "h.io/…");
        assert_eq!(elide_url("9nine://h.io/a/b", 20), "9nine://h.io/a/b");
    }

    /// Every block in this file measures its decoration off the columns the
    /// *block* has, not the pane — `emit_code`, `emit_table`, the heading arm.
    /// An inline elision reading `self.width` gave a link six cells inside a
    /// nested quote the same budget as one at column zero.
    #[test]
    fn a_links_budget_is_the_room_its_block_has_and_not_the_panes() {
        let md = "[a](http://x.dev/q)";
        let flat = text(&render(md, 54, Mode::Dark)).join(" ");
        let buried = text(&render(&format!("> > > {md}"), 54, Mode::Dark)).join(" ");
        // Fifty-four columns is inside budget at column zero; six cells of
        // quote gutter is the whole of what pushes the second one under.
        assert!(flat.contains("(http://x.dev/q)"), "{flat:?}");
        assert!(buried.contains("(x.dev/…)"), "{buried:?}");
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

    /// The bargain `emit_code` claims front matter gets is the one
    /// `mermaid::render` makes: declined when it cannot be *drawn* and declined
    /// when it cannot be *fitted*. A width-blind predicate could only keep half
    /// of that, and the other half became an unmarked clip downstream — two
    /// fields drawn under one identical invented key, which is the failure the
    /// whole module is written to avoid.
    #[test]
    fn front_matter_too_wide_for_its_keys_keeps_the_block_it_had() {
        // Both of these clipped to `implementation_st` at nineteen columns —
        // one invented key over two different fields, and nothing on the page
        // saying so. Declined, every character is still there: the wrap breaks
        // the key across rows and the `┆ ` says it was broken, which is the
        // whole difference from a cut that says nothing.
        let md = "---\nimplementation_strategy: a\nimplementation_status: b\n---\n";
        let out = render(md, 19, Mode::Dark);
        assert_fits(&out, 19);
        let whole: String = text(&out)
            .iter()
            .map(|l| l.trim_start_matches(['│', '┆', ' ']).to_string())
            .collect();
        assert!(whole.contains("implementation_strategy: a"), "{whole:?}");
        assert!(whole.contains("implementation_status: b"), "{whole:?}");

        // One key on its own gets the same answer: an unmarked cut is unmarked
        // however few fields it happens to.
        let long = "---\nimplementation_strategy: two passes\n---\n";
        let out = render(long, 18, Mode::Dark);
        assert_eq!(
            text(&out),
            ["│ implementation_s", "┆ trategy: two pas", "┆ ses"]
        );
        assert_fits(&out, 18);

        // Wide enough for the key and it is a header again. Content-aware
        // rather than a blunt width gate: `k: v` still draws at four columns,
        // where a `CRAMPED` test would have thrown it away.
        assert_eq!(
            text(&render(long, 40, Mode::Dark)),
            ["implementation_strategy: two passes"]
        );
        assert_eq!(text(&render("---\nk: v\n---\n", 4, Mode::Dark)), ["k: v"]);

        // Under two cells for the key, `emit_record` drops it altogether and
        // the value goes out with its name silently removed. The source block
        // is both narrower and complete.
        assert_eq!(
            text(&render("---\nkey: value\n---\n", 3, Mode::Dark)),
            ["key", ": v", "alu", "e"]
        );
    }

    /// Everything the predicate declines, one line each. All of them used to
    /// render as *something*, which is the one outcome the module says it must
    /// not produce.
    #[test]
    fn front_matter_that_yaml_would_read_differently_keeps_the_block_it_had() {
        let dim = |md: &str| text(&render(md, 40, Mode::Dark));

        // A run of spaces inside a value. It goes through `wrap_spans`, which
        // collapses whitespace, so the header would draw `one two` and the
        // reader would have no way to know. `hard_wrap` would keep the run and
        // was rejected: it breaks at the column, so every ordinary title would
        // split mid-word to protect a case that hardly happens.
        assert_eq!(dim("---\na: one    two\n---\n"), ["│ a: one    two"]);

        // A trailing comment. YAML drops it, so drawing it is the author's
        // aside promoted to data — the rule the leading-`#` case already
        // states, one line further down.
        assert_eq!(
            dim("---\ntitle: Plan # a note\n---\n"),
            ["│ title: Plan # a note"]
        );

        // A colon with no space after it. `url:https://x.dev` is a plain
        // scalar, not a mapping, and `"a:b": v` has its key in quotes.
        assert_eq!(
            dim("---\nurl:https://x.dev\n---\n"),
            ["│ url:https://x.dev"]
        );
        // A quoted key survives whole instead of being cut at its first colon.
        // The quotes are drawn because the author wrote them — the same reason
        // a value is reproduced rather than re-quoted — not stripped, which
        // would be this deciding what the YAML meant.
        assert_eq!(dim("---\n\"a:b\": v\n---\n"), ["\"a:b\": v"]);

        // A block scalar indicator, which is YAML syntax rather than a value.
        for ind in ["|", ">", "|-", "|+", ">-", ">+", "|2"] {
            assert_eq!(
                dim(&format!("---\nnotes: {ind}\n---\n")),
                [format!("│ notes: {ind}")]
            );
        }
    }

    /// `Tag::MetadataBlock` carries the flavour and it used to be discarded, so
    /// a TOML block would have gone through a YAML predicate: `+++` around
    /// `date = 2024-01-01T00:00:00` splits at the colon into a key of
    /// `date = 2024-01-01T00`, which is a field invented out of a timestamp.
    ///
    /// Driven through the parser directly because `options()` does not enable
    /// the pluses flavour — which is exactly why a wildcard here was a trap
    /// rather than a bug: it springs the day somebody turns the option on.
    #[test]
    fn a_metadata_block_that_is_not_yaml_keeps_the_block_it_had() {
        let src = "+++\ndate = 2024-01-01T00:00:00\n+++\n\nbody\n";
        let opts = options() | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
        let mut r = Renderer::new(40, Mode::Dark);
        for event in Parser::new_ext(src, opts) {
            r.event(event);
        }
        assert_eq!(
            text(&r.finish().0),
            ["│ date = 2024-01-01T00:00:00", "", "body"]
        );
    }

    /// A contract on a helper whose whole job is a width. `split_to_width`
    /// always emits at least one character, so a two-cell ideograph came back
    /// from a one-cell budget. Nothing reaches that today — both callers hold
    /// the budget at two or more — which is the reason to pin it rather than
    /// the reason to leave it.
    #[test]
    fn clip_label_never_returns_more_cells_than_it_was_given() {
        for width in 0..8 {
            for label in ["ascii label", "日本語キー", "aあbい", ""] {
                let got = clip_label(label, width);
                assert!(
                    got.width() <= width,
                    "clip_label({label:?}, {width}) is {} cells",
                    got.width()
                );
            }
        }
        // And it still cuts where it can.
        assert_eq!(clip_label("abcdef", 3), "abc");
        assert_eq!(clip_label("日本語", 2), "日");
        assert_eq!(clip_label("日本語", 1), "");
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
    }

    /// No alt text is the common case in an agent-written plan — which is the
    /// argument for showing the destination, not for showing nothing. A bare
    /// `▨` says a picture was here without saying which one, and in a file
    /// viewer over a repository `x.png` is a path the reader can go and open.
    #[test]
    fn an_image_with_no_alt_text_falls_back_to_its_destination() {
        assert_eq!(text(&render("![](x.png)", 40, Mode::Dark)), ["▨ x.png"]);
        // Through the same elision a link's destination goes through, and at
        // the same budget.
        assert_eq!(
            text(&render("![](https://h.io/a/b.png)", 24, Mode::Dark)),
            ["▨ h.io/…"]
        );
        // Whitespace is not an alt. The trailing space on the glyph goes with
        // the wrap, which drops a space nothing follows.
        assert_eq!(text(&render("![ ](x.png)", 40, Mode::Dark)), ["▨ x.png"]);
        // An image with no destination either is the glyph and nothing else,
        // which is all there is to say.
        assert_eq!(text(&render("![]()", 40, Mode::Dark)), ["▨"]);
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
            // Below eight this fixture cannot fit, and not because of anything
            // here: `wrap` never clips a prefix, so four quote gutters are nine
            // cells before a word is drawn whatever the block behind them is —
            // `> > > > just a paragraph here` overflows the same way and did
            // before this pane drew a heading rule at all. Not a claim about
            // every document at every width under eight; a note that this one
            // is out of scope, and that ratatui clips. See
            // `absurd_widths_produce_something_rather_than_a_panic`.
            //
            // The heading change *narrowed* this rather than widening it:
            // `> > > > # heading` at eight was eleven cells with the hashes on
            // and is nine now.
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

    /// `(row, level, text)` for every heading the renderer reported.
    fn outlined(md: &str, width: usize) -> Vec<(usize, u8, String)> {
        render_outlined(md, width, Mode::Dark)
            .1
            .into_iter()
            .map(|e| (e.row, e.level, e.text))
            .collect()
    }

    #[test]
    fn a_heading_reports_the_row_its_words_are_on_and_not_the_rule_under_them() {
        let md = "# Title\n\nsome prose\n\n## Section\n\nmore prose\n";
        let (rows, entries) = render_outlined(md, 40, Mode::Dark);
        let rows = text(&rows);
        // The rule is a row of its own and it is the row *after* the heading,
        // so an entry pointing at it would land the reader with the section
        // above filling the pane and the title itself off the top edge.
        assert_eq!(rows[0], "Title");
        assert_eq!(rows[1], "━".repeat(40));
        assert_eq!(rows[5], "Section");
        assert_eq!(rows[6], "─".repeat("Section".len()));
        assert_eq!(
            outlined(md, 40),
            [(0, 1, "Title".into()), (5, 2, "Section".into())]
        );
        // And the entries are what the pane will index `rows` with.
        for (row, ..) in &entries.iter().map(|e| (e.row,)).collect::<Vec<_>>() {
            assert!(*row < rows.len());
        }
    }

    #[test]
    fn a_heading_wide_enough_to_wrap_reports_only_the_row_it_starts_on() {
        // Nineteen columns and a heading that cannot fit on one of them. Three
        // rows come out of it and exactly one entry, pointing at the first —
        // the row with the beginning of the sentence on it, which is the row a
        // reader jumping to this section wants at the top of the pane.
        let md = "## Four four four four four\n\nbody\n";
        let (rows, entries) = render_outlined(md, 19, Mode::Dark);
        assert!(text(&rows)[0].starts_with("Four"), "{:?}", text(&rows));
        assert!(text(&rows)[1].starts_with("four"), "{:?}", text(&rows));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].row, 0);
        // The whole heading, not the part that fitted on the first row: the
        // outline is a list of the document's own names, and a name cut off at
        // the pane's width would change as the window was dragged.
        assert_eq!(entries[0].text, "Four four four four four");
    }

    #[test]
    fn a_heading_inside_a_list_or_a_quote_is_reported_without_what_encloses_it() {
        // The prefixes on the drawn row — the bullet, the quote gutter, the pip
        // — are the pane talking about the document. They belong to whatever the
        // heading is nested in rather than to the heading, so they are not part
        // of its name and would make two `## Notes` in different places read as
        // two different sections.
        let md = "- item\n\n  ## In a list\n\n> ### In a quote\n";
        let out = text(&render(md, 40, Mode::Dark));
        assert!(out.iter().any(|l| l.contains("▏ In a quote")), "{out:?}");
        assert_eq!(
            outlined(md, 40)
                .into_iter()
                .map(|(_, level, t)| (level, t))
                .collect::<Vec<_>>(),
            [(2, "In a list".into()), (3, "In a quote".into())]
        );
    }

    #[test]
    fn a_heading_with_no_words_in_it_is_not_a_row_of_the_outline() {
        // `#` on its own is a legal ATX heading. It draws no rule, for the
        // reason `H1_RULE` gives, and it names nothing — so a row in a jump
        // list for it would be a blank line the reader cannot tell from any
        // other blank line.
        assert!(outlined("#\n\nbody\n", 40).is_empty());
        assert_eq!(outlined("# Real\n", 40).len(), 1);
    }

    #[test]
    fn a_headings_inline_markup_is_not_part_of_its_name() {
        // The text is taken from the spans the renderer is about to draw, which
        // is after `**` and backticks have been eaten — so the outline says
        // what the page says rather than what the file says. That is the same
        // choice `search` makes about a rendered document, and it is the one
        // that keeps `o` and the page agreeing.
        assert_eq!(
            outlined("## The `Page` **type**\n", 40),
            [(0, 2, "The Page type".into())]
        );
    }
}
