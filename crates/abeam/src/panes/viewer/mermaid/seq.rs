//! `sequenceDiagram`: participants, and what they say to each other.
//!
//! ## Two layouts, and the measurement between them
//!
//! A sequence diagram is a *grid* problem rather than a flow problem: every
//! message is a horizontal run between two fixed columns, and those columns
//! have to be far enough apart to hold every run that crosses them. So the
//! whole drawing is planned before a single cell is written — the participant
//! boxes give a minimum spacing, each message adds a "these two lifelines must
//! be at least this far apart" constraint, and because every constraint points
//! from a lower participant index to a higher one the system solves in one
//! left-to-right pass instead of needing a relaxation loop.
//!
//! When the plan does not fit the pane, the diagram is written out as a
//! numbered list instead. That is the same trade `markdown::emit_table_as_records`
//! makes when a grid will not fit, for the same reason: lifelines six columns
//! apart are not a diagram, they are a puzzle.
//!
//! ## Wrapping is the free variable
//!
//! Message text is the thing that forces lifelines apart, and text can wrap. So
//! the plan is solved repeatedly against a cap on how wide any one label line
//! may be, and the widest cap that still fits the pane is the one drawn. The
//! search bottoms out at the longest *word* and never goes below it: a word
//! broken across two label rows is a word the reader has to reassemble, and the
//! premise of this module is that the drawing is at least as readable as the
//! source it replaced. That is also the gate at the end of [`render`] — every
//! word of every label has to still be on screen as a *whole token*, and a
//! drawing that lost one is thrown away in favour of the list, or the list in
//! favour of `None`. Whole token, not "appears somewhere": see [`keeps`], where
//! the difference is the gate working and the gate being satisfied by `no`
//! turning up inside `Note`. It is a check rather than a width formula because
//! the ways a word can go missing (a note clipped at the pane edge, a block
//! label too long for its frame, a caption centred over too short a span) are
//! several, and one assertion that catches all of them beats four arithmetic
//! rules that each catch one.
//!
//! ## Why a cell grid rather than strings
//!
//! Rows are built as `Vec<Cell>` and stamped into, because nearly everything
//! here draws *over* something already drawn: a message label crosses the
//! lifelines between its two ends, a note is an opaque box laid on top of one,
//! a block frame rules straight through them all. Building that from strings
//! would mean splicing by byte offset into text that may be ideographs, and the
//! bug that costs — half a wide glyph left behind after something was stamped
//! over its other half — is precisely the bug that makes a row measure right
//! and print wrong. [`Cell::Tail`] is that failure made unrepresentable, and
//! [`Row::marks`] is the other half of the same problem: a combining mark is
//! *zero* cells and belongs on the character before it, not in a cell of its
//! own and not on the floor.
//!
//! Tried and rejected: putting the message text *on* the arrow run rather than
//! above it. It reads well in a wide pane and collapses in a narrow one,
//! because the run can then never be shorter than the text — nothing wraps, and
//! the first sentence-long message pushes the diagram past any pane there is.
//!
//! ## Colour
//!
//! None of it is decided here. The palette is assigned by role in [`super`],
//! for both diagram families at once, so that a document containing a flowchart
//! and a sequence diagram does not read as two pieces of software: text is the
//! body colour, every connector is `dim` whatever its stroke, a severed end is
//! `╳` in `danger`, and a keyword abeam supplies is `special`. What is decided
//! here is only which of those roles a thing *is* — and the one judgement in it
//! is that a participant's name is text, not chrome.

use std::collections::{HashMap, HashSet};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{MAX_EDGES, MAX_NODES, Rows, lex};
use crate::panes::viewer::theme::Theme;
use crate::text::wrap::wrap_spans;

/// Blank columns between two participant boxes that nothing else has pushed
/// apart. Counted as blank *cells*, the way `flow::GAP` counts them, and it has
/// to be at least one: two boxes whose borders touch draw as `┐┌`, which at
/// forty columns reads as one wide box with a seam rather than as two
/// participants. One is enough, because each box already carries a space of
/// padding inside its own border.
const GAP: usize = 1;
/// Columns to the right of a lifeline that a self-message's hook needs.
const SELF_HOOK: usize = 3;
/// Below this many usable columns, the list stops indenting by depth — the
/// same floor `markdown` calls `CRAMPED`, for the same reason: past it the
/// indent is eating the text it exists to organise.
const CRAMPED: usize = 12;

/// Most rows the lifelines may take before the list is drawn instead.
///
/// `flow::ROW_CAP` is the same number for the same reason, and this is that
/// argument applied to a conversation: a diagram three screens tall has stopped
/// being a diagram, and the list says the same exchange in a row or two per
/// message. Not a clock bound — the caps in [`super`] are what keep the layout
/// itself cheap — but the two work together, because the drawing is not built
/// twice and the list is reached without paying for a third layout.
const ROW_CAP: usize = 240;
/// ...and where the list itself gives up, matching `outline`'s own cap. Past
/// this there is no rendering left that beats the source: a thousand rows of
/// numbered messages is a transcript, and the transcript was the fence.
const LIST_CAP: usize = 1000;

/// Blocks that open and are closed by `end`.
const BLOCKS: [&str; 7] = ["loop", "alt", "opt", "par", "critical", "break", "rect"];
/// Section keywords, each with the block it is only meaningful inside.
const SECTIONS: [(&str, &str); 3] = [("else", "alt"), ("and", "par"), ("option", "critical")];

/// Draw a sequence diagram body into at most `width` columns.
pub fn render(body: &[String], width: usize, theme: &'static Theme) -> Option<Rows> {
    let diagram = parse(body)?;
    let words = diagram.words();

    // Lifelines when they fit *and* survive the reading; the list otherwise.
    // Both are checked against the same words, so the fallback chain can only
    // ever move to a layout that keeps more of the diagram, never less.
    if let Some(rows) = lifelines(&diagram, width, theme)
        && keeps(&rows, &words)
    {
        return Some(rows);
    }
    let rows = as_list(&diagram, width, theme)?;
    keeps(&rows, &words).then_some(rows)
}

// --- the diagram ---------------------------------------------------------

/// Everything one `sequenceDiagram` says, in the order it says it.
#[derive(Default)]
struct Diagram {
    title: Option<String>,
    /// Display labels, in the order mermaid itself orders participants:
    /// declaration order, and for anything undeclared, order of first mention.
    actors: Vec<String>,
    events: Vec<Event>,
}

enum Event {
    Message(Message),
    Note(Note),
    /// `loop` / `alt` / `opt` / `par` / `critical` / `break` / `rect`.
    Open(&'static str, String),
    /// `else` / `and` / `option` — a new section of the block already open.
    Section(&'static str, String),
    Close,
    /// `activate` / `deactivate`, as a bare statement rather than a suffix.
    Toggle(usize, bool),
}

struct Message {
    from: usize,
    to: usize,
    arrow: Arrow,
    text: String,
    /// The `autonumber` this message was dealt, if the diagram asked for one.
    number: Option<usize>,
    /// The `+`/`-` suffix: activate the target, or deactivate the sender.
    toggle: Option<bool>,
}

impl Message {
    /// What is drawn over the arrow. The number is part of the text rather than
    /// a separate span because it has to be *measured* with the text — it is
    /// what decides how far apart the two lifelines end up.
    fn caption(&self) -> String {
        match (self.number, self.text.is_empty()) {
            (Some(n), true) => format!("{n}."),
            (Some(n), false) => format!("{n}. {}", self.text),
            (None, _) => self.text.clone(),
        }
    }
}

struct Note {
    place: Place,
    /// The one participant, or the two a `Note over A,B` spans.
    over: (usize, usize),
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Left,
    Right,
    Over,
}

impl Diagram {
    fn messages(&self) -> impl Iterator<Item = &Message> {
        self.events.iter().filter_map(|e| match e {
            Event::Message(m) => Some(m),
            _ => None,
        })
    }

    /// How deeply the blocks nest, which is what the frame gutters cost.
    fn depth(&self) -> usize {
        let mut deepest = 0;
        let mut open = 0usize;
        for e in &self.events {
            match e {
                Event::Open(..) => {
                    open += 1;
                    deepest = deepest.max(open);
                }
                Event::Close => open = open.saturating_sub(1),
                _ => {}
            }
        }
        deepest
    }

    /// Every word that has to survive to the screen. See the module note: this
    /// is the rule from `mermaid`'s own note — no fourth outcome where content
    /// is dropped — turned into something checkable.
    fn words(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut take = |text: &str| out.extend(text.split_whitespace().map(str::to_string));
        if let Some(title) = &self.title {
            take(title);
        }
        for actor in &self.actors {
            take(actor);
        }
        for e in &self.events {
            match e {
                Event::Message(m) => take(&m.caption()),
                Event::Note(n) => take(&n.text),
                Event::Open(kind, label) | Event::Section(kind, label) => {
                    take(kind);
                    take(label);
                }
                _ => {}
            }
        }
        out
    }
}

/// Whether every word is still on screen, whole.
///
/// Whole is the load-bearing part, and it is why this compares *tokens* rather
/// than asking whether the drawing contains the word somewhere. Containment
/// passes on an accident — `no` is inside `Note`, `end` is inside `friend`,
/// `and` is inside `command`, and all three of those words are collected by
/// [`Diagram::words`] from block keywords this module writes itself. A gate
/// that can be satisfied by a coincidence is a gate that ships a clipped
/// drawing, and this one decides lifelines → list → `None`.
///
/// Every glyph this draws is separated from text by a space (see the box and
/// frame drawing), with one exception: the list glues the `:` or `,` that
/// follows a participant's name onto the name, because the wrapper breaks
/// between spans as readily as between words and a row consisting of one colon
/// is a row. So a token is also offered with that punctuation taken back off.
fn keeps(rows: &Rows, words: &[String]) -> bool {
    let text = rows
        .iter()
        .map(|row| row.iter().map(|s| s.content.as_ref()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    let mut seen: HashSet<&str> = HashSet::new();
    for token in text.split_whitespace() {
        seen.insert(token);
        let bare = token.trim_end_matches([',', ':']);
        if bare != token && !bare.is_empty() {
            seen.insert(bare);
        }
    }
    words.iter().all(|word| seen.contains(word.as_str()))
}

// --- arrows --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stroke {
    Solid,
    Dotted,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Head {
    /// `->`: a line that arrives without pointing.
    Plain,
    /// `->>`: the everyday call.
    Filled,
    /// `-)`: the asynchronous one, drawn open so it reads as "and no reply".
    Open,
    /// `-x`: the one that ends in a failure.
    Cross,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Arrow {
    stroke: Stroke,
    head: Head,
    /// `<<->>`: a head at the sending end as well.
    back: bool,
}

/// Longest token first, because `-->>` starts with `-->` and `->>` starts with
/// `->`. Scanning is by position, so the order *within* this table is the whole
/// of the disambiguation.
const ARROWS: [(&str, Stroke, Head, bool); 10] = [
    ("<<-->>", Stroke::Dotted, Head::Filled, true),
    ("<<->>", Stroke::Solid, Head::Filled, true),
    ("-->>", Stroke::Dotted, Head::Filled, false),
    ("--x", Stroke::Dotted, Head::Cross, false),
    ("--)", Stroke::Dotted, Head::Open, false),
    ("-->", Stroke::Dotted, Head::Plain, false),
    ("->>", Stroke::Solid, Head::Filled, false),
    ("-x", Stroke::Solid, Head::Cross, false),
    ("-)", Stroke::Solid, Head::Open, false),
    ("->", Stroke::Solid, Head::Plain, false),
];

impl Arrow {
    /// The run between the two lifelines. Two glyphs rather than two colours,
    /// which is the module note's rule and not a preference: a dotted reply and
    /// a solid call have to be tellable apart in a screenshot, in monochrome,
    /// and by a reader who does not receive hue. So both runs are `dim` — see
    /// [`Arrow::ink`] — and the glyph carries the whole distinction.
    fn line(self) -> char {
        match self.stroke {
            Stroke::Solid => '─',
            Stroke::Dotted => '┄',
        }
    }

    /// What lands on the lifeline being spoken *to*. `rightward` is where the
    /// message is going, which is also which side of the run this end is on.
    fn tip(self, rightward: bool) -> char {
        match (self.head, rightward) {
            // A line with no head still has to join its lifeline rather than
            // stop beside it, and box drawing already has the two glyphs for
            // exactly that junction.
            (Head::Plain, true) => '┤',
            (Head::Plain, false) => '├',
            (Head::Filled, true) => '▶',
            (Head::Filled, false) => '◀',
            (Head::Open, true) => '>',
            (Head::Open, false) => '<',
            // `╳`, the box-drawing cross, and not the dingbat `✗` that reads the
            // same in a proportional font: `flow` draws `--x` with this one, the
            // two grammars are spelling a single concept, and a reader with both
            // diagrams in one document must not see two.
            (Head::Cross, _) => '╳',
        }
    }

    /// What lands on the lifeline doing the speaking.
    fn tail(self, rightward: bool) -> char {
        if self.back {
            self.tip(!rightward)
        } else if rightward {
            '├'
        } else {
            '┤'
        }
    }

    /// The same arrow in one short token, for the list layout, where direction
    /// is carried by the row's word order and the glyph only has to say *kind*.
    fn token(self) -> String {
        let mut out = String::new();
        if self.back {
            out.push(self.tip(false));
        }
        out.push(self.line());
        out.push(match self.head {
            Head::Plain => self.line(),
            _ => self.tip(true),
        });
        out
    }

    /// The run, and the junction it leaves from. A connector is structure and
    /// is `dim` at every stroke: the module note assigns the palette by role
    /// exactly so that a reader cannot tell which of the two drawers is on
    /// screen, and an earlier version of this painted solid runs in the body
    /// colour, which made hue carry the solid/dotted difference a second time
    /// and made a diagram of solid calls the loudest thing on the page.
    fn ink(self, theme: &'static Theme) -> Color {
        theme.dim
    }

    /// The tip, which is the one part of an arrow that is allowed to shout. A
    /// cross is the arrow that means something went wrong, and it is the one a
    /// reader should find without having to look for it.
    fn tip_ink(self, theme: &'static Theme) -> Color {
        match self.head {
            Head::Cross => theme.danger,
            _ => theme.dim,
        }
    }
}

// --- parsing -------------------------------------------------------------

fn parse(body: &[String]) -> Option<Diagram> {
    let mut d = Diagram::default();
    let mut ids: HashMap<String, usize> = HashMap::new();
    // `(next number, step)` while `autonumber` is on.
    let mut counter: Option<(usize, usize)> = None;
    let mut open: Vec<&'static str> = Vec::new();

    for raw in body {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (kw, rest) = split_keyword(line);
        match kw {
            // Participants that arrive and leave partway down, and the `box`
            // that groups them, are drawn as things that *change* — a lifeline
            // that starts late, a box that ends early. A static column of
            // lifelines cannot say that, and saying it badly is worse than the
            // source, which says it exactly.
            "create" | "destroy" | "box" | "link" | "links" | "properties" => return None,

            // One keyword, one drawing. Mermaid draws `actor` as a stick figure
            // and `participant` as a box, and that difference does not survive
            // the trip to a character grid: the figure is three strokes and a
            // head, so it is either four rows of header on every diagram that
            // contains one actor, or a single glyph — `☺`, `웃` — whose width a
            // terminal is free to disagree about, in a layout counted in cells.
            // It is also the one mermaid distinction that changes nothing else:
            // not the ordering, not the lifeline, not a single arrow. What it
            // says — that this participant is a person — the *name* already
            // says. So both draw as a box, and `actor` is accepted rather than
            // declined, because declining over a synonym would cost the reader
            // the whole diagram. (`flow` makes the same trade, louder: twelve
            // node shapes onto three frames.)
            "participant" | "actor" => {
                let (id, alias) = split_alias(rest);
                actor(&mut d, &mut ids, id, alias)?;
            }

            "autonumber" => {
                let mut words = rest.split_whitespace();
                counter = match words.next() {
                    None => Some((1, 1)),
                    Some("off") => None,
                    Some(start) => {
                        let start = start.parse().ok()?;
                        let step = match words.next() {
                            Some(step) => step.parse().ok()?,
                            None => 1,
                        };
                        Some((start, step))
                    }
                };
            }

            "activate" | "deactivate" => {
                let who = actor(&mut d, &mut ids, rest, None)?;
                d.events.push(Event::Toggle(who, kw == "activate"));
            }

            // `title Foo` and `title: Foo` are both written in the wild, and
            // the second arrives with the colon still stuck to the keyword.
            "title" | "title:" => {
                let text = flatten(&lex::label(rest.trim_start_matches(':')));
                if !text.is_empty() {
                    d.title = Some(text);
                }
            }

            // Matched through the tables rather than as literals so the keyword
            // kept in the event is the `'static` one, not a slice of the line
            // it was read from.
            _ if BLOCKS.contains(&kw) => {
                let kind = BLOCKS.into_iter().find(|block| *block == kw)?;
                open.push(kind);
                d.events.push(Event::Open(kind, flatten(&lex::label(rest))));
            }
            // A section keyword outside the block it sections is not a diagram
            // mermaid draws either, and guessing which block was meant is how a
            // drawing ends up claiming a structure the file does not have.
            _ if SECTIONS.iter().any(|(name, _)| *name == kw) => {
                let (kind, parent) = SECTIONS.into_iter().find(|(name, _)| *name == kw)?;
                if *open.last()? != parent {
                    return None;
                }
                d.events.push(Event::Section(kind, flatten(&lex::label(rest))));
            }
            "end" => {
                open.pop()?;
                d.events.push(Event::Close);
            }

            _ => {
                // `Note` is matched case-insensitively because mermaid's own
                // lexer does; everything above is not, because mermaid's is
                // not. A participant genuinely called `note` still works — the
                // placement word has to follow for this branch to take it.
                if kw.eq_ignore_ascii_case("note")
                    && let Some((place, rest)) = placement(rest)
                {
                    let note = note(&mut d, &mut ids, place, rest)?;
                    d.events.push(Event::Note(note));
                } else {
                    let message = message(&mut d, &mut ids, line, &mut counter)?;
                    d.events.push(Event::Message(message));
                }
            }
        }
    }

    // An unclosed block is a diagram the reader's browser refuses to draw, and
    // closing it here would put abeam and that browser in disagreement about
    // the same file.
    if !open.is_empty() {
        return None;
    }
    // Nothing at all is not a drawing. The header on its own is worth more as
    // source, where it at least says what the author was starting.
    if d.actors.is_empty() && d.events.is_empty() && d.title.is_none() {
        return None;
    }
    // Checked after parsing rather than while, matching `flow::Graph::parse`:
    // the caps are about the size of the layout problem, and a statement is not
    // one of anything — `participant A` adds a column and no band, `autonumber`
    // adds neither.
    //
    // A *band* is anything that claims vertical space of its own: a message, a
    // note, and each block or section that opens a frame — the `end` closing
    // one is implied by the block it closes and needs no count of its own.
    // Counting only messages left the two shapes that are not messages
    // unbounded, and thirty kilobytes of `Note over A:` is legal mermaid that
    // took four times the layout budget of a diagram sitting at both caps at
    // once. The bands are what the row count is made of, so the bands are what
    // is counted.
    let bands = d
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Message(_) | Event::Note(_) | Event::Open(..) | Event::Section(..)
            )
        })
        .count();
    (d.actors.len() <= MAX_NODES && bands <= MAX_EDGES).then_some(d)
}

/// `(first word, the rest)`.
fn split_keyword(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(at) => (&line[..at], line[at..].trim_start()),
        None => (line, ""),
    }
}

/// `A as Alice` into its two halves. Matched on ` as ` with its spaces, so that
/// `participant Cassandra` is one participant rather than `C` aliased to
/// `sandra`.
fn split_alias(rest: &str) -> (&str, Option<&str>) {
    match rest.find(" as ") {
        Some(at) => (&rest[..at], Some(&rest[at + 4..])),
        None => (rest, None),
    }
}

/// Find or make a participant, and give it its display label.
fn actor(
    d: &mut Diagram,
    ids: &mut HashMap<String, usize>,
    id: &str,
    alias: Option<&str>,
) -> Option<usize> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    let shown = flatten(&lex::label(alias.unwrap_or(id)));
    if shown.is_empty() {
        return None;
    }
    if let Some(&at) = ids.get(id) {
        // A later `participant A as Alice` renames a participant an earlier
        // message already brought into being.
        if alias.is_some() {
            d.actors[at] = shown;
        }
        return Some(at);
    }
    // No cap here: `parse` applies it once, at the end, for the reason written
    // there. The input is bounded to `super::MAX_BYTES` long before this, so
    // what grows in between cannot get away from us.
    d.actors.push(shown);
    ids.insert(id.to_string(), d.actors.len() - 1);
    Some(d.actors.len() - 1)
}

fn message(
    d: &mut Diagram,
    ids: &mut HashMap<String, usize>,
    line: &str,
    counter: &mut Option<(usize, usize)>,
) -> Option<Message> {
    let (left, arrow, right) = split_arrow(line)?;
    // The colon is not optional in mermaid, and a line without one is far more
    // likely to be a statement this does not know than a message with no text.
    let (target, text) = right.split_once(':')?;
    let target = target.trim();
    let (target, toggle) = match target.strip_prefix('+') {
        Some(rest) => (rest, Some(true)),
        None => match target.strip_prefix('-') {
            Some(rest) => (rest, Some(false)),
            None => (target, None),
        },
    };
    // Source first, so that declaration order is order of first *mention*,
    // which is how mermaid itself orders undeclared participants.
    let from = actor(d, ids, left, None)?;
    let to = actor(d, ids, target, None)?;
    // Saturating, not wrapping and not a refusal. `autonumber 18446744073709551615`
    // is two lines from any document, and both alternatives are worse than a
    // counter that stops: wrapping numbers the next message `0.`, which is a
    // lie the reader has no way to see through, and declining throws away every
    // participant, note and block in the file over a decoration on a caption
    // whose *text* is the content. Two messages numbered alike at the top of
    // `usize` mislead nobody — nobody reads that as an ordinal.
    let number = counter.map(|(next, step)| {
        *counter = Some((next.saturating_add(step), step));
        next
    });
    Some(Message {
        from,
        to,
        arrow,
        text: flatten(&lex::label(text)),
        number,
        toggle,
    })
}

fn split_arrow(line: &str) -> Option<(&str, Arrow, &str)> {
    for (at, _) in line.char_indices() {
        for (token, stroke, head, back) in ARROWS {
            if line[at..].starts_with(token) {
                // An arrow with nothing before it has no sender. Half-typed
                // mermaid arrives here constantly — the watcher shows us the
                // file while the agent is still writing it.
                if at == 0 {
                    return None;
                }
                let arrow = Arrow { stroke, head, back };
                return Some((&line[..at], arrow, &line[at + token.len()..]));
            }
        }
    }
    None
}

/// `over` / `left of` / `right of`, and what follows it.
fn placement(rest: &str) -> Option<(Place, &str)> {
    if let Some(tail) = strip_word(rest, "over") {
        return Some((Place::Over, tail));
    }
    if let Some(tail) = strip_word(rest, "left").and_then(|t| strip_word(t, "of")) {
        return Some((Place::Left, tail));
    }
    if let Some(tail) = strip_word(rest, "right").and_then(|t| strip_word(t, "of")) {
        return Some((Place::Right, tail));
    }
    None
}

/// `word` at the front of `s`, as a whole word and ignoring case. Sliced with
/// `get` rather than indexed, because `s` may open with an ideograph and a byte
/// index into the middle of one is a panic reachable from a document.
fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    if !s.get(..word.len())?.eq_ignore_ascii_case(word) {
        return None;
    }
    let tail = &s[word.len()..];
    (tail.is_empty() || tail.starts_with(char::is_whitespace)).then(|| tail.trim_start())
}

fn note(
    d: &mut Diagram,
    ids: &mut HashMap<String, usize>,
    place: Place,
    rest: &str,
) -> Option<Note> {
    let (names, text) = rest.split_once(':')?;
    let mut names = names.split(',').map(str::trim).filter(|n| !n.is_empty());
    let first = actor(d, ids, names.next()?, None)?;
    let second = match names.next() {
        Some(name) => actor(d, ids, name, None)?,
        None => first,
    };
    // Mermaid spans a note over two participants at most; a third name means
    // this is not the statement it looks like.
    if names.next().is_some() {
        return None;
    }
    Some(Note {
        place,
        over: (first, second),
        text: flatten(&lex::label(text)),
    })
}

/// A label on one line. `lex::label` keeps `<br>` as a newline because a *node*
/// box has room for two lines; a participant header and a message caption are
/// laid out by width and do not, so the break becomes the space it stands in
/// for rather than something the grid has to make room for.
fn flatten(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ")
}

// --- the cell grid -------------------------------------------------------

#[derive(Clone, Copy)]
enum Cell {
    Blank,
    Ch(char, Style),
    /// The right-hand half of the wide character in the cell before this one.
    /// It prints nothing, which is what makes a row's cell count and its
    /// display width the same number.
    Tail,
}

struct Row {
    cells: Vec<Cell>,
    /// Combining marks, each held against the cell whose character it modifies.
    ///
    /// Beside the grid rather than in it, because a `Cell` is `Copy` and one
    /// row in a thousand carries a mark: a diagram written in Latin should not
    /// pay a `String` per cell so that `नमस्ते` can be drawn. Empty is the
    /// overwhelmingly common case and costs one capacity check.
    marks: Vec<(usize, char)>,
}

impl Row {
    fn new(width: usize) -> Self {
        Row {
            cells: vec![Cell::Blank; width],
            marks: Vec::new(),
        }
    }

    /// Stamp one character, clipping anything that would run past the end. The
    /// callers have all measured first, so a clip here is a bug — but it is a
    /// bug that loses a word, which the gate in `render` catches, rather than
    /// one that overflows the pane, which nothing would.
    fn put(&mut self, x: usize, ch: char, style: Style) {
        let w = ch.width().unwrap_or(0);
        if w == 0 || x + w > self.cells.len() {
            return;
        }
        for at in x..x + w {
            self.erase(at);
        }
        self.cells[x] = Cell::Ch(ch, style);
        if w == 2 {
            self.cells[x + 1] = Cell::Tail;
        }
    }

    /// Blank the cell at `x` *and* the other half of whatever wide character
    /// was straddling it. Leaving the orphan behind is how a row that measured
    /// correctly prints one cell wrong.
    fn erase(&mut self, x: usize) {
        match self.cells.get(x) {
            Some(Cell::Tail) => {
                if x > 0 {
                    self.blank(x - 1);
                }
                self.blank(x);
            }
            Some(Cell::Ch(ch, _)) => {
                if ch.width() == Some(2) && x + 1 < self.cells.len() {
                    self.blank(x + 1);
                }
                self.blank(x);
            }
            _ => {}
        }
    }

    /// Empty one cell, marks and all. A mark belongs to the character it was
    /// written against, so it goes when that character does.
    fn blank(&mut self, x: usize) {
        self.cells[x] = Cell::Blank;
        if !self.marks.is_empty() {
            self.marks.retain(|(at, _)| *at != x);
        }
    }

    /// Stamp only where nothing has been drawn yet. A block frame's sides are
    /// filled in after its contents, and a side that overwrote the `├` of its
    /// own `else` rule would break the frame open at exactly the row that says
    /// what the frame is.
    fn under(&mut self, x: usize, ch: char, style: Style) {
        if matches!(self.cells.get(x), Some(Cell::Blank)) {
            self.put(x, ch, style);
        }
    }

    /// Stamp a string, one cell per column of display width.
    ///
    /// A zero-width character — a combining mark, and Devanagari and Hebrew are
    /// full of them — is *appended to the cell of the character it modifies*
    /// rather than given a cell or dropped. Dropping is what this did first,
    /// and it was not a lost-content bug because `keeps` caught it — but it
    /// caught it by throwing the lifelines away, so `A->>B: नमस्ते there` fell to
    /// the list while the identical Latin diagram drew boxes. `flow` attaches
    /// too; two families disagreeing about one input is a bug even when both
    /// answers are defensible.
    fn puts(&mut self, x: usize, text: &str, style: Style) {
        let mut at = x;
        let mut base: Option<usize> = None;
        for ch in text.chars() {
            match ch.width().unwrap_or(0) {
                // A mark with nothing before it modifies nothing. It is dropped,
                // `keeps` notices, and the drawing steps down a layout — which
                // is the right answer for text that begins mid-grapheme.
                0 => {
                    if let Some(cell) = base
                        && matches!(self.cells.get(cell), Some(Cell::Ch(..)))
                    {
                        self.marks.push((cell, ch));
                    }
                }
                w => {
                    self.put(at, ch, style);
                    base = Some(at);
                    at += w;
                }
            }
        }
    }

    fn fill(&mut self, from: usize, to: usize, ch: char, style: Style) {
        for at in from..=to.min(self.cells.len().saturating_sub(1)) {
            self.put(at, ch, style);
        }
    }

    fn spans(mut self) -> Vec<Span<'static>> {
        // Trailing blanks are a row of spaces the pane would have to paint and
        // the tests would have to spell out.
        let end = self
            .cells
            .iter()
            .rposition(|c| !matches!(c, Cell::Blank))
            .map_or(0, |at| at + 1);
        self.cells.truncate(end);

        let mut out: Vec<Span<'static>> = Vec::new();
        let mut text = String::new();
        let mut style = Style::default();
        for (col, cell) in self.cells.into_iter().enumerate() {
            let (ch, next) = match cell {
                Cell::Tail => continue,
                // A blank has no colour of its own, so it neither continues a
                // run nor is worth one: the page under it is already painted.
                Cell::Blank => (' ', Style::default()),
                Cell::Ch(ch, owned) => (ch, owned),
            };
            if next != style && !text.is_empty() {
                out.push(Span::styled(std::mem::take(&mut text), style));
            }
            style = next;
            text.push(ch);
            // Marks ride with their base character, and add no width: a cell
            // still measures one column with three of them on it.
            for (_, mark) in self.marks.iter().filter(|(at, _)| *at == col) {
                text.push(*mark);
            }
        }
        if !text.is_empty() {
            out.push(Span::styled(text, style));
        }
        out
    }
}

// --- planning ------------------------------------------------------------

/// Where everything goes, once the width is known.
struct Plan {
    /// The column each participant's lifeline runs down.
    x: Vec<usize>,
    /// Half of each participant box, exactly: the boxes are forced to an odd
    /// width so that their centre is a cell rather than a boundary.
    half: Vec<usize>,
    /// Cells the whole drawing occupies, which is at most the pane.
    grid: usize,
    /// Columns the frame gutters own on each side.
    pad: usize,
    /// The label-line width this plan was solved for. Notes are wrapped to it
    /// at draw time so that what is drawn is the size that was reserved.
    cap: usize,
    /// Each message's caption, already wrapped to the cap this plan solved for.
    caption: Vec<Vec<String>>,
}

fn box_width(label: &str) -> usize {
    let w = label.width() + 4;
    // Odd, so the lifeline leaves the box from a cell and not from between two.
    if w.is_multiple_of(2) { w + 1 } else { w }
}

/// What the participants have to be spaced by, before any spacing is chosen.
///
/// Constraints are collected against their *right-hand* participant, which is
/// what lets the sweep be a single pass: by the time index `j` is placed, every
/// `i < j` it depends on is already final.
#[derive(Default)]
struct Spacing {
    half: Vec<usize>,
    /// `(left participant, cells between the two centres)`, from messages.
    messages: Vec<Vec<(usize, usize)>>,
    /// The same, from notes — kept apart because they are the constraints this
    /// is willing to abandon. See [`Spacing::sweep`].
    notes: Vec<Vec<(usize, usize)>>,
    /// Room a note wants to the left of the first participant.
    lead: usize,
    /// Room past the last lifeline, for a self-message or a note.
    overhang: usize,
    note_overhang: usize,
}

impl Spacing {
    /// Positions and the grid width they need. `notes` is whether the notes'
    /// constraints are honoured: they are *preferences*, because a note is
    /// commentary, it is frequently the longest text in the file, and letting
    /// one decide the diagram does not fit at all would be the tail wagging the
    /// dog. When honouring them costs the drawing, they are dropped and the
    /// note is clamped into whatever room exists instead.
    fn sweep(&self, pad: usize, notes: bool) -> (Vec<usize>, usize) {
        let n = self.half.len();
        let mut x = vec![0usize; n];
        x[0] = pad + if notes { self.lead } else { 0 } + self.half[0];
        for j in 1..n {
            // The next box opens on the cell after the previous box's right
            // edge, plus `GAP` blank ones — hence the `+ 1`, which is that
            // first cell and is the difference between `┐┌` and `┐ ┌`.
            let mut at = x[j - 1] + self.half[j - 1] + 1 + GAP + self.half[j];
            for &(i, need) in &self.messages[j] {
                at = at.max(x[i] + need);
            }
            if notes {
                for &(i, need) in &self.notes[j] {
                    at = at.max(x[i] + need);
                }
            }
            x[j] = at;
        }
        let last = x[n - 1];
        let tail = match notes {
            true => self.overhang.max(self.note_overhang),
            false => self.overhang,
        };
        let grid = (last + self.half[n - 1]).max(last + tail) + 1 + pad;
        (x, grid)
    }
}

/// Solve the spacing for one cap on label-line width, or fail to fit.
fn place(d: &Diagram, cap: usize, pad: usize, width: usize) -> Option<Plan> {
    let n = d.actors.len();
    if n == 0 {
        return None;
    }
    let mut s = Spacing {
        half: d
            .actors
            .iter()
            .map(|label| (box_width(label) - 1) / 2)
            .collect(),
        messages: vec![Vec::new(); n],
        notes: vec![Vec::new(); n],
        ..Spacing::default()
    };
    let mut caption = Vec::new();

    for m in d.messages() {
        let lines = wrap_words(&m.caption(), cap);
        let text = lines.iter().map(|l| l.width()).max().unwrap_or(0);
        caption.push(lines);
        if m.from == m.to {
            // A self-message hooks out to the right and comes back, with its
            // caption starting two cells clear of the lifeline.
            let want = (text + 2).max(SELF_HOOK + 1);
            match m.from + 1 < n {
                true => s.messages[m.from + 1].push((m.from, want + 1 + s.half[m.from + 1])),
                false => s.overhang = s.overhang.max(want),
            }
        } else {
            let (lo, hi) = (m.from.min(m.to), m.from.max(m.to));
            // Three more than the text: one cell of clearance either side of
            // the caption, and the run itself has to start after the sending
            // lifeline rather than on it.
            s.messages[hi].push((lo, text + 3));
        }
    }

    for note in d.events.iter().filter_map(|e| match e {
        Event::Note(n) => Some(n),
        _ => None,
    }) {
        let text = wrap_words(&note.text, cap)
            .iter()
            .map(|l| l.width())
            .max()
            .unwrap_or(0);
        let box_w = text + 4;
        let (lo, hi) = note.over;
        match note.place {
            // Beside a lifeline means beside it: the whole point of `left of`
            // is that the note is not on top of the participant to the left.
            Place::Left if lo > 0 => s.notes[lo].push((lo - 1, box_w + 1)),
            Place::Left => s.lead = s.lead.max(box_w),
            Place::Right if lo + 1 < n => s.notes[lo + 1].push((lo, box_w + 1)),
            Place::Right => s.note_overhang = s.note_overhang.max(box_w),
            // A note over one participant is centred on it and wants half its
            // width clear on either side; over two, it only has to be wide
            // enough to look like it covers the pair.
            Place::Over if lo == hi => {
                if lo > 0 {
                    s.notes[lo].push((lo - 1, box_w / 2 + 1));
                }
                if lo + 1 < n {
                    s.notes[lo + 1].push((lo, box_w / 2 + 1));
                } else {
                    s.note_overhang = s.note_overhang.max(box_w / 2);
                }
                if lo == 0 {
                    s.lead = s.lead.max(box_w / 2);
                }
            }
            Place::Over => s.notes[hi.max(lo)].push((lo.min(hi), box_w.saturating_sub(3))),
        }
    }

    let (x, grid) = s.sweep(pad, true);
    let (x, grid) = match grid <= width {
        true => (x, grid),
        false => s.sweep(pad, false),
    };
    (grid <= width).then_some(Plan {
        x,
        half: s.half,
        grid,
        pad,
        cap,
        caption,
    })
}

/// The widest captions this pane can afford, or `None` if even the narrowest
/// possible ones do not fit.
fn fit(d: &Diagram, pad: usize, width: usize) -> Option<Plan> {
    // Everything the cap applies to. Notes are in here as well as messages
    // because the cap they are wrapped to is the same one, and a search that
    // only looked at messages would settle on a cap of one word for a diagram
    // whose only text is a note.
    let text = || {
        d.messages()
            .map(Message::caption)
            .chain(d.events.iter().filter_map(|e| match e {
                Event::Note(n) => Some(n.text.clone()),
                _ => None,
            }))
    };
    // The floor is the longest word in any of it: below that, wrapping starts
    // breaking words, and a broken word is the one thing this layout will not
    // trade width for.
    let floor = text()
        .filter_map(|t| t.split_whitespace().map(|w| w.width()).max())
        .max()
        .unwrap_or(1)
        .max(1);
    let ceiling = text().map(|t| t.width()).max().unwrap_or(1).max(floor);

    let mut best = place(d, floor, pad, width)?;
    let (mut lo, mut hi) = (floor, ceiling);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        match place(d, mid, pad, width) {
            Some(plan) => {
                best = plan;
                lo = mid;
            }
            None => hi = mid - 1,
        }
    }
    Some(best)
}

/// Greedy word wrap that never breaks a word. A word wider than `cap` gets a
/// line to itself and makes that line over-wide, which is exactly what the
/// caller needs to see: it is the plan's job to widen for it, or to give up.
fn wrap_words(text: &str, cap: usize) -> Vec<String> {
    let cap = cap.max(1);
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match out.last_mut() {
            Some(line) if line.width() + 1 + word.width() <= cap => {
                line.push(' ');
                line.push_str(word);
            }
            _ => out.push(word.to_string()),
        }
    }
    out
}

// --- drawing -------------------------------------------------------------

struct Canvas {
    rows: Vec<Row>,
    grid: usize,
    /// How many activations deep each participant currently is.
    active: Vec<usize>,
    theme: &'static Theme,
}

fn lifelines(d: &Diagram, width: usize, theme: &'static Theme) -> Option<Rows> {
    let depth = d.depth();
    // Every nesting level owns one column on each side for its frame, plus one
    // more so the innermost frame is not touching a participant box.
    let pad = if depth == 0 { 0 } else { depth + 1 };
    let plan = fit(d, pad, width)?;

    let mut c = Canvas {
        rows: Vec::new(),
        grid: plan.grid,
        active: vec![0; d.actors.len()],
        theme,
    };
    c.title(d, width);
    c.boxes(d, &plan, true);
    c.events(d, &plan);
    c.boxes(d, &plan, false);
    // Height is the one dimension nothing above measures: the caps in `super`
    // bound how many bands there are, but a note wraps into as many rows as it
    // likes and the width search will happily buy a narrow cap with rows. Past
    // `ROW_CAP` the list says the same conversation in a fraction of them.
    (c.rows.len() <= ROW_CAP).then(|| c.rows.into_iter().map(Row::spans).collect())
}

impl Canvas {
    fn dim(&self) -> Style {
        Style::new().fg(self.theme.dim)
    }

    /// A fresh row with the lifelines already on it. Everything else is stamped
    /// over the top, which is what mermaid's own drawing does and the only way a
    /// note or a long caption can cross a lifeline it has nothing to do with.
    fn open_row(&mut self, plan: &Plan) -> usize {
        let mut row = Row::new(self.grid);
        for (i, &x) in plan.x.iter().enumerate() {
            // An activation is a heavier lifeline rather than a box beside it:
            // a box costs two columns the layout has already spent, and the
            // weight difference survives being read at a glance.
            let ch = if self.active.get(i).copied().unwrap_or(0) > 0 {
                '┃'
            } else {
                '│'
            };
            row.put(x, ch, self.dim());
        }
        self.rows.push(row);
        self.rows.len() - 1
    }

    fn title(&mut self, d: &Diagram, width: usize) {
        let Some(title) = &d.title else { return };
        let style = Style::new()
            .fg(self.theme.fg)
            .add_modifier(Modifier::BOLD);
        for line in wrap_words(title, width) {
            let mut row = Row::new(width);
            // Centred over the drawing rather than over the pane: the title
            // belongs to the diagram, not to the page it is sitting on.
            let at = self.grid.saturating_sub(line.width()) / 2;
            row.puts(at, &line, style);
            self.rows.push(row);
        }
    }

    /// The participant headers, and the same boxes again at the foot. Repeating
    /// them is how the lifelines are closed rather than left dangling, and in a
    /// diagram taller than the pane it is also the only way the reader still
    /// knows which column is which.
    fn boxes(&mut self, d: &Diagram, plan: &Plan, top: bool) {
        let dim = self.dim();
        // The name is text, so it is the body colour — the frame around it is
        // the part that recedes. See the module note: the palette is assigned
        // by role for both families at once, and a participant name is not a
        // different kind of thing from a node label.
        let name = Style::new().fg(self.theme.fg);
        let mut upper = Row::new(self.grid);
        let mut middle = Row::new(self.grid);
        let mut lower = Row::new(self.grid);

        for (i, label) in d.actors.iter().enumerate() {
            let (x, half) = (plan.x[i], plan.half[i]);
            let (left, right) = (x - half, x + half);
            upper.fill(left, right, '─', dim);
            upper.put(left, '┌', dim);
            upper.put(right, '┐', dim);
            middle.put(left, '│', dim);
            middle.put(right, '│', dim);
            middle.puts(left + 2, label, name);
            lower.fill(left, right, '─', dim);
            lower.put(left, '└', dim);
            lower.put(right, '┘', dim);
            if top {
                lower.put(x, '┬', dim);
            } else {
                upper.put(x, '┴', dim);
            }
        }
        self.rows.push(upper);
        self.rows.push(middle);
        self.rows.push(lower);
        if top {
            self.open_row(plan);
        }
    }

    fn events(&mut self, d: &Diagram, plan: &Plan) {
        // `(depth, the row its opening rule went on)`, so the frame's sides can
        // be filled in once its extent is actually known.
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut nth = 0usize;

        for event in &d.events {
            match event {
                Event::Message(m) => {
                    let caption = plan.caption.get(nth).cloned().unwrap_or_default();
                    nth += 1;
                    self.message(m, &caption, plan);
                }
                Event::Note(n) => self.note(n, plan),
                Event::Open(kind, label) => {
                    let depth = stack.len();
                    let at = self.rule(plan, depth, ('╭', '╮'), kind, label);
                    stack.push((depth, at));
                }
                Event::Section(kind, label) => {
                    if let Some(&(depth, _)) = stack.last() {
                        self.rule(plan, depth, ('├', '┤'), kind, label);
                    }
                }
                Event::Close => {
                    if let Some((depth, from)) = stack.pop() {
                        let to = self.rule(plan, depth, ('╰', '╯'), "", "");
                        self.sides(depth, from, to);
                    }
                }
                Event::Toggle(who, on) => self.toggle(*who, *on),
            }
        }
    }

    fn toggle(&mut self, who: usize, on: bool) {
        if let Some(count) = self.active.get_mut(who) {
            *count = if on { *count + 1 } else { count.saturating_sub(1) };
        }
    }

    fn message(&mut self, m: &Message, caption: &[String], plan: &Plan) {
        let text = Style::new().fg(self.theme.fg);
        let ink = Style::new().fg(m.arrow.ink(self.theme));
        // Only the tip is allowed a colour of its own, and only when the arrow
        // is one that severs — see [`Arrow::tip_ink`].
        let tip = Style::new().fg(m.arrow.tip_ink(self.theme));
        let (from, to) = (plan.x[m.from], plan.x[m.to]);

        if m.from == m.to {
            for line in caption {
                let at = self.open_row(plan);
                self.rows[at].puts(from + 2, line, text);
            }
            if m.toggle == Some(true) {
                self.toggle(m.to, true);
            }
            let out = self.open_row(plan);
            let back = self.open_row(plan);
            let end = (from + SELF_HOOK).min(self.grid.saturating_sub(1));
            self.rows[out].put(from, '├', ink);
            self.rows[out].fill(from + 1, end.saturating_sub(1), m.arrow.line(), ink);
            self.rows[out].put(end, '┐', ink);
            self.rows[back].put(from, m.arrow.tip(false), tip);
            self.rows[back].fill(from + 1, end.saturating_sub(1), m.arrow.line(), ink);
            self.rows[back].put(end, '┘', ink);
            if m.toggle == Some(false) {
                self.toggle(m.from, false);
            }
            return;
        }

        let (lo, hi) = (from.min(to), from.max(to));
        let rightward = to > from;
        for line in caption {
            let at = self.open_row(plan);
            // Centred in the span the plan reserved for it, which is always at
            // least two cells wider than the widest of these lines.
            let inner = hi.saturating_sub(lo + 1);
            let start = lo + 1 + inner.saturating_sub(line.width()) / 2;
            self.rows[at].puts(start.min(self.grid.saturating_sub(line.width())), line, text);
        }

        if m.toggle == Some(true) {
            self.toggle(m.to, true);
        }
        let at = self.open_row(plan);
        let row = &mut self.rows[at];
        row.fill(lo + 1, hi.saturating_sub(1), m.arrow.line(), ink);
        if rightward {
            row.put(lo, m.arrow.tail(true), ink);
            row.put(hi, m.arrow.tip(true), tip);
        } else {
            row.put(hi, m.arrow.tail(false), ink);
            row.put(lo, m.arrow.tip(false), tip);
        }
        if m.toggle == Some(false) {
            self.toggle(m.from, false);
        }
    }

    /// A note, drawn as a box laid over whatever it covers.
    ///
    /// The plan has usually already made room for this exact box — see
    /// [`Spacing::sweep`] — but it is allowed to have given up on doing so, so
    /// everything here is clamped into the room that actually exists. A note
    /// overlapping a lifeline is how mermaid draws them anyway; a note hanging
    /// off the edge of the pane would be a bug.
    fn note(&mut self, n: &Note, plan: &Plan) {
        let dim = self.dim();
        // Body colour, like every other piece of text here. A note used to be
        // drawn in `warn` — mermaid's notes are yellow — but `warn` is this
        // palette's attention colour, and a design document where every note is
        // shouting is a document whose actual warnings have nowhere left to go.
        // The box already says it is a note.
        let ink = Style::new().fg(self.theme.fg);
        let left = plan.pad;
        let right = self.grid.saturating_sub(1 + plan.pad);
        let room = (right + 1).saturating_sub(left);
        let (lo, hi) = (
            plan.x[n.over.0].min(plan.x[n.over.1]),
            plan.x[n.over.0].max(plan.x[n.over.1]),
        );

        let lines = wrap_words(&n.text, plan.cap.min(room.saturating_sub(4)));
        let text = lines.iter().map(|l| l.width()).max().unwrap_or(0);
        let want = match n.place {
            // A note over two participants is at least as wide as the pair it
            // is claiming to cover, or it does not look like it covers them.
            Place::Over => (text + 4).max(hi - lo + 3),
            _ => text + 4,
        };
        let width = want.min(room).max(2);
        let start = match n.place {
            Place::Over => (lo + hi) / 2 - (width / 2).min((lo + hi) / 2),
            Place::Left => plan.x[n.over.0].saturating_sub(width),
            Place::Right => plan.x[n.over.0] + 1,
        };
        let start = start.clamp(left, right.saturating_sub(width - 1).max(left));
        let end = (start + width - 1).min(right);

        let at = self.open_row(plan);
        self.rows[at].fill(start, end, '─', dim);
        self.rows[at].put(start, '┌', dim);
        self.rows[at].put(end, '┐', dim);
        for line in &lines {
            let at = self.open_row(plan);
            self.rows[at].fill(start, end, ' ', dim);
            self.rows[at].put(start, '│', dim);
            self.rows[at].put(end, '│', dim);
            self.rows[at].puts(start + 2, line, ink);
        }
        let at = self.open_row(plan);
        self.rows[at].fill(start, end, '─', dim);
        self.rows[at].put(start, '└', dim);
        self.rows[at].put(end, '┘', dim);
    }

    /// One horizontal rule of a block frame, returning the row it went on.
    fn rule(
        &mut self,
        plan: &Plan,
        depth: usize,
        corners: (char, char),
        kind: &str,
        label: &str,
    ) -> usize {
        let dim = self.dim();
        let key = Style::new().fg(self.theme.special);
        let body = Style::new().fg(self.theme.fg);
        let (left, right) = (depth, self.grid.saturating_sub(1 + depth));

        let mut row = Row::new(self.grid);
        row.fill(left, right, '─', dim);
        row.put(left, corners.0, dim);
        row.put(right, corners.1, dim);
        // The lifelines are crossed rather than cut. A frame that erased them
        // for a row reads as the diagram stopping and starting again.
        for &x in &plan.x {
            if x > left && x < right {
                row.put(x, '┼', dim);
            }
        }

        let at = self.rows.len();
        let room = right.saturating_sub(left + 4);
        // The `+ 1` the room has to hold is the space between the two.
        let inline = !kind.is_empty() && kind.width() + label.width() < room;
        if inline {
            row.puts(left + 2, &format!(" {kind}"), key);
            let after = left + 3 + kind.width();
            match label.is_empty() {
                true => row.put(after, ' ', dim),
                false => row.puts(after, &format!(" {label} "), body),
            }
            self.rows.push(row);
        } else {
            self.rows.push(row);
            // The label did not fit on the rule, so it goes inside the frame
            // rather than being cut down to fit — it is content, and the whole
            // point of the frame is to say what it is.
            //
            // On bare rows, not on lifelines: these belong to the frame's
            // heading rather than to the conversation, and a lifeline poking
            // through the middle of a wrapped label reads as a message that
            // was never sent.
            if !kind.is_empty() {
                let text = match label.is_empty() {
                    true => kind.to_string(),
                    false => format!("{kind} {label}"),
                };
                for line in wrap_words(&text, room.max(1)) {
                    let mut row = Row::new(self.grid);
                    row.puts(left + 2, &line, key);
                    self.rows.push(row);
                }
            }
        }
        at
    }

    /// The two sides of a frame, drawn once its extent is known.
    fn sides(&mut self, depth: usize, from: usize, to: usize) {
        let dim = self.dim();
        let right = self.grid.saturating_sub(1 + depth);
        for row in self.rows.iter_mut().take(to).skip(from + 1) {
            row.under(depth, '│', dim);
            row.under(right, '│', dim);
        }
    }
}

// --- the list ------------------------------------------------------------

/// The diagram as a numbered list, for a pane the lifelines will not fit.
///
/// Everything the drawing carries is still here — the cast, the arrow kinds,
/// the notes, the block labels — with structure carried by indentation instead
/// of by frames. It always fits, because every row is wrapped to the pane.
fn as_list(d: &Diagram, width: usize, theme: &'static Theme) -> Option<Rows> {
    let dim = Style::new().fg(theme.dim);
    // Names, message text, note text and block labels are all text, and all
    // the body colour. What is dimmed here is only the scaffolding this layout
    // adds and the document never wrote: the row numbers, `Note over`, and the
    // `activate` a lifeline drawing would have said with a thicker line.
    let body = Style::new().fg(theme.fg);
    let key = Style::new().fg(theme.special);

    let mut out: Rows = Vec::new();
    let push = |out: &mut Rows, depth: usize, spans: Vec<Span<'static>>| {
        let indent = (2 * depth).min(width.saturating_sub(CRAMPED));
        let first = match indent {
            0 => Vec::new(),
            n => vec![Span::raw(" ".repeat(n))],
        };
        // Continuations hang two past their own row, so a wrapped message is
        // never mistaken for the next one.
        let rest = vec![Span::raw(" ".repeat(indent + 2))];
        out.extend(
            wrap_spans(spans, width, &first, &rest)
                .into_iter()
                .map(|line| line.spans),
        );
    };

    if let Some(title) = &d.title {
        push(
            &mut out,
            0,
            vec![Span::styled(
                title.clone(),
                body.add_modifier(Modifier::BOLD),
            )],
        );
    }
    if !d.actors.is_empty() {
        push(&mut out, 0, vec![Span::styled(d.actors.join(", "), body)]);
    }

    let label = |at: usize| -> String {
        d.actors.get(at).cloned().unwrap_or_else(|| "?".to_string())
    };
    let mut depth = 0usize;
    let mut nth = 0usize;

    for event in &d.events {
        match event {
            Event::Message(m) => {
                nth += 1;
                // The colon rides on the name it belongs to rather than being
                // its own span: the wrapper breaks between spans as readily as
                // between words, and a row consisting of one colon is a row.
                let (to, colon) = match m.text.is_empty() {
                    true => (label(m.to), None),
                    false => (format!("{}:", label(m.to)), Some(m.text.clone())),
                };
                let mut spans = vec![
                    // The document's own number when `autonumber` dealt one —
                    // that is content and reads as content; this layout's index
                    // otherwise, which is scaffolding and recedes.
                    Span::styled(
                        format!("{}. ", m.number.unwrap_or(nth)),
                        if m.number.is_some() { body } else { dim },
                    ),
                    Span::styled(label(m.from), body),
                    Span::raw(" "),
                    // One span for the whole token, at the tip's colour: the
                    // wrapper breaks between spans, and an arrow split across
                    // two rows says nothing at all.
                    Span::styled(m.arrow.token(), Style::new().fg(m.arrow.tip_ink(theme))),
                    Span::raw(" "),
                    Span::styled(to, body),
                ];
                if let Some(text) = colon {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(text, body));
                }
                push(&mut out, depth, spans);
            }
            Event::Note(n) => {
                let where_ = match n.place {
                    Place::Left => "Note left of ",
                    Place::Right => "Note right of ",
                    Place::Over => "Note over ",
                };
                let who = match n.over.0 == n.over.1 {
                    true => label(n.over.0),
                    false => format!("{}, {}", label(n.over.0), label(n.over.1)),
                };
                let mut spans = vec![Span::styled(where_, dim), Span::styled(who, body)];
                if !n.text.is_empty() {
                    spans.push(Span::styled(":", body));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(n.text.clone(), body));
                }
                push(&mut out, depth, spans);
            }
            Event::Open(kind, text) => {
                let mut spans = vec![Span::styled(*kind, key)];
                if !text.is_empty() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(text.clone(), body));
                }
                push(&mut out, depth, spans);
                depth += 1;
            }
            Event::Section(kind, text) => {
                let mut spans = vec![Span::styled(*kind, key)];
                if !text.is_empty() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(text.clone(), body));
                }
                push(&mut out, depth.saturating_sub(1), spans);
            }
            Event::Close => {
                depth = depth.saturating_sub(1);
                // The same colour its `loop` got. `end` is the other half of
                // one keyword, and colouring the halves differently is the sort
                // of seam a reader notices without being able to say why.
                push(&mut out, depth, vec![Span::styled("end", key)]);
            }
            Event::Toggle(who, on) => {
                let word = if *on { "activate " } else { "deactivate " };
                push(
                    &mut out,
                    depth,
                    vec![Span::styled(word, dim), Span::styled(label(*who), body)],
                );
            }
        }
    }
    // Past here the list has stopped being a summary of the diagram and become
    // a transcript of it — and the transcript is the fence, which the caller
    // still has and which is never wrong.
    (out.len() <= LIST_CAP).then_some(out)
}

#[cfg(test)]
mod tests {
    use ratatui::text::Span;

    use super::super::tests::{assert_fits, assert_keeps, draw};
    use super::super::{MAX_EDGES, MAX_NODES};
    use super::{ROW_CAP, Rows, keeps};

    /// The drawing, or a failure that names the input rather than the line.
    fn rows(src: &str, width: usize) -> Vec<String> {
        draw(src, width).unwrap_or_else(|| panic!("{src:?} declined at {width} columns"))
    }

    const HELLO: &str = "sequenceDiagram\n  Alice->>Bob: Hello Bob\n  Bob-->>Alice: Hi Alice\n";

    #[test]
    fn two_participants_talking_become_boxes_over_their_lifelines() {
        assert_eq!(
            rows(HELLO, 48),
            [
                "┌───────┐    ┌─────┐",
                "│ Alice │    │ Bob │",
                "└───┬───┘    └──┬──┘",
                "    │           │",
                "    │ Hello Bob │",
                "    ├───────────▶",
                "    │ Hi Alice  │",
                "    ◀┄┄┄┄┄┄┄┄┄┄┄┤",
                "┌───┴───┐    ┌──┴──┐",
                "│ Alice │    │ Bob │",
                "└───────┘    └─────┘",
            ]
        );
    }

    #[test]
    fn every_arrow_mermaid_spells_reads_as_a_run_and_a_head_of_its_own() {
        // The whole table in one diagram, because the failure being guarded
        // against here is two of them drawing the same and nobody noticing.
        let src = "sequenceDiagram\n  A->B: p\n  A-->B: q\n  A->>B: r\n  A-->>B: s\n  A-xB: t\n  \
                   A--xB: u\n  A-)B: v\n  A--)B: w\n  A<<->>B: y\n  A<<-->>B: z\n";
        let drawn = rows(src, 40);
        let arrows: Vec<&str> = drawn
            .iter()
            .skip(5)
            .step_by(2)
            .take(10)
            .map(String::as_str)
            .collect();
        assert_eq!(
            arrows,
            [
                "  ├─────┤", // `->`     solid, no head
                "  ├┄┄┄┄┄┤", // `-->`    dotted, no head
                "  ├─────▶", // `->>`    solid, arrowhead
                "  ├┄┄┄┄┄▶", // `-->>`   dotted, arrowhead
                "  ├─────╳", // `-x`     solid, cross — the box-drawing one
                "  ├┄┄┄┄┄╳", // `--x`    dotted, cross
                "  ├─────>", // `-)`     solid, open
                "  ├┄┄┄┄┄>", // `--)`    dotted, open
                "  ◀─────▶", // `<<->>`  a head at both ends
                "  ◀┄┄┄┄┄▶", // `<<-->>` the same, dotted
            ]
        );
    }

    #[test]
    fn an_undeclared_participant_takes_its_place_at_first_mention() {
        // Mermaid's own ordering rule, and one a reader notices immediately
        // when it is wrong: every column is in the wrong place all the way down.
        let src = "sequenceDiagram\n  B->>A: one\n  C->>A: two\n";
        assert_eq!(rows(src, 12)[0], "B, A, C");
    }

    #[test]
    fn a_declared_participant_keeps_its_place_and_wears_its_alias() {
        let src = "sequenceDiagram\n  participant B as Bravo\n  A->>B: hi\n";
        assert_eq!(
            rows(src, 40),
            [
                "┌───────┐ ┌───┐",
                "│ Bravo │ │ A │",
                "└───┬───┘ └─┬─┘",
                "    │       │",
                "    │  hi   │",
                "    ◀───────┤",
                "┌───┴───┐ ┌─┴─┐",
                "│ Bravo │ │ A │",
                "└───────┘ └───┘",
            ]
        );
    }

    #[test]
    fn a_message_a_participant_sends_itself_hooks_out_and_comes_back() {
        assert_eq!(
            rows("sequenceDiagram\n  A->>A: think\n", 30),
            [
                "┌───┐",
                "│ A │",
                "└─┬─┘",
                "  │",
                "  │ think",
                "  ├──┐",
                "  ◀──┘",
                "┌─┴─┐",
                "│ A │",
                "└───┘",
            ]
        );
    }

    #[test]
    fn each_note_placement_puts_its_box_where_it_says_to() {
        let src = "sequenceDiagram\n  Note left of A: on the left\n  \
                   Note right of B: on the right\n  Note over A,B: over both\n";
        assert_eq!(
            rows(src, 44),
            [
                "               ┌───┐     ┌───┐",
                "               │ A │     │ B │",
                "               └─┬─┘     └─┬─┘",
                "                 │         │",
                "  ┌─────────────┐│         │",
                "  │ on the left ││         │",
                "  └─────────────┘│         │",
                "                 │         │┌──────────────┐",
                "                 │         ││ on the right │",
                "                 │         │└──────────────┘",
                "                ┌───────────┐",
                "                │ over both │",
                "                └───────────┘",
                "               ┌─┴─┐     ┌─┴─┐",
                "               │ A │     │ B │",
                "               └───┘     └───┘",
            ]
        );
    }

    #[test]
    fn the_note_keyword_is_read_in_either_case_the_way_mermaids_lexer_reads_it() {
        for src in [
            "sequenceDiagram\n  Note over A: hi\n",
            "sequenceDiagram\n  note over A: hi\n",
            "sequenceDiagram\n  NOTE OVER A: hi\n",
        ] {
            assert!(rows(src, 30).iter().any(|row| row.contains("hi")), "{src:?}");
        }
    }

    #[test]
    fn blocks_are_framed_across_the_lifelines_and_nest() {
        let src = "sequenceDiagram\n  loop retry\n    A->>B: ping\n    opt slow\n      \
                   B-->>A: wait\n    end\n  end\n";
        assert_eq!(
            rows(src, 44),
            [
                "   ┌───┐  ┌───┐",
                "   │ A │  │ B │",
                "   └─┬─┘  └─┬─┘",
                "     │      │",
                "╭─ loop retry ───╮",
                "│    │ ping │    │",
                "│    ├──────▶    │",
                "│╭─ opt slow ───╮│",
                "││   │ wait │   ││",
                "││   ◀┄┄┄┄┄┄┤   ││",
                "│╰───┼──────┼───╯│",
                "╰────┼──────┼────╯",
                "   ┌─┴─┐  ┌─┴─┐",
                "   │ A │  │ B │",
                "   └───┘  └───┘",
            ]
        );
    }

    #[test]
    fn a_blocks_section_rules_across_it_without_breaking_the_frame() {
        let src =
            "sequenceDiagram\n  alt found\n    A->>B: yes\n  else missing\n    A->>B: no\n  end\n";
        let drawn = rows(src, 40);
        assert!(drawn.contains(&"╭─ alt found ─╮".to_string()), "{drawn:#?}");
        assert!(drawn.contains(&"├───┼─────┼───┤".to_string()), "{drawn:#?}");
        assert_keeps(&drawn, &["missing"], "the else label");
    }

    #[test]
    fn a_block_label_too_long_for_its_rule_moves_inside_the_frame() {
        // Cutting it down to fit would lose words, and the label is the only
        // thing the frame exists to say.
        let src = "sequenceDiagram\n  loop a label far too long to sit on its own rule\n    \
                   A->>B: x\n  end\n";
        assert_eq!(
            rows(src, 24),
            [
                "  ┌───┐ ┌───┐",
                "  │ A │ │ B │",
                "  └─┬─┘ └─┬─┘",
                "    │     │",
                "╭───┼─────┼───╮",
                "│ loop a      │",
                "│ label far   │",
                "│ too long    │",
                "│ to sit on   │",
                "│ its own     │",
                "│ rule        │",
                "│   │  x  │   │",
                "│   ├─────▶   │",
                "╰───┼─────┼───╯",
                "  ┌─┴─┐ ┌─┴─┐",
                "  │ A │ │ B │",
                "  └───┘ └───┘",
            ]
        );
    }

    #[test]
    fn every_block_keyword_mermaid_has_is_drawn_rather_than_declined() {
        for (src, label) in [
            ("sequenceDiagram\n  loop each\n  A->>B: x\n  end\n", "loop each"),
            ("sequenceDiagram\n  opt maybe\n  A->>B: x\n  end\n", "opt maybe"),
            (
                "sequenceDiagram\n  alt one\n  A->>B: x\n  else two\n  A->>B: y\n  end\n",
                "alt one",
            ),
            (
                "sequenceDiagram\n  par one\n  A->>B: x\n  and two\n  A->>B: y\n  end\n",
                "par one",
            ),
            (
                "sequenceDiagram\n  critical go\n  A->>B: x\n  option fail\n  A->>B: y\n  end\n",
                "critical go",
            ),
            ("sequenceDiagram\n  break bad\n  A->>B: x\n  end\n", "break bad"),
            (
                "sequenceDiagram\n  rect rgb(0,0,0)\n  A->>B: x\n  end\n",
                "rect rgb(0,0,0)",
            ),
        ] {
            assert_keeps(&rows(src, 40), &[label], src);
        }
    }

    #[test]
    fn autonumber_counts_from_where_it_is_told_and_by_the_step_it_is_given() {
        let plain = rows("sequenceDiagram\n  autonumber\n  A->>B: one\n  A->>B: two\n", 30);
        assert!(plain.contains(&"  │ 1. one │".to_string()), "{plain:#?}");
        assert!(plain.contains(&"  │ 2. two │".to_string()), "{plain:#?}");

        let offset = rows("sequenceDiagram\n  autonumber 10 5\n  A->>B: one\n  A->>B: two\n", 30);
        assert!(offset.contains(&"  │ 10. one │".to_string()), "{offset:#?}");
        assert!(offset.contains(&"  │ 15. two │".to_string()), "{offset:#?}");

        // ...and stops counting when told to, rather than numbering the rest.
        let off = rows(
            "sequenceDiagram\n  autonumber\n  A->>B: one\n  autonumber off\n  A->>B: two\n",
            30,
        );
        assert!(off.contains(&"  │ 1. one │".to_string()), "{off:#?}");
        assert!(off.contains(&"  │  two   │".to_string()), "{off:#?}");
    }

    #[test]
    fn an_activation_thickens_the_lifeline_it_is_on() {
        // The `+` and `-` are syntax; what has to survive them is the text.
        let src = "sequenceDiagram\n  A->>+B: call\n  B-->>-A: back\n";
        assert_eq!(
            rows(src, 30),
            [
                "┌───┐  ┌───┐",
                "│ A │  │ B │",
                "└─┬─┘  └─┬─┘",
                "  │      │",
                "  │ call │",
                "  ├──────▶",
                "  │ back ┃",
                "  ◀┄┄┄┄┄┄┤",
                "┌─┴─┐  ┌─┴─┐",
                "│ A │  │ B │",
                "└───┘  └───┘",
            ]
        );

        // The long form says the same thing.
        let src = "sequenceDiagram\n  A->>B: call\n  activate B\n  B->>A: back\n  deactivate B\n";
        assert!(rows(src, 30).iter().any(|row| row.contains('┃')));
    }

    #[test]
    fn a_title_sits_over_the_drawing_rather_than_over_the_pane() {
        let drawn = rows("sequenceDiagram\n  title A day\n  A->>B: x\n", 30);
        assert_eq!(drawn[0], "   A day");
        // `title: text` is written in the wild as often as `title text`.
        let colon = rows("sequenceDiagram\n  title: A day\n  A->>B: x\n", 30);
        assert_eq!(colon, drawn);
    }

    #[test]
    fn a_message_longer_than_the_pane_wraps_over_its_own_arrow() {
        let src =
            "sequenceDiagram\n  Alice->>Bob: a message far too long for this pane to hold at all\n";
        assert_eq!(
            rows(src, 30),
            [
                "┌───────┐             ┌─────┐",
                "│ Alice │             │ Bob │",
                "└───┬───┘             └──┬──┘",
                "    │                    │",
                "    │ a message far too  │",
                "    │ long for this pane │",
                "    │   to hold at all   │",
                "    ├────────────────────▶",
                "┌───┴───┐             ┌──┴──┐",
                "│ Alice │             │ Bob │",
                "└───────┘             └─────┘",
            ]
        );
    }

    #[test]
    fn wide_characters_are_measured_in_cells_rather_than_in_characters() {
        assert_eq!(
            rows("sequenceDiagram\n  日本->>語版: メッセージ\n", 30),
            [
                "┌───────┐    ┌───────┐",
                "│ 日本  │    │ 語版  │",
                "└───┬───┘    └───┬───┘",
                "    │            │",
                "    │ メッセージ │",
                "    ├────────────▶",
                "┌───┴───┐    ┌───┴───┐",
                "│ 日本  │    │ 語版  │",
                "└───────┘    └───────┘",
            ]
        );
    }

    #[test]
    fn a_message_with_no_text_still_draws_its_arrow() {
        assert_eq!(
            rows("sequenceDiagram\n  A->>B:\n", 20),
            [
                "┌───┐ ┌───┐",
                "│ A │ │ B │",
                "└─┬─┘ └─┬─┘",
                "  │     │",
                "  ├─────▶",
                "┌─┴─┐ ┌─┴─┐",
                "│ A │ │ B │",
                "└───┘ └───┘",
            ]
        );
    }

    #[test]
    fn the_lifelines_give_way_to_a_numbered_list_the_column_before_they_would_not_fit() {
        let src = "sequenceDiagram\n  A->>B: x\n";
        assert_eq!(rows(src, 11)[0], "┌───┐ ┌───┐");
        assert_eq!(rows(src, 10), ["A, B", "1. A ─▶ B:", "  x"]);
    }

    #[test]
    fn the_list_keeps_the_blocks_the_arrows_and_the_indentation() {
        let src = "sequenceDiagram\n  alt yes\n    A->>B: go\n  else no\n    A->>B: stop\n  end\n";
        assert_eq!(
            rows(src, 14),
            [
                "A, B",
                "alt yes",
                "  1. A ─▶ B:",
                "    go",
                "else no",
                "  2. A ─▶ B:",
                "    stop",
                "end",
            ]
        );
    }

    #[test]
    fn the_list_keeps_the_notes_and_still_says_where_they_pointed() {
        // Thirteen columns is under `CRAMPED`, so the indent has already been
        // squeezed to one — the structure survives on less than it wants.
        let src = "sequenceDiagram\n  loop retry\n    A->>B: ping\n    Note over A,B: both\n  end\n";
        assert_eq!(
            rows(src, 13),
            [
                "A, B",
                "loop retry",
                " 1. A ─▶ B:",
                "   ping",
                " Note over A,",
                "   B: both",
                "end",
            ]
        );
    }

    /// One diagram with every shape of statement in it, for the width sweep.
    const RICH: &str = "sequenceDiagram\n  \
        title A day in the life\n  \
        autonumber\n  \
        participant W as Watcher\n  \
        actor V as Viewer\n  \
        W->>+V: file changed\n  \
        Note over V: repaints\n  \
        loop every save\n    \
        V-->>-W: done\n    \
        alt fresh\n      \
        W->>W: think\n    \
        else stale\n      \
        W--xV: give up\n    \
        end\n  \
        end\n";

    const RICH_WORDS: &[&str] = &[
        "A day in the life",
        "Watcher",
        "Viewer",
        "file changed",
        "repaints",
        "every save",
        "done",
        "fresh",
        "think",
        "stale",
        "give up",
    ];

    #[test]
    fn nothing_is_lost_at_any_width_a_sequence_diagram_draws_at() {
        for width in 4..=80 {
            let Some(drawn) = draw(RICH, width) else {
                continue;
            };
            let what = format!("the rich diagram at {width}");
            assert_fits(&drawn, width, &what);
            assert_keeps(&drawn, RICH_WORDS, &what);
        }
    }

    #[test]
    fn a_pane_that_can_hold_the_longest_word_draws_something() {
        // The list only gives up below the width of a single word. Above that
        // there is always an answer, and it is never the source.
        for width in 12..=80 {
            assert!(draw(RICH, width).is_some(), "nothing drawn at {width}");
        }
    }

    #[test]
    fn a_statement_this_cannot_read_declines_the_whole_diagram() {
        // Never a partial drawing: the source is always true, and a diagram
        // missing a message is worse than one nobody drew.
        for src in [
            "sequenceDiagram\n  Alice speaks to Bob\n",
            "sequenceDiagram\n  Alice->>Bob\n",
            "sequenceDiagram\n  ->>Bob: no sender\n",
            "sequenceDiagram\n  A->>: no target\n",
            "sequenceDiagram\n  Note over: nobody\n",
            "sequenceDiagram\n  Note beside A: nowhere\n",
            "sequenceDiagram\n  Note over A,B,C: too many\n",
            "sequenceDiagram\n  autonumber soon\n",
            "sequenceDiagram\n  activate\n",
        ] {
            assert!(draw(src, 60).is_none(), "{src:?} should have declined");
        }
    }

    #[test]
    fn a_participant_that_comes_or_goes_is_left_as_source() {
        // Creation and destruction are things that *change* partway down the
        // page, and a static column of lifelines cannot say either of them.
        for src in [
            "sequenceDiagram\n  create participant C\n  A->>C: hi\n",
            "sequenceDiagram\n  A->>C: hi\n  destroy C\n",
            "sequenceDiagram\n  box Team\n  participant A\n  end\n  A->>B: hi\n",
            "sequenceDiagram\n  A->>B: hi\n  link A: Dashboard @ https://x.dev\n",
        ] {
            assert!(draw(src, 60).is_none(), "{src:?} should have declined");
        }
    }

    #[test]
    fn a_block_that_never_closes_declines_the_way_mermaid_refuses_it() {
        assert!(draw("sequenceDiagram\n  loop forever\n    A->>B: x\n", 60).is_none());
        assert!(draw("sequenceDiagram\n  A->>B: x\n  end\n", 60).is_none());
    }

    #[test]
    fn a_section_keyword_outside_the_block_it_sections_declines() {
        assert!(draw("sequenceDiagram\n  loop x\n  else y\n  end\n", 60).is_none());
        assert!(draw("sequenceDiagram\n  alt x\n  and y\n  end\n", 60).is_none());
        assert!(draw("sequenceDiagram\n  else y\n", 60).is_none());
    }

    #[test]
    fn an_empty_diagram_is_worth_more_as_its_own_source() {
        assert!(draw("sequenceDiagram\n", 60).is_none());
        assert!(draw("sequenceDiagram\n  %% only a comment\n", 60).is_none());
    }

    #[test]
    fn a_diagram_past_the_caps_is_left_as_source_rather_than_laid_out() {
        let many = (0..MAX_NODES + 1)
            .map(|i| format!("  participant p{i}\n"))
            .collect::<String>();
        assert!(draw(&format!("sequenceDiagram\n{many}"), 60).is_none());

        let loud = (0..MAX_EDGES + 1)
            .map(|i| format!("  A->>B: m{i}\n"))
            .collect::<String>();
        assert!(draw(&format!("sequenceDiagram\n{loud}"), 60).is_none());
    }

    #[test]
    fn notes_and_blocks_are_counted_against_the_cap_the_way_messages_are() {
        // Every one of these claims vertical space, so every one of them is a
        // band. Counting only messages left two thirds of the grammar able to
        // ask for an unbounded layout: thirty kilobytes of `Note over A:` is
        // legal mermaid and used to cost four times what a diagram sitting at
        // both caps costs.
        let notes = (0..MAX_EDGES + 1)
            .map(|i| format!("  Note over A: n{i}\n"))
            .collect::<String>();
        assert!(draw(&format!("sequenceDiagram\n{notes}"), 60).is_none());

        let blocks = (0..MAX_EDGES + 1)
            .map(|i| format!("  loop b{i}\n  end\n"))
            .collect::<String>();
        assert!(draw(&format!("sequenceDiagram\n{blocks}"), 60).is_none());

        // ...and they are counted *together*, or a diagram simply spends its
        // budget in whichever currency is not being watched.
        let mixed = (0..MAX_EDGES / 2 + 1)
            .map(|i| format!("  A->>B: m{i}\n  Note over A: n{i}\n"))
            .collect::<String>();
        assert!(draw(&format!("sequenceDiagram\n{mixed}"), 60).is_none());

        // One under the cap still draws, so the cap is a cap and not a ban.
        let ok = (0..MAX_EDGES)
            .map(|i| format!("  Note over A: n{i}\n"))
            .collect::<String>();
        assert!(draw(&format!("sequenceDiagram\n{ok}"), 60).is_some());
    }

    #[test]
    fn a_drawing_taller_than_three_screens_becomes_the_list_instead() {
        // `ROW_CAP` is about the shape of the answer rather than the clock:
        // past it the lifelines have stopped being a picture, and the list says
        // the same conversation in one row per message.
        let long = (0..150)
            .map(|i| format!("  A->>B: m{i}\n"))
            .collect::<String>();
        let drawn = rows(&format!("sequenceDiagram\n{long}"), 60);
        assert!(drawn.len() <= ROW_CAP, "{} rows", drawn.len());
        assert_eq!(drawn[0], "A, B", "should have fallen to the list");
        assert!(drawn.iter().any(|row| row.contains("m149")));
    }

    #[test]
    fn autonumber_at_the_top_of_usize_does_not_overflow() {
        // Two lines of any document reach this. Saturating rather than
        // declining: the number is a decoration on a caption whose text is the
        // content, and throwing the diagram away over a counter would cost the
        // reader everything else in the file.
        let src = "sequenceDiagram\n  autonumber 18446744073709551615\n  A->>B: x\n  B->>A: y\n";
        let drawn = rows(src, 60);
        assert_keeps(&drawn, &["x", "y"], "either side of the counter stopping");
        assert!(drawn.iter().any(|row| row.contains("18446744073709551615")));

        // A start `usize` cannot hold is a different thing, and is refused
        // where it is read rather than papered over.
        assert!(draw("sequenceDiagram\n  autonumber 99999999999999999999\n  A->>B: x\n", 60).is_none());
        assert!(draw("sequenceDiagram\n  autonumber 1 18446744073709551615\n  A->>B: x\n  B->>A: y\n", 60).is_some());
    }

    #[test]
    fn a_participant_and_an_actor_draw_the_same_way() {
        // Mermaid draws `actor` as a stick figure. On a character grid that is
        // either four rows of header or a glyph whose width a terminal is free
        // to disagree about, and it changes nothing else about the diagram —
        // see the argument where the two keywords are read. What must not
        // happen is `actor` being declined, so this pins the equivalence.
        let boxes = rows("sequenceDiagram\n  participant V as Viewer\n  A->>V: hi\n", 40);
        let stick = rows("sequenceDiagram\n  actor V as Viewer\n  A->>V: hi\n", 40);
        assert_eq!(boxes, stick);
        assert!(boxes[1].contains("Viewer"));
    }

    #[test]
    fn a_combining_mark_rides_on_the_character_it_modifies() {
        // Devanagari and Hebrew are full of zero-width marks. Dropping them
        // lost no content — the gate caught it — but it caught it by falling to
        // the list, so the same diagram drew boxes in Latin and a list in
        // Hindi. `flow` attaches them too.
        let src = "sequenceDiagram\n  A->>B: नमस्ते there\n";
        let drawn = rows(src, 40);
        assert_eq!(drawn[0], "┌───┐        ┌───┐", "should still be lifelines");
        assert_keeps(&drawn, &["नमस्ते", "there"], "a caption with marks in it");
        assert_fits(&drawn, 40, "a caption with marks in it");
        // The marks cost no cells: they are in the row as characters and not in
        // its width, which is the whole of what "attached" means here.
        let caption = drawn
            .iter()
            .find(|row| row.contains("नमस्ते"))
            .expect("the caption is on some row");
        assert!(
            caption.chars().count() > unicode_width::UnicodeWidthStr::width(caption.as_str()),
            "{caption:?} spent a cell on a mark"
        );
    }

    #[test]
    fn a_word_hiding_inside_a_longer_one_does_not_satisfy_the_gate() {
        // The gate decides lifelines → list → `None`, so a spurious pass is a
        // clipped drawing shipping. `no` is inside `Note`, `end` is inside
        // `friend`, and both of those words are ones this module writes itself.
        let drawn: Rows = vec![vec![Span::raw("│ Note over friend │")]];
        assert!(keeps(&drawn, &["Note".to_string(), "friend".to_string()]));
        assert!(!keeps(&drawn, &["no".to_string()]));
        assert!(!keeps(&drawn, &["end".to_string()]));
        assert!(!keeps(&drawn, &["frien".to_string()]));

        // The one glue this module applies to a word is the list's `:` and `,`
        // after a name, and the gate has to see through exactly that much.
        let listed: Rows = vec![vec![Span::raw("1. A ─▶ Bob: hi")], vec![Span::raw("X, Y")]];
        for word in ["Bob", "hi", "X", "Y"] {
            assert!(keeps(&listed, &[word.to_string()]), "{word:?}");
        }
    }



    #[test]
    fn every_glyph_the_drawing_uses_is_one_cell_wide() {
        // The layout is counted in cells, so a glyph a terminal draws in two
        // would shear every row it appears in. The arrowheads and `╳` are the
        // risk — all East Asian Ambiguous — and this is the assertion that
        // records which set was checked.
        for glyph in "─│┄┃┌┐└┘┬┴├┤┼╭╮╰╯▶◀><╳".chars() {
            assert_eq!(
                unicode_width::UnicodeWidthChar::width(glyph),
                Some(1),
                "{glyph:?} is not one cell"
            );
        }
    }
}
