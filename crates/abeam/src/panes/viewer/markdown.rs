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

    /// `(language, text, dim)` while inside a fenced block, an indented block,
    /// an HTML block or a front-matter block.
    code: Option<(String, String, bool)>,
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
                    if let Some((lang, text, dim)) = self.code.take() {
                        self.emit_code(&lang, &text, dim);
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
                self.code = Some((lang, String::new(), false));
            }
            // Rendered dim rather than highlighted: it is metadata about the
            // document, not part of it, and it is always at the top where it
            // would otherwise be the loudest thing on screen.
            Tag::HtmlBlock | Tag::MetadataBlock(_) => {
                self.start_block();
                self.code = Some((String::new(), String::new(), true));
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
            Tag::Image { .. } => {
                self.spans.push(Span::styled("[image: ", self.theme.dim()));
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
            TagEnd::Heading(level) => {
                let (mut first, mut rest) = self.prefixes();
                let hashes = format!("{} ", "#".repeat(level as usize));
                let pad = " ".repeat(hashes.width());
                first.push(Span::styled(hashes, self.theme.dim()));
                rest.push(Span::raw(pad));
                let content = std::mem::take(&mut self.spans);
                let content = restyle(content, self.theme.heading(level as u8));
                self.out
                    .extend(wrap::wrap_spans(content, self.width, &first, &rest));
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
                        self.spans.push(Span::styled(format!(" ({url})"), self.theme.dim()));
                    }
                }
            }
            TagEnd::Image => {
                self.pop_style();
                self.spans.push(Span::styled("]", self.theme.dim()));
            }

            TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {
                // Handled by the raw-capture branch in `event`; only reachable
                // if a block opened and closed with nothing in between.
                if let Some((lang, text, dim)) = self.code.take() {
                    self.emit_code(&lang, &text, dim);
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

    fn emit_code(&mut self, lang: &str, text: &str, dim_all: bool) {
        // Fences arrive with a trailing newline that is fence syntax, not a
        // blank line the author wrote.
        let body = text.strip_suffix('\n').unwrap_or(text);
        // Taken before either branch draws, because it consumes the pending
        // list marker and the marker may only be spent once.
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

        let rows = if dim_all {
            source::plain(body, self.theme.dim())
        } else {
            source::highlight_code(body, lang, self.mode)
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

/// Cut a table label down to `width` cells. Deliberately *not* `text::clip`:
/// this is the one place truncation goes unmarked, because at the widths a
/// narrow-table label is squeezed into, the ellipsis is a measurable fraction
/// of the label itself.
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

    #[test]
    fn headings_are_marked_by_level_and_coloured() {
        let out = render("# One\n\n## Two\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["# One", "", "## Two"]);
        assert_eq!(
            styles_of(&out[0], "One").and_then(|s| s.fg),
            theme::DARK.heading(1).fg
        );
        assert_eq!(
            styles_of(&out[2], "Two").and_then(|s| s.fg),
            theme::DARK.heading(2).fg
        );
        // The hashes stay, dimmed: a terminal has no font size, so the level
        // has to be readable as text.
        assert_eq!(styles_of(&out[0], "#").and_then(|s| s.fg), Some(theme::DARK.dim));
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
    fn front_matter_is_shown_dimmed_rather_than_parsed_as_a_rule() {
        let out = render("---\ntitle: x\n---\n\n# Body\n", 40, Mode::Dark);
        assert_eq!(text(&out), ["│ title: x", "", "# Body"]);
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
# Heading here

> quoted text that runs on

- item one
  - nested item

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
        // there to a plausible right pane must fit exactly.
        for width in 8..=48 {
            assert_fits(&render(KITCHEN_SINK, width, Mode::Dark), width);
        }
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

