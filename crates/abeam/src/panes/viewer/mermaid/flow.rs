//! `graph` / `flowchart`: parse, rank, and draw.
//!
//! ## Twelve shapes, three frames
//!
//! Mermaid gives a node twelve outlines — rectangle, round, stadium,
//! subroutine, cylinder, circle, flag, rhombus, hexagon, two parallelograms and
//! two trapezoids. A browser draws all twelve; a character grid draws about
//! three of them legibly. The first attempt here spent a different corner glyph
//! on each family and produced boxes a reader had to *count the corners of* to
//! tell apart, which is not a distinction anybody was going to make while
//! skimming a design document. So the twelve are collapsed onto the three
//! differences that survive being made out of `─` and `│`:
//!
//! - **square corners** for the process-like shapes (rect, subroutine, flag,
//!   the parallelograms and the trapezoids) — the default, and what most nodes
//!   in most diagrams are;
//! - **rounded corners** for the round-edged family (round, stadium, cylinder,
//!   circle), which is the same thing those four are saying in a browser;
//! - **a double rule** for `{}` and `{{}}`. A rhombus is the one shape in a
//!   flowchart that carries meaning rather than decoration — it is where the
//!   diagram branches — so it gets the one frame that is unmistakable at a
//!   glance rather than on inspection.
//!
//! Line style is kept, because three stroke sets *are* distinguishable: solid
//! `─│`, dotted `┄┊` and thick `━┃`. An author who drew one edge dotted meant
//! something by it.
//!
//! ## Layered, with dummies
//!
//! Nodes are ranked by longest path and drawn a rank at a time, ranks down the
//! page for `TD`/`BT` and across it for `LR`/`RL`. An edge that spans more than
//! one rank gets a one-column **dummy** slot in each rank it passes through,
//! which is the classic Sugiyama trick and is here for a very concrete reason:
//! without it a connector has to be routed *through* a rank it does not belong
//! to, and the only two options at that point are drawing over a box or
//! reserving a corridor nobody else may use. A dummy is that corridor, and it
//! costs one column and draws as the line itself.
//!
//! Cycles are broken first — a layered layout needs a DAG or it ranks forever.
//! The back-edges found by the DFS are not dropped: they are drawn under the
//! diagram as `↩ from ─ label ─▶ to` rows, because a back-edge is exactly the
//! edge a reader is looking for in a retry loop and the alternative was routing
//! it up the side of the drawing, which at forty columns crosses everything.
//!
//! ## When this gives up
//!
//! Two ways, and they mean different things:
//!
//! - **the outline** ([`super::outline`]) when the boxes will not fit the
//!   width, when the drawing would run past [`ROW_CAP`] rows, when a word is
//!   wider than the box it would go in, or when an edge label cannot be placed
//!   without landing on top of another connector. Nothing is lost — the outline
//!   says the same graph with the labels wrapped instead of boxed;
//! - **`None`** when a statement cannot be parsed at all, when the diagram holds
//!   a `subgraph` or a `click` (both carry text this cannot draw, and the module
//!   note's rule is that a partial drawing is never the answer), or when the
//!   caps in [`super`] are exceeded. `style`, `classDef`, `class` and
//!   `linkStyle` are the exception: they are skipped in silence, because the
//!   reader loses nothing when a colour this pane would not have honoured
//!   anyway goes missing.

use std::collections::{HashMap, VecDeque};

use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::{Direction, Rows, lex};
use crate::panes::viewer::theme::Theme;
use crate::text::wrap;

/// Most rows a *box* drawing may take before the outline is used instead.
///
/// Not a correctness bound — the caps in [`super`] are what stop a generated
/// graph from being laid out at all. This one is about the shape of the answer:
/// a diagram three screens tall has stopped being a diagram, and the outline
/// says the same thing in one row per edge. The outline has its own, higher cap
/// where it declines instead, because past that there is no rendering left that
/// is better than the source.
const ROW_CAP: usize = 240;

/// Widest a node label is allowed to be before it wraps inside its own box.
///
/// A box is only readable if the eye can find the next one, and a
/// sixty-character node in a sixty-column pane is a paragraph with a border.
const LABEL_MAX: usize = 24;

/// Blank cells between two slots in the same rank. One is enough to keep two
/// borders apart; two is enough to see that they *are* two.
const GAP: usize = 2;

/// Narrowest box content this will draw. Below it the frame costs more cells
/// than it says, and the outline is the better answer.
const MIN_CONTENT: usize = 3;

/// Draw a flowchart body into at most `width` columns.
///
/// `None` declines the whole diagram — see the module note on why a partial
/// drawing is never the answer.
pub fn render(
    direction: Direction,
    body: &[String],
    width: usize,
    theme: &'static Theme,
) -> Option<Rows> {
    let graph = Graph::parse(body)?;
    // A header and nothing else. There is no drawing of no nodes, and an empty
    // one would replace the fence with a blank line.
    if graph.nodes.is_empty() {
        return None;
    }
    let laid = Layered::of(&graph);
    if let Some(rows) = draw_boxes(&graph, &laid, direction, width, theme) {
        return Some(rows);
    }
    super::outline::render(&graph, width, theme)
}

/// The drawn diagram, or `None` for "not at this width" — which is a fallback
/// to the outline and never a refusal. Every way out of here is measured: the
/// pane's columns, [`ROW_CAP`], and whether every edge label found somewhere to
/// sit that was not on top of another edge.
fn draw_boxes(
    graph: &Graph,
    laid: &Layered,
    direction: Direction,
    width: usize,
    theme: &'static Theme,
) -> Option<Rows> {
    // A back-edge row wraps like prose, so a word in one has to fit the pane
    // the same way a word in a box does.
    let room = width.saturating_sub(2);
    let broken = graph.edges.iter().zip(&laid.back).any(|(edge, &back)| {
        back && edge.label.as_deref().is_some_and(|l| longest_word(l) > room)
    });
    if broken {
        return None;
    }
    let plan = Plan::of(graph, laid, direction, width)?;
    let mut rows = plan.draw(theme)?;
    let back = back_rows(graph, laid, width, theme);
    if !back.is_empty() {
        rows.push(Vec::new());
        rows.extend(back);
    }
    (rows.len() <= ROW_CAP).then_some(rows)
}

// --- the graph -----------------------------------------------------------

/// A node's frame, after twelve mermaid shapes have been mapped onto the three
/// a terminal can tell apart. See the module note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Frame {
    Process,
    Round,
    Decision,
}

/// How an edge is drawn along its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stroke {
    Solid,
    Dotted,
    Thick,
}

/// What an edge ends in, at either end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tip {
    None,
    Arrow,
    Circle,
    Cross,
}

#[derive(Clone, Debug)]
pub(super) struct Node {
    /// What the box says. The id when the node was never given a shape, which
    /// is what `A --> B` means by `A`.
    pub label: String,
    pub frame: Frame,
}

#[derive(Clone, Debug)]
pub(super) struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub stroke: Stroke,
    /// The tip drawn at `to`.
    pub head: Tip,
    /// The tip drawn at `from` — only ever set by the `<-->` family.
    pub tail: Tip,
}

pub(super) struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub(super) fn parse(body: &[String]) -> Option<Self> {
        let mut graph = Graph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let mut index: HashMap<String, usize> = HashMap::new();
        for line in body {
            for stmt in lex::split_statements(line) {
                graph.statement(&stmt, &mut index)?;
            }
        }
        // Checked after parsing rather than while: the caps are about the size
        // of the layout problem, and a statement is not one node.
        (graph.nodes.len() <= super::MAX_NODES && graph.edges.len() <= super::MAX_EDGES)
            .then_some(graph)
    }

    fn statement(&mut self, stmt: &str, index: &mut HashMap<String, usize>) -> Option<()> {
        match stmt.split_whitespace().next().unwrap_or("") {
            // Styling. Skipped rather than declined: none of it is text the
            // reader can lose, and a diagram is not worth refusing over a
            // colour this pane would have repainted anyway.
            "style" | "classDef" | "class" | "linkStyle" => return Some(()),
            // Content this cannot draw. `subgraph` has a label and a grouping,
            // `click` a URL and a tooltip; flattening either would be the
            // fourth outcome the module note rules out.
            "subgraph" | "end" | "click" => return None,
            _ => {}
        }

        let Split { groups, links } = split_links(stmt);
        let mut refs: Vec<Vec<usize>> = Vec::with_capacity(groups.len());
        for group in &groups {
            let mut ids = Vec::new();
            // `A & B --> C & D` is the cross product of the two groups, which
            // is what mermaid means by it.
            for part in split_ampersands(group) {
                let (id, label, frame, declared) = node_ref(&part)?;
                let at = *index.entry(id).or_insert_with(|| {
                    self.nodes.push(Node {
                        label: label.clone(),
                        frame,
                    });
                    self.nodes.len() - 1
                });
                // A shape written later names the node; a bare mention of it
                // later does not take the name back off. `A --> B` followed by
                // `B[Stop]` is one node called Stop, and `B --> C` after that
                // is still called Stop.
                if declared && let Some(node) = self.nodes.get_mut(at) {
                    node.label = label;
                    node.frame = frame;
                }
                ids.push(at);
            }
            if ids.is_empty() {
                return None;
            }
            refs.push(ids);
        }

        for (i, link) in links.iter().enumerate() {
            let (Some(lhs), Some(rhs)) = (refs.get(i), refs.get(i + 1)) else {
                return None;
            };
            for &from in lhs {
                for &to in rhs {
                    self.edges.push(Edge {
                        from,
                        to,
                        label: link.label.clone(),
                        stroke: link.stroke,
                        head: link.head,
                        tail: link.tail,
                    });
                }
            }
        }
        Some(())
    }
}

/// One statement cut into `n + 1` node groups and the `n` links between them.
struct Split {
    groups: Vec<String>,
    links: Vec<Link>,
}

#[derive(Clone, Debug)]
struct Link {
    stroke: Stroke,
    head: Tip,
    tail: Tip,
    label: Option<String>,
    /// A bare `--`, `-.` or `==`: the opening half of `A -- text --> B`, which
    /// is only knowable once we have seen whether another link follows.
    opener: bool,
}

fn split_links(stmt: &str) -> Split {
    let chars: Vec<char> = stmt.chars().collect();
    let mut groups = Vec::new();
    let mut links = Vec::new();
    let mut text = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if quoted {
            quoted = c != '"';
            text.push(c);
            i += 1;
            continue;
        }
        match c {
            '"' => {
                quoted = true;
                text.push(c);
                i += 1;
            }
            '[' | '(' | '{' => {
                depth += 1;
                text.push(c);
                i += 1;
            }
            ']' | ')' | '}' => {
                depth = depth.saturating_sub(1);
                text.push(c);
                i += 1;
            }
            // Only outside a label: `A[a --> b]` is one node whose text
            // happens to contain an arrow.
            _ if depth == 0 => match link_at(&chars, i) {
                Some((link, next)) => {
                    groups.push(std::mem::take(&mut text));
                    links.push(link);
                    i = next;
                }
                None => {
                    text.push(c);
                    i += 1;
                }
            },
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
    groups.push(text);

    merge_openers(Split { groups, links })
}

/// `A -- text --> B` arrives as two links with the text between them. Fold it
/// into one labelled link, which is the same edge `A -->|text| B` produces.
fn merge_openers(split: Split) -> Split {
    let Split { groups, links } = split;
    let mut out_groups = Vec::with_capacity(groups.len());
    let mut out_links = Vec::with_capacity(links.len());
    let mut groups = groups.into_iter();
    let mut links = links.into_iter().peekable();

    out_groups.push(groups.next().unwrap_or_default());
    while let Some(mut link) = links.next() {
        let text = groups.next().unwrap_or_default();
        if link.opener && let Some(mut closer) = links.next() {
            let label = lex::label(&text);
            closer.label = closer.label.or((!label.is_empty()).then_some(label));
            // The opener carries the stroke — `-. text .-> B` is dotted from
            // its first two characters, and its closer says so too, but
            // `A == text --> B` is the sort of thing half-typed mermaid does.
            closer.stroke = link.stroke;
            out_links.push(closer);
            out_groups.push(groups.next().unwrap_or_default());
            continue;
        }
        // A bare `A -- B` never became a label form. Mermaid would refuse it;
        // reading it as the undirected link it looks like keeps a half-typed
        // diagram on screen, which is the state the watcher shows us most.
        link.opener = false;
        out_links.push(link);
        out_groups.push(text);
    }
    Split {
        groups: out_groups,
        links: out_links,
    }
}

/// A link token starting at `at`, and the index just past it (and past any
/// `|label|` that followed it).
fn link_at(chars: &[char], at: usize) -> Option<(Link, usize)> {
    let mut i = at;
    let mut tail = Tip::None;
    match chars.get(i) {
        // `<-->`. No boundary test: `<` is not a character an id may hold, so
        // `A<-->B` is unambiguous.
        Some('<') if is_line(chars.get(i + 1)) => {
            tail = Tip::Arrow;
            i += 1;
        }
        // `o--o` and `x--x`, which need one: `box--o` would otherwise lose its
        // last letter to a tip.
        Some('o') if is_line(chars.get(i + 1)) && starts_token(chars, i) => {
            tail = Tip::Circle;
            i += 1;
        }
        Some('x') if is_line(chars.get(i + 1)) && starts_token(chars, i) => {
            tail = Tip::Cross;
            i += 1;
        }
        _ => {}
    }

    let body = i;
    let (mut dots, mut equals) = (0usize, 0usize);
    while let Some(&c) = chars.get(i) {
        match c {
            '-' => {}
            '.' => dots += 1,
            '=' => equals += 1,
            _ => break,
        }
        i += 1;
    }
    let length = i - body;
    if length < 2 {
        return None;
    }

    let head = match chars.get(i) {
        Some('>') => {
            i += 1;
            Tip::Arrow
        }
        // Same test as the tail, the other way round: `--- open` is a link and
        // a node, `--o` is a link with a circle on it.
        Some('o') if ends_token(chars, i + 1) => {
            i += 1;
            Tip::Circle
        }
        Some('x') if ends_token(chars, i + 1) => {
            i += 1;
            Tip::Cross
        }
        _ => Tip::None,
    };

    let stroke = if equals > 0 {
        Stroke::Thick
    } else if dots > 0 {
        Stroke::Dotted
    } else {
        Stroke::Solid
    };
    // Exactly two line characters and no tip at either end is the opening half
    // of `A -- text --> B`; three is `A --- B`, a link in its own right.
    let opener = length == 2 && head == Tip::None && tail == Tip::None;

    // `A -->|yes| B`.
    let mut label = None;
    let mut j = i;
    while matches!(chars.get(j), Some(c) if c.is_whitespace()) {
        j += 1;
    }
    if chars.get(j) == Some(&'|')
        && let Some(end) = chars.iter().skip(j + 1).position(|c| *c == '|')
    {
        let text: String = chars[j + 1..j + 1 + end].iter().collect();
        let text = lex::label(&text);
        label = (!text.is_empty()).then_some(text);
        i = j + end + 2;
    }

    Some((
        Link {
            stroke,
            head,
            tail,
            label,
            opener,
        },
        i,
    ))
}

fn is_line(c: Option<&char>) -> bool {
    matches!(c, Some('-' | '='))
}

fn starts_token(chars: &[char], at: usize) -> bool {
    at.checked_sub(1)
        .and_then(|i| chars.get(i))
        .is_none_or(|c| c.is_whitespace())
}

fn ends_token(chars: &[char], at: usize) -> bool {
    chars
        .get(at)
        .is_none_or(|c| !c.is_alphanumeric() && *c != '_')
}

/// Split a node group on `&`, ignoring one inside a label — `A[a &amp; b]` is
/// one node, and `&` is a character labels really do contain.
fn split_ampersands(group: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    for c in group.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            _ if quoted => cur.push(c),
            '[' | '(' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ']' | ')' | '}' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            '&' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.retain(|part| !part.trim().is_empty());
    out
}

/// Every wrapper mermaid has, longest opener first so `((circle))` is not read
/// as a round node whose label starts with a bracket.
const WRAPPERS: &[(&str, &str, Frame)] = &[
    ("([", "])", Frame::Round),     // stadium
    ("[[", "]]", Frame::Process),   // subroutine
    ("[(", ")]", Frame::Round),     // cylinder
    ("((", "))", Frame::Round),     // circle
    ("[/", "/]", Frame::Process),   // parallelogram
    ("[/", "\\]", Frame::Process),  // trapezoid
    ("[\\", "\\]", Frame::Process), // parallelogram, the other lean
    ("[\\", "/]", Frame::Process),  // trapezoid, the other lean
    ("{{", "}}", Frame::Decision),  // hexagon
    ("[", "]", Frame::Process),     // rectangle
    ("(", ")", Frame::Round),       // round
    ("{", "}", Frame::Decision),    // rhombus
    (">", "]", Frame::Process),     // flag
];

/// `(id, label, frame, whether a shape was written)`, or `None` for anything
/// this cannot read — an unclosed bracket, a shape mermaid does not have, an id
/// with punctuation in it. All of which decline the diagram.
fn node_ref(part: &str) -> Option<(String, String, Frame, bool)> {
    let part = part.trim();
    // `A:::highlight` attaches a class. The class is styling; the node is not.
    let part = part.split(":::").next().unwrap_or(part).trim();
    if part.is_empty() {
        return None;
    }

    let open = part.find(['[', '(', '{', '>']);
    let Some(open) = open else {
        let id = part.to_string();
        return is_id(&id).then(|| (id.clone(), id, Frame::Process, false));
    };

    let (id, rest) = part.split_at(open);
    let id = id.trim();
    if !is_id(id) {
        return None;
    }
    for (prefix, suffix, frame) in WRAPPERS {
        if rest.len() >= prefix.len() + suffix.len()
            && rest.starts_with(prefix)
            && rest.ends_with(suffix)
        {
            let text = &rest[prefix.len()..rest.len() - suffix.len()];
            let label = lex::label(text);
            let label = if label.is_empty() {
                id.to_string()
            } else {
                label
            };
            return Some((id.to_string(), label, *frame, true));
        }
    }
    None
}

/// What may be a node id. Deliberately permissive about alphabet — `日本` is a
/// perfectly good id and mermaid accepts it — and strict about punctuation,
/// because that is what makes `accTitle: x` decline instead of becoming a node
/// nobody wrote.
fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

// --- ranking -------------------------------------------------------------

/// One place in a rank: either a node's box, or a single column of an edge
/// passing through on its way to a rank further on. See the module note on why
/// the pass-through gets a slot of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Node(usize),
    /// An edge passing through on its way further down, holding the index of
    /// the edge so the pass-through is drawn in that edge's own stroke.
    Dummy(usize),
}

/// One edge's crossing of one band, from a slot in `rank` to a slot in
/// `rank + 1`. A short edge is one of these; a long one is a chain.
struct Seg {
    edge: usize,
    rank: usize,
    from: usize,
    to: usize,
    /// Leaves a real node, so it draws the tail marker and the border tick.
    first: bool,
    /// Arrives at a real node, so it draws the arrowhead.
    last: bool,
}

/// The graph with the cycles cut out of it and everything in a rank.
struct Layered {
    /// Per edge: cut by the cycle break, and drawn under the diagram instead.
    back: Vec<bool>,
    slots: Vec<Vec<Slot>>,
    /// `pos[rank][slot]` — where in the rank that slot is drawn, which is the
    /// only thing the ordering pass produces.
    pos: Vec<Vec<usize>>,
    segs: Vec<Seg>,
}

impl Layered {
    fn of(graph: &Graph) -> Self {
        let mut back = back_edges(graph);
        let rank = ranks(graph, &back);
        // A forward edge that did not end up going forward would vanish when
        // the segments below are built, and a vanished edge is the outcome the
        // module note forbids. Demoting it to a back-edge draws it under the
        // diagram instead, which is where an edge nobody can rank belongs.
        for (e, edge) in graph.edges.iter().enumerate() {
            if !back[e] && rank[edge.to] <= rank[edge.from] {
                back[e] = true;
            }
        }

        let depth = rank.iter().copied().max().unwrap_or(0) + 1;
        let mut slots = vec![Vec::new(); depth];
        let mut node_at = vec![0usize; graph.nodes.len()];
        for (node, &r) in rank.iter().enumerate() {
            node_at[node] = slots[r].len();
            slots[r].push(Slot::Node(node));
        }

        let mut segs = Vec::new();
        for (e, edge) in graph.edges.iter().enumerate() {
            if back[e] {
                continue;
            }
            let (top, bottom) = (rank[edge.from], rank[edge.to]);
            let mut from = node_at[edge.from];
            for r in top..bottom {
                let to = if r + 1 == bottom {
                    node_at[edge.to]
                } else {
                    slots[r + 1].push(Slot::Dummy(e));
                    slots[r + 1].len() - 1
                };
                segs.push(Seg {
                    edge: e,
                    rank: r,
                    from,
                    to,
                    first: r == top,
                    last: r + 1 == bottom,
                });
                from = to;
            }
        }

        let pos = order(&slots, &segs);
        Layered {
            back,
            slots,
            pos,
            segs,
        }
    }
}

/// Which edges close a cycle, by depth-first search in declaration order.
///
/// Deterministic by construction: the search starts from each node in the order
/// the document declared it and follows edges in the order it wrote them, so the
/// same file always loses the same edge. Which edge gets cut is arbitrary — any
/// choice is — but it must not be arbitrary *twice* for the same input, or the
/// pane would redraw a diagram differently on a keystroke.
fn back_edges(graph: &Graph) -> Vec<bool> {
    let out = adjacency(graph);
    let mut back = vec![false; graph.edges.len()];
    // 0 unseen, 1 on the current path, 2 finished. An edge to a node on the
    // current path is the definition of a back-edge; a self-loop is the case
    // where that node is the one we are standing on.
    let mut state = vec![0u8; graph.nodes.len()];
    // Iterative rather than recursive: a hand-written diagram will not nest
    // deeply, but a generated one is exactly the input that would find the
    // stack limit, and there is no bound on shape below the node cap.
    let mut stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..graph.nodes.len() {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        stack.push((start, 0));
        while let Some((node, next)) = stack.last_mut() {
            let (node, at) = (*node, *next);
            match out[node].get(at) {
                Some(&e) => {
                    *next += 1;
                    let to = graph.edges[e].to;
                    match state[to] {
                        0 => {
                            state[to] = 1;
                            stack.push((to, 0));
                        }
                        1 => back[e] = true,
                        _ => {}
                    }
                }
                None => {
                    state[node] = 2;
                    stack.pop();
                }
            }
        }
    }
    back
}

/// Longest path from a source, over the edges the cycle break left behind.
///
/// Longest rather than shortest so that every edge spans at least one band:
/// with shortest paths a diamond's long side would arrive in the same rank it
/// left, and an edge inside a rank has nowhere to be drawn.
fn ranks(graph: &Graph, back: &[bool]) -> Vec<usize> {
    let out = adjacency(graph);
    let mut rank = vec![0usize; graph.nodes.len()];
    let mut waiting = vec![0usize; graph.nodes.len()];
    for (e, edge) in graph.edges.iter().enumerate() {
        if !back[e] {
            waiting[edge.to] += 1;
        }
    }
    let mut queue: VecDeque<usize> = (0..graph.nodes.len())
        .filter(|&n| waiting[n] == 0)
        .collect();
    while let Some(node) = queue.pop_front() {
        for &e in &out[node] {
            if back[e] {
                continue;
            }
            let to = graph.edges[e].to;
            rank[to] = rank[to].max(rank[node] + 1);
            waiting[to] -= 1;
            if waiting[to] == 0 {
                queue.push_back(to);
            }
        }
    }
    rank
}

fn adjacency(graph: &Graph) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); graph.nodes.len()];
    for (e, edge) in graph.edges.iter().enumerate() {
        out[edge.from].push(e);
    }
    out
}

/// Where each slot sits in its rank.
///
/// One barycentre pass, downward only. A full Sugiyama would sweep up and down
/// until the crossings stopped improving; this is a pane redrawing on a
/// keystroke, and one pass already puts a child under its parent, which is the
/// difference a reader actually sees. Ties break on declaration order, so the
/// layout is a function of the file and nothing else.
fn order(slots: &[Vec<Slot>], segs: &[Seg]) -> Vec<Vec<usize>> {
    let mut pos: Vec<Vec<usize>> = slots.iter().map(|rank| (0..rank.len()).collect()).collect();
    for r in 1..slots.len() {
        let mut sum = vec![0u64; slots[r].len()];
        let mut count = vec![0u64; slots[r].len()];
        for seg in segs.iter().filter(|s| s.rank + 1 == r) {
            sum[seg.to] += pos[r - 1][seg.from] as u64;
            count[seg.to] += 1;
        }
        // Scaled integer division rather than a float: the sort has to be a
        // total order with no rounding surprises, and `f64` comparison in a
        // sort key is a `partial_cmp` and an `unwrap` waiting to happen.
        // A slot with no predecessor sorts last rather than first: it is a
        // node the ranking could not tie to anything above it, and the
        // barycentres are what the ranks either side were ordered by.
        let key = |i: usize| (sum[i] * 64).checked_div(count[i]).unwrap_or(u64::MAX);
        let mut idx: Vec<usize> = (0..slots[r].len()).collect();
        idx.sort_by_key(|&i| (key(i), i));
        for (place, &i) in idx.iter().enumerate() {
            pos[r][i] = place;
        }
    }
    pos
}

// --- the character grid --------------------------------------------------

/// The second cell of a two-cell character. Emits nothing; see [`Canvas::write`].
const TAKEN: char = '\u{0}';

/// The four ways a connector can leave a cell.
///
/// Junctions are resolved from these rather than from whichever line was drawn
/// last, which is what the first attempt did and why a fan out of one port drew
/// its own corners over: the second edge's riser ran straight through the first
/// edge's turn and left `│` where `┤` belonged.
const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// What a cell is, which is the only thing its colour depends on. Held per
/// cell rather than per span so that the run-coalescing at the end is the one
/// place a style is chosen, and a crossing cannot end up half one colour.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ink {
    Blank,
    Frame,
    Text,
    Line,
    Label,
}

struct Canvas {
    rows: usize,
    cols: usize,
    cell: Vec<char>,
    ink: Vec<Ink>,
}

impl Canvas {
    fn new(rows: usize, cols: usize) -> Self {
        Canvas {
            rows,
            cols,
            cell: vec![' '; rows * cols],
            ink: vec![Ink::Blank; rows * cols],
        }
    }

    fn put(&mut self, row: usize, col: usize, ch: char, ink: Ink) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let at = row * self.cols + col;
        self.cell[at] = ch;
        self.ink[at] = ink;
    }

    fn free(&self, row: usize, col: usize, line: char) -> bool {
        if row >= self.rows || col >= self.cols {
            return false;
        }
        let at = row * self.cols + col;
        // A label may be written over its own edge's horizontal run — that is
        // what `╰─ yes ─╮` is — but never over another edge's, and never over
        // a crossing, which is somebody else's line passing through.
        match self.ink[at] {
            Ink::Blank => true,
            Ink::Line => self.cell[at] == line,
            _ => false,
        }
    }

    /// Text, one *cell* at a time rather than one character.
    ///
    /// An ideograph is two cells wide and the grid is measured in cells, so the
    /// second cell is claimed by a marker that emits nothing. Without it a
    /// four-ideograph label would be laid into four cells, drawn into eight,
    /// and every box to its right would be in the wrong column — which is the
    /// bug `str::len` is wrong about twice over.
    fn write(&mut self, row: usize, col: usize, text: &str, ink: Ink) {
        let mut at = col;
        for ch in text.chars() {
            let wide = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            if at + wide > self.cols {
                return;
            }
            self.put(row, at, ch, ink);
            for skip in 1..wide {
                self.put(row, at + skip, TAKEN, ink);
            }
            at += wide;
        }
    }

    /// Rows of spans, runs of one ink at a time, trailing blanks dropped.
    fn into_rows(self, theme: &'static Theme) -> Rows {
        let mut out = Rows::new();
        for row in 0..self.rows {
            let base = row * self.cols;
            let end = (0..self.cols)
                .rev()
                .find(|&c| self.cell[base + c] != ' ')
                .map_or(0, |c| c + 1);
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run = String::new();
            let mut ink = Ink::Blank;
            for col in 0..end {
                if self.cell[base + col] == TAKEN {
                    continue;
                }
                let here = self.ink[base + col];
                if here != ink && !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), style(ink, theme)));
                }
                ink = here;
                run.push(self.cell[base + col]);
            }
            if !run.is_empty() {
                spans.push(Span::styled(run, style(ink, theme)));
            }
            out.push(spans);
        }
        out
    }
}

fn style(ink: Ink, theme: &'static Theme) -> ratatui::style::Style {
    use ratatui::style::Style;
    match ink {
        // No foreground at all: a blank cell inherits the page, and naming a
        // colour for a space is one more thing to keep in step with the theme.
        Ink::Blank => Style::default(),
        // The frame and the connectors are chrome — the pane's own voice, the
        // same argument `markdown` makes for a code gutter. What the reader is
        // there to read is the text inside.
        Ink::Frame | Ink::Line => Style::default().fg(theme.dim),
        Ink::Text => Style::default().fg(theme.fg),
        // An edge label is the one piece of text that would otherwise be lost
        // in the chrome around it.
        Ink::Label => Style::default().fg(theme.accent),
    }
}

/// Which way a line leaves a cell. Named rather than signed because the two
/// axes swap meaning between `TD` and `LR`, and a `-1` that means "up" in one
/// layout and "left" in the other is how the corners got drawn backwards the
/// first time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Up,
    Down,
    Left,
    Right,
}

impl Side {
    fn opposite(self) -> Self {
        match self {
            Side::Up => Side::Down,
            Side::Down => Side::Up,
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    fn arrow(self) -> char {
        match self {
            Side::Up => '▲',
            Side::Down => '▼',
            Side::Left => '◀',
            Side::Right => '▶',
        }
    }

    fn bit(self) -> u8 {
        match self {
            Side::Up => UP,
            Side::Down => DOWN,
            Side::Left => LEFT,
            Side::Right => RIGHT,
        }
    }
}

/// The glyphs one stroke draws with.
struct Pen {
    across: char,
    down: char,
}

impl Pen {
    fn of(stroke: Stroke) -> Self {
        match stroke {
            Stroke::Solid => Pen {
                across: '─',
                down: '│',
            },
            Stroke::Dotted => Pen {
                across: '┄',
                down: '┊',
            },
            Stroke::Thick => Pen {
                across: '━',
                down: '┃',
            },
        }
    }

    /// The glyph for a cell that lines leave by the given sides.
    ///
    /// Dotted borrows the light corners and junctions — Unicode has no dotted
    /// `╰` and the dashes along the run are what carry the style anyway. So
    /// does a junction where two strokes meet: a crossing belongs to neither
    /// edge, and drawing it in one of the two would say it did.
    fn joint(&self, stroke: Stroke, ways: u8) -> char {
        let (up, down) = (ways & UP != 0, ways & DOWN != 0);
        let (left, right) = (ways & LEFT != 0, ways & RIGHT != 0);
        let (light, heavy) = match (up, down, left, right) {
            // Straight through, including the one-way case a dead end leaves.
            (_, _, false, false) => return self.down,
            (false, false, _, _) => return self.across,
            (true, true, true, true) => ('┼', '╋'),
            (true, true, true, false) => ('┤', '┫'),
            (true, true, false, true) => ('├', '┣'),
            (true, false, true, true) => ('┴', '┻'),
            (false, true, true, true) => ('┬', '┳'),
            (true, false, true, false) => ('╯', '┛'),
            (true, false, false, true) => ('╰', '┗'),
            (false, true, true, false) => ('╮', '┓'),
            (false, true, false, true) => ('╭', '┏'),
        };
        if stroke == Stroke::Thick { heavy } else { light }
    }

    /// What an edge ends in, drawn at the target's port.
    ///
    /// A two-way edge is marked here rather than by a second arrowhead above
    /// its source, and that is not a shortcut: every edge leaving one node
    /// shares one riser, so a `▲` in that column would claim the whole fan
    /// pointed both ways. Mermaid's three two-way spellings are all
    /// *symmetric* — `<-->`, `o--o`, `x--x`, never a mixed pair — so one
    /// symmetric glyph at the far end says the whole of what they mean.
    fn tip(&self, tip: Tip, two_way: bool, facing: Side) -> char {
        match tip {
            Tip::None => match facing {
                Side::Up | Side::Down => self.down,
                Side::Left | Side::Right => self.across,
            },
            Tip::Arrow if two_way => match facing {
                Side::Up | Side::Down => '↕',
                Side::Left | Side::Right => '↔',
            },
            Tip::Arrow => facing.arrow(),
            // `--o` and `--x` mean something in mermaid — a link that ends in
            // a state rather than a step — so they keep a mark of their own
            // rather than degrading to a plain arrow.
            Tip::Circle => '●',
            Tip::Cross => '╳',
        }
    }
}

/// The frame a node is drawn in. Three of them; see the module note.
struct Border {
    tl: char,
    tr: char,
    bl: char,
    br: char,
    across: char,
    down: char,
}

impl Border {
    fn of(frame: Frame) -> Self {
        match frame {
            Frame::Process => Border {
                tl: '┌',
                tr: '┐',
                bl: '└',
                br: '┘',
                across: '─',
                down: '│',
            },
            Frame::Round => Border {
                tl: '╭',
                tr: '╮',
                bl: '╰',
                br: '╯',
                across: '─',
                down: '│',
            },
            Frame::Decision => Border {
                tl: '╔',
                tr: '╗',
                bl: '╚',
                br: '╝',
                across: '═',
                down: '║',
            },
        }
    }

    /// The border cell an edge leaves through, so a box shows where its own
    /// connectors start rather than having them appear a row below it.
    fn tick(&self, side: Side) -> char {
        let double = self.across == '═';
        match (side, double) {
            (Side::Up, false) => '┴',
            (Side::Up, true) => '╧',
            (Side::Down, false) => '┬',
            (Side::Down, true) => '╤',
            (Side::Left, false) => '┤',
            (Side::Left, true) => '╢',
            (Side::Right, false) => '├',
            (Side::Right, true) => '╟',
        }
    }
}

// --- geometry ------------------------------------------------------------

/// A run of cells on one axis: where it starts and how long it is.
#[derive(Clone, Copy)]
struct Extent {
    at: usize,
    len: usize,
}

impl Extent {
    fn last(&self) -> usize {
        self.at + self.len.saturating_sub(1)
    }
}

/// The gap between two ranks, and where each edge crossing it turns.
struct BandPlan {
    at: usize,
    len: usize,
    /// Cells reserved before the first channel. Across the page only, and for
    /// one thing: room to write a label along the leg an edge leaves by, since
    /// text cannot be drawn down a column.
    lead: usize,
    /// Indices into `Layered::segs`, in declaration order.
    segs: Vec<usize>,
    /// The channel each of those turns in, if it has to move sideways at all.
    channel: Vec<Option<usize>>,
}

/// Everything measured, before a single cell is drawn.
///
/// Measuring first is the whole shape of this module: the pane hands down a
/// column count and the answer has to be "boxes" or "not boxes" *before*
/// anything is emitted. A drawer that found out halfway down that a rank would
/// not fit would have to either overflow the pane or abandon rows it had
/// already emitted, and the second is how a diagram loses an edge.
struct Plan<'a> {
    graph: &'a Graph,
    laid: &'a Layered,
    vertical: bool,
    forward: bool,
    text: Vec<Vec<String>>,
    rank: Vec<Extent>,
    slot: Vec<Vec<Extent>>,
    port: Vec<Vec<usize>>,
    bands: Vec<BandPlan>,
    rows: usize,
    cols: usize,
}

impl<'a> Plan<'a> {
    fn of(graph: &'a Graph, laid: &'a Layered, direction: Direction, width: usize) -> Option<Self> {
        let vertical = direction.is_vertical();
        let forward = !direction.is_reversed();
        // Two borders and a space either side of the text. Below `MIN_CONTENT`
        // what is left is a box holding a syllable, which is the puzzle the
        // module note says to hand to the outline instead.
        let limit = LABEL_MAX.min(width.saturating_sub(4));
        if limit < MIN_CONTENT {
            return None;
        }
        // A word wider than the box it goes in comes out in pieces, and half a
        // word is not the word — see `tests::assert_keeps`, which is the rule
        // this enforces. The outline gets the same graph with more room per
        // row, and declines in its turn if even that is not enough.
        if graph.nodes.iter().any(|n| longest_word(&n.label) > limit) {
            return None;
        }
        let text: Vec<Vec<String>> = graph
            .nodes
            .iter()
            .map(|node| wrap_label(&node.label, limit))
            .collect();
        let size: Vec<(usize, usize)> = text
            .iter()
            .map(|lines| {
                let widest = lines.iter().map(|l| l.width()).max().unwrap_or(0);
                (widest, lines.len())
            })
            .collect();

        // Every box in a rank is the same size along the rank's own axis, so
        // that the band between two ranks starts at one major coordinate
        // rather than at each box's own edge. A connector that had to reach
        // back for a shorter neighbour would cross the gap twice.
        let ranks: Vec<usize> = laid
            .slots
            .iter()
            .map(|slots| {
                let most = slots
                    .iter()
                    .filter_map(|slot| match slot {
                        Slot::Node(n) => Some(if vertical { size[*n].1 } else { size[*n].0 }),
                        Slot::Dummy(_) => None,
                    })
                    .max()
                    .unwrap_or(0);
                most + if vertical { 2 } else { 4 }
            })
            .collect();

        // Rounded up to an odd number of cells, which is not decoration: a
        // box's port is its middle cell, and with every box an odd number of
        // cells wide *every* rank holding one slot puts its port in the same
        // column. Without it a ten-cell box over a five-cell one drew a jog —
        // `╭╯` — between two boxes that are both, visibly, centred.
        let minor_len = |slot: Slot| match slot {
            Slot::Node(n) if vertical => (size[n].0 + 4) | 1,
            Slot::Node(n) => (size[n].1 + 2) | 1,
            Slot::Dummy(_) => 1,
        };
        let mut across = Vec::with_capacity(laid.slots.len());
        for (r, slots) in laid.slots.iter().enumerate() {
            let mut idx: Vec<usize> = (0..slots.len()).collect();
            idx.sort_by_key(|&i| laid.pos[r][i]);
            let sum: usize = idx.iter().map(|&i| minor_len(slots[i])).sum();
            across.push((idx, sum + GAP * slots.len().saturating_sub(1)));
        }
        let total_minor = across.iter().map(|(_, sum)| *sum).max().unwrap_or(0);

        let mut slot = Vec::with_capacity(laid.slots.len());
        let mut port = Vec::with_capacity(laid.slots.len());
        for (r, (idx, sum)) in across.iter().enumerate() {
            // Ranks are centred on each other rather than left-aligned: a root
            // fanning out is the commonest shape in a flowchart, and hanging
            // it off the left edge sends every one of its connectors the same
            // way for no reason.
            let mut at = (total_minor - sum) / 2;
            let mut extents = vec![Extent { at: 0, len: 0 }; laid.slots[r].len()];
            let mut ports = vec![0usize; laid.slots[r].len()];
            for &i in idx {
                let len = minor_len(laid.slots[r][i]);
                extents[i] = Extent { at, len };
                ports[i] = match laid.slots[r][i] {
                    Slot::Node(_) => at + len / 2,
                    Slot::Dummy(_) => at,
                };
                at += len + GAP;
            }
            slot.push(extents);
            port.push(ports);
        }

        let mut bands = Vec::new();
        for r in 0..laid.slots.len().saturating_sub(1) {
            bands.push(band_plan(graph, laid, &port, r, vertical));
        }

        let mut at = 0;
        let mut rank = Vec::with_capacity(ranks.len());
        for (r, len) in ranks.iter().enumerate() {
            rank.push(Extent { at, len: *len });
            at += len;
            if let Some(band) = bands.get_mut(r) {
                band.at = at;
                at += band.len;
            }
        }
        let total_major = at;
        // `BT` and `RL` are `TD` and `LR` with the ranks in the other order,
        // which is the whole of their implementation — see `Direction`. Only
        // the offsets are mirrored: the boxes themselves are drawn the same
        // way up, because a label read bottom-to-top is not a label.
        if !forward {
            for extent in &mut rank {
                extent.at = total_major - extent.at - extent.len;
            }
            for band in &mut bands {
                band.at = total_major - band.at - band.len;
            }
        }

        let (rows, across) = if vertical {
            (total_major, total_minor)
        } else {
            (total_minor, total_major)
        };
        if across > width || rows > ROW_CAP {
            return None;
        }
        Some(Plan {
            graph,
            laid,
            vertical,
            forward,
            text,
            rank,
            slot,
            port,
            bands,
            rows,
            cols: width,
        })
    }

    /// Screen coordinates. The major axis runs the way the ranks advance and
    /// the minor axis across them, which is rows and columns for `TD` and the
    /// other way round for `LR`.
    fn cell(&self, major: usize, minor: usize) -> (usize, usize) {
        if self.vertical {
            (major, minor)
        } else {
            (minor, major)
        }
    }

    fn draw(&self, theme: &'static Theme) -> Option<Rows> {
        let mut canvas = Canvas::new(self.rows, self.cols);
        self.draw_nodes(&mut canvas);
        for r in 0..self.bands.len() {
            self.draw_band(&mut canvas, r)?;
        }
        Some(canvas.into_rows(theme))
    }

    fn draw_nodes(&self, canvas: &mut Canvas) {
        for (r, slots) in self.laid.slots.iter().enumerate() {
            for (i, slot) in slots.iter().enumerate() {
                // A pass-through is the edge itself, drawn straight through a
                // rank it has no box in.
                if let Slot::Dummy(edge) = *slot {
                    let pen = Pen::of(self.graph.edges[edge].stroke);
                    let line = if self.vertical { pen.down } else { pen.across };
                    for step in 0..self.rank[r].len {
                        let (row, col) = self.cell(self.rank[r].at + step, self.slot[r][i].at);
                        canvas.put(row, col, line, Ink::Line);
                    }
                    continue;
                }
                let Slot::Node(node) = *slot else { continue };
                let (row, col) = self.cell(self.rank[r].at, self.slot[r][i].at);
                let (high, wide) = if self.vertical {
                    (self.rank[r].len, self.slot[r][i].len)
                } else {
                    (self.slot[r][i].len, self.rank[r].len)
                };
                let border = Border::of(self.graph.nodes[node].frame);
                for step in 0..wide {
                    canvas.put(row, col + step, border.across, Ink::Frame);
                    canvas.put(row + high - 1, col + step, border.across, Ink::Frame);
                }
                for step in 0..high {
                    canvas.put(row + step, col, border.down, Ink::Frame);
                    canvas.put(row + step, col + wide - 1, border.down, Ink::Frame);
                }
                canvas.put(row, col, border.tl, Ink::Frame);
                canvas.put(row, col + wide - 1, border.tr, Ink::Frame);
                canvas.put(row + high - 1, col, border.bl, Ink::Frame);
                canvas.put(row + high - 1, col + wide - 1, border.br, Ink::Frame);

                // Centred both ways inside the frame. A rank whose tallest box
                // has three lines gives every box in it three lines of room,
                // and a one-line label floating at the top of that would read
                // as a box with something missing under it.
                let lines = &self.text[node];
                let (inner_h, inner_w) = (high - 2, wide - 4);
                let top = row + 1 + (inner_h.saturating_sub(lines.len())) / 2;
                for (n, line) in lines.iter().enumerate() {
                    let left = col + 2 + inner_w.saturating_sub(line.width()) / 2;
                    canvas.write(top + n, left, line, Ink::Text);
                }
            }
        }
    }

    fn draw_band(&self, canvas: &mut Canvas, r: usize) -> Option<()> {
        let band = &self.bands[r];
        let major = |i: usize| {
            if self.forward {
                band.at + i
            } else {
                band.at + band.len - 1 - i
            }
        };
        let back = match (self.vertical, self.forward) {
            (true, true) => Side::Up,
            (true, false) => Side::Down,
            (false, true) => Side::Left,
            (false, false) => Side::Right,
        };
        let ahead = back.opposite();
        let entry = band.len - 1;

        // Every cell a connector passes through, and which ways it leaves by.
        // Collected for the whole band before a glyph is chosen for any of it:
        // a junction is a property of the cell, not of whichever edge reached
        // it last.
        let mut joints: HashMap<(usize, usize), (u8, Stroke, bool)> = HashMap::new();
        let mut tips: Vec<(usize, usize, char)> = Vec::new();
        let along = if self.vertical { UP | DOWN } else { LEFT | RIGHT };
        let sideways = if self.vertical { LEFT | RIGHT } else { UP | DOWN };

        for (k, &s) in band.segs.iter().enumerate() {
            let seg = &self.laid.segs[s];
            let edge = &self.graph.edges[seg.edge];
            let pen = Pen::of(edge.stroke);
            let (from, to) = (self.port[r][seg.from], self.port[r + 1][seg.to]);
            // Only the last segment of a long edge carries its tip; the ones
            // before it are the line passing through a rank.
            let head = if seg.last { edge.head } else { Tip::None };

            if seg.first && let Slot::Node(node) = self.laid.slots[r][seg.from] {
                let border = if self.forward {
                    self.rank[r].last()
                } else {
                    self.rank[r].at
                };
                let (row, col) = self.cell(border, from);
                let tick = Border::of(self.graph.nodes[node].frame).tick(ahead);
                canvas.put(row, col, tick, Ink::Frame);
            }

            match band.channel[k] {
                Some(channel) => {
                    let turn = band.lead + channel;
                    for i in 0..turn {
                        join(&mut joints, self.cell(major(i), from), along, edge.stroke);
                    }
                    if from == to {
                        join(&mut joints, self.cell(major(turn), from), along, edge.stroke);
                    } else {
                        let toward = match (to > from, self.vertical) {
                            (true, true) => Side::Right,
                            (true, false) => Side::Down,
                            (false, true) => Side::Left,
                            (false, false) => Side::Up,
                        };
                        let corner = back.bit() | toward.bit();
                        join(&mut joints, self.cell(major(turn), from), corner, edge.stroke);
                        for m in from.min(to) + 1..from.max(to) {
                            join(&mut joints, self.cell(major(turn), m), sideways, edge.stroke);
                        }
                        let corner = toward.opposite().bit() | ahead.bit();
                        join(&mut joints, self.cell(major(turn), to), corner, edge.stroke);
                    }
                    for i in turn + 1..entry {
                        join(&mut joints, self.cell(major(i), to), along, edge.stroke);
                    }
                }
                None => {
                    for i in 0..entry {
                        join(&mut joints, self.cell(major(i), from), along, edge.stroke);
                    }
                }
            }

            let (row, col) = self.cell(major(entry), to);
            let two_way = edge.tail != Tip::None;
            tips.push((row, col, pen.tip(head, two_way, ahead)));
        }

        for (&(row, col), &(ways, stroke, mixed)) in &joints {
            let stroke = if mixed { Stroke::Solid } else { stroke };
            canvas.put(row, col, Pen::of(stroke).joint(stroke, ways), Ink::Line);
        }
        for (row, col, glyph) in tips {
            canvas.put(row, col, glyph, Ink::Line);
        }

        // Labels last, and only once every connector in the band is down. A
        // label may be written over its own edge's run — that is what
        // `╰─ yes ─╮` is — but a label written over somebody else's would take
        // an edge off the drawing, so placement fails and the outline answers
        // instead.
        for (k, &s) in band.segs.iter().enumerate() {
            let seg = &self.laid.segs[s];
            let edge = &self.graph.edges[seg.edge];
            if !seg.first {
                continue;
            }
            let Some(text) = edge.label.as_deref() else {
                continue;
            };
            let pen = Pen::of(edge.stroke);
            let (from, to) = (self.port[r][seg.from], self.port[r + 1][seg.to]);
            let spots = self.label_spots(band, k, &major, from, to, text.width());
            let placed = spots.into_iter().find(|&(row, col)| {
                (0..text.width()).all(|n| canvas.free(row, col + n, pen.across))
            });
            let (row, col) = placed?;
            canvas.write(row, col, text, Ink::Label);
        }
        Some(())
    }

    /// Where an edge label might go, best first.
    ///
    /// Down the page the answer is nearly always "on the horizontal run", which
    /// is the one part of a connector that is already going the way text does.
    /// Across the page there is no such run — the sideways move is a column —
    /// so the label goes on the leg instead, and the band was widened by
    /// [`band_plan`] to make room for it.
    fn label_spots(
        &self,
        band: &BandPlan,
        k: usize,
        major: &dyn Fn(usize) -> usize,
        from: usize,
        to: usize,
        wide: usize,
    ) -> Vec<(usize, usize)> {
        let mut spots = Vec::new();
        let entry = band.len - 1;
        if self.vertical {
            let Some(channel) = band.channel[k] else {
                return spots;
            };
            let row = major(band.lead + channel);
            if from != to {
                let (lo, hi) = (from.min(to) + 1, from.max(to));
                if hi - lo >= wide {
                    spots.push((row, lo + (hi - lo - wide) / 2));
                }
            }
            spots.push((row, from.max(to) + 2));
            if let Some(col) = from.min(to).checked_sub(wide + 1) {
                spots.push((row, col));
            }
            return spots;
        }

        let turn = band.channel[k].map(|channel| band.lead + channel);
        let mut leg = |first: usize, last: usize, row: usize| {
            if first >= last {
                return;
            }
            let (lo, hi) = (major(first).min(major(last - 1)), major(first).max(major(last - 1)));
            if hi - lo < wide {
                return;
            }
            // One cell of line is kept between the box and the text, at
            // whichever end of the leg the box is on.
            spots.push((row, if self.forward { lo + 1 } else { hi - wide }));
        };
        leg(0, turn.unwrap_or(entry), from);
        if let Some(turn) = turn {
            leg(turn + 1, entry, to);
        }
        spots
    }
}

/// Record that a connector passes through a cell, leaving by `ways`.
fn join(
    joints: &mut HashMap<(usize, usize), (u8, Stroke, bool)>,
    at: (usize, usize),
    ways: u8,
    stroke: Stroke,
) {
    let cell = joints.entry(at).or_insert((0, stroke, false));
    cell.0 |= ways;
    cell.2 |= cell.1 != stroke;
}

/// The band between rank `r` and rank `r + 1`: how many channels it needs, and
/// how much room its labels want.
fn band_plan(
    graph: &Graph,
    laid: &Layered,
    port: &[Vec<usize>],
    r: usize,
    vertical: bool,
) -> BandPlan {
    let segs: Vec<usize> = laid
        .segs
        .iter()
        .enumerate()
        .filter(|(_, seg)| seg.rank == r)
        .map(|(i, _)| i)
        .collect();

    let mut channel = vec![None; segs.len()];
    let mut channels = 0;
    let mut widest = 0;
    for (k, &s) in segs.iter().enumerate() {
        let seg = &laid.segs[s];
        let edge = &graph.edges[seg.edge];
        let labelled = seg.first && edge.label.is_some();
        if let Some(label) = edge.label.as_deref().filter(|_| seg.first) {
            widest = widest.max(label.width());
        }
        let (from, to) = (port[r][seg.from], port[r + 1][seg.to]);
        // A channel is a row (or column) of its own, so an edge only earns one
        // by having somewhere to be: a sideways move, or — down the page — a
        // label, which has to sit beside the riser and needs a row to sit in.
        if from != to || (vertical && labelled) {
            channel[k] = Some(channels);
            channels += 1;
        }
    }

    // Down the page a label sits beside the riser on its own channel and the
    // band needs nothing extra; across the page it has to be written along a
    // leg, so both legs are given room for the widest of them.
    let room = if widest > 0 { widest + 2 } else { 0 };
    let lead = if vertical { 0 } else { room };
    // Across the page a label goes on a leg, and an edge has two: the one out
    // of its source and the one into its target. Both are reserved, because a
    // fan whose labels all wanted the same leg would otherwise have nowhere to
    // put the second one.
    let trail = if vertical { 0 } else { room };
    // Across the page a band of one column puts an arrowhead between two
    // borders with nothing either side of it, and `│ B │◀┤ A │` reads as one
    // box rather than two. Down the page one row is enough — the boxes above
    // and below it are already a row apart — unless a stroke in the band is
    // dotted or thick, which is a distinction with nowhere to be drawn when
    // the only cell an edge gets is its arrowhead.
    let plain = segs
        .iter()
        .all(|&s| graph.edges[laid.segs[s].edge].stroke == Stroke::Solid);
    let least = if vertical && plain { 1 } else { 2 };
    BandPlan {
        at: 0,
        len: (lead + channels + 1 + trail).max(least),
        lead,
        segs,
        channel,
    }
}

/// The widest single word in a label, in cells.
///
/// The measure that decides whether a drawing is possible at all: everything
/// here wraps, and wrapping is only lossless while every word still fits on a
/// row of its own.
pub(super) fn longest_word(text: &str) -> usize {
    text.split_whitespace()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
}

/// A node's label, wrapped to the widest box this will draw.
fn wrap_label(label: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    // The break `lex` decoded from `<br>` is honoured: it is a line the author
    // asked for, and a box has room for it.
    for part in label.split('\n') {
        for line in wrap::wrap_spans(vec![Span::raw(part.to_string())], limit, &[], &[]) {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if !text.is_empty() {
                out.push(text);
            }
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// The edges the cycle break cut, written out under the drawing.
///
/// Not dropped and not routed back up the side of the diagram: at forty columns
/// a return line crosses everything between its two ends, and the thing a
/// reader wants from a retry loop is *which two nodes* and *on what condition*,
/// both of which a row of text says exactly.
fn back_rows(graph: &Graph, laid: &Layered, width: usize, theme: &'static Theme) -> Rows {
    let dim = ratatui::style::Style::default().fg(theme.dim);
    let text = ratatui::style::Style::default().fg(theme.fg);
    let mark = ratatui::style::Style::default().fg(theme.accent);
    let mut out = Rows::new();
    for (e, edge) in graph.edges.iter().enumerate() {
        if !laid.back[e] {
            continue;
        }
        let mut spans = vec![
            Span::styled("↩ ", dim),
            Span::styled(graph.nodes[edge.from].label.clone(), text),
            Span::styled(" ─", dim),
        ];
        if let Some(label) = &edge.label {
            spans.push(Span::styled(" ", dim));
            spans.push(Span::styled(label.clone(), mark));
            spans.push(Span::styled(" ─", dim));
        }
        spans.push(Span::styled("▶ ", dim));
        spans.push(Span::styled(graph.nodes[edge.to].label.clone(), text));
        let hang = [Span::styled("  ", dim)];
        out.extend(
            wrap::wrap_spans(spans, width, &[], &hang)
                .into_iter()
                .map(|line| line.spans),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::tests::{assert_fits, assert_keeps, draw};
    use super::*;

    /// A body with its header already taken off, so the parser can be tested
    /// without going through the drawing.
    fn parse(body: &str) -> Option<Graph> {
        Graph::parse(&lex::meaningful_lines(body))
    }

    fn edges(body: &str) -> Vec<(usize, usize, Option<String>)> {
        let graph = parse(body).expect("a body this test wrote itself");
        graph
            .edges
            .iter()
            .map(|e| (e.from, e.to, e.label.clone()))
            .collect()
    }

    fn drawn(src: &str, width: usize) -> Vec<String> {
        draw(src, width).unwrap_or_else(|| panic!("{src:?} declined at {width}"))
    }

    // --- shapes ----------------------------------------------------------

    #[test]
    fn twelve_shapes_arrive_as_three_frames_and_keep_their_text() {
        // The mapping the module note argues for, asserted shape by shape, so
        // that a thirteenth cannot quietly become a rectangle.
        let cases: &[(&str, char, &str)] = &[
            ("A[rect]", '┌', "rect"),
            ("A[[subroutine]]", '┌', "subroutine"),
            ("A>flag]", '┌', "flag"),
            ("A[/para/]", '┌', "para"),
            ("A[\\para\\]", '┌', "para"),
            ("A[/trap\\]", '┌', "trap"),
            ("A[\\trap/]", '┌', "trap"),
            ("A(round)", '╭', "round"),
            ("A([stadium])", '╭', "stadium"),
            ("A[(database)]", '╭', "database"),
            ("A((circle))", '╭', "circle"),
            ("A{decision}", '╔', "decision"),
            ("A{{hexagon}}", '╔', "hexagon"),
        ];
        for (node, corner, text) in cases {
            let rows = drawn(&format!("graph TD\n  {node}\n"), 40);
            assert!(
                rows[0].starts_with(*corner),
                "{node} drew {:?}, which is not a {corner}",
                rows[0]
            );
            assert!(rows[1].contains(text), "{node} lost its label");
        }
    }

    #[test]
    fn a_node_with_no_shape_of_its_own_says_its_id() {
        assert!(drawn("graph TD\n  parse --> draw\n", 40)[1].contains("parse"));
    }

    #[test]
    fn a_shape_written_later_names_a_node_declared_earlier() {
        // `A --> B` and then `B[Stop]` is one node called Stop, not two nodes.
        let graph = parse("A --> B\nB[Stop]").expect("parses");
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[1].label, "Stop");
    }

    // --- edges -----------------------------------------------------------

    #[test]
    fn every_arrow_mermaid_spells_reads_as_a_stroke_and_its_tips() {
        let cases: &[(&str, Stroke, Tip, Tip)] = &[
            ("A --> B", Stroke::Solid, Tip::Arrow, Tip::None),
            ("A --- B", Stroke::Solid, Tip::None, Tip::None),
            ("A -.-> B", Stroke::Dotted, Tip::Arrow, Tip::None),
            ("A -.- B", Stroke::Dotted, Tip::None, Tip::None),
            ("A ==> B", Stroke::Thick, Tip::Arrow, Tip::None),
            ("A === B", Stroke::Thick, Tip::None, Tip::None),
            ("A --o B", Stroke::Solid, Tip::Circle, Tip::None),
            ("A --x B", Stroke::Solid, Tip::Cross, Tip::None),
            ("A <--> B", Stroke::Solid, Tip::Arrow, Tip::Arrow),
            ("A <-.-> B", Stroke::Dotted, Tip::Arrow, Tip::Arrow),
            ("A <==> B", Stroke::Thick, Tip::Arrow, Tip::Arrow),
            ("A o--o B", Stroke::Solid, Tip::Circle, Tip::Circle),
            ("A x--x B", Stroke::Solid, Tip::Cross, Tip::Cross),
            // Mermaid lengthens a link to push two ranks apart. The extra
            // characters are spacing, not a different arrow.
            ("A ----> B", Stroke::Solid, Tip::Arrow, Tip::None),
            ("A==>B", Stroke::Thick, Tip::Arrow, Tip::None),
        ];
        for (src, stroke, head, tail) in cases {
            let graph = parse(src).unwrap_or_else(|| panic!("{src} did not parse"));
            assert_eq!(graph.edges.len(), 1, "{src}");
            let edge = &graph.edges[0];
            assert_eq!(
                (edge.stroke, edge.head, edge.tail),
                (*stroke, *head, *tail),
                "{src}"
            );
        }
    }

    #[test]
    fn a_label_reads_the_same_written_either_way() {
        for src in ["A -->|yes| B", "A -- yes --> B"] {
            assert_eq!(edges(src), [(0, 1, Some("yes".to_string()))], "{src}");
        }
        // ...including on the two strokes that spell their opener otherwise.
        assert_eq!(edges("A -. maybe .-> B"), [(0, 1, Some("maybe".to_string()))]);
        assert_eq!(
            parse("A -. maybe .-> B").map(|g| g.edges[0].stroke),
            Some(Stroke::Dotted)
        );
        assert_eq!(
            parse("A == fast ==> B").map(|g| g.edges[0].stroke),
            Some(Stroke::Thick)
        );
    }

    #[test]
    fn a_chain_declares_every_link_along_it() {
        assert_eq!(edges("A --> B --> C"), [(0, 1, None), (1, 2, None)]);
        assert_eq!(edges("A-->B; B-->C"), [(0, 1, None), (1, 2, None)]);
    }

    #[test]
    fn an_ampersand_group_is_the_cross_product_of_its_two_sides() {
        // Nodes are numbered in declaration order, which reads both groups
        // through before it makes any edge: A, B, C, D.
        assert_eq!(
            edges("A & B --> C & D"),
            [(0, 2, None), (0, 3, None), (1, 2, None), (1, 3, None)]
        );
        let graph = parse("A & B --> C & D").expect("parses");
        let labels: Vec<&str> = graph.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["A", "B", "C", "D"]);
    }

    #[test]
    fn an_arrow_inside_a_label_is_text_rather_than_an_edge() {
        let graph = parse("A[a --> b] --> B").expect("parses");
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].label, "a --> b");
    }

    #[test]
    fn a_word_beginning_with_o_or_x_is_not_an_arrowhead() {
        // `A --- open` would lose its first letter to a `--o` if the tip test
        // did not require the token to end where the tip does.
        let graph = parse("A --- open").expect("parses");
        assert_eq!(graph.nodes[1].label, "open");
        assert_eq!(graph.edges[0].head, Tip::None);
    }

    // --- what gets drawn -------------------------------------------------

    #[test]
    fn each_direction_puts_the_ranks_where_its_name_says() {
        assert_eq!(
            drawn("graph TD\n  A --> B\n", 20),
            ["┌───┐", "│ A │", "└─┬─┘", "  ▼", "┌───┐", "│ B │", "└───┘"]
        );
        assert_eq!(
            drawn("graph BT\n  A --> B\n", 20),
            ["┌───┐", "│ B │", "└───┘", "  ▲", "┌─┴─┐", "│ A │", "└───┘"]
        );
        assert_eq!(
            drawn("graph LR\n  A --> B\n", 20),
            ["┌───┐  ┌───┐", "│ A ├─▶│ B │", "└───┘  └───┘"]
        );
        assert_eq!(
            drawn("graph RL\n  A --> B\n", 20),
            ["┌───┐  ┌───┐", "│ B │◀─┤ A │", "└───┘  └───┘"]
        );
    }

    #[test]
    fn a_decision_and_its_two_labelled_edges_land_where_they_are_drawn() {
        // The common case, whole, in one assertion: a rank centred over the one
        // below it, a `╤` where the connector leaves the frame, one channel per
        // edge that has to move sideways, and each label written along its own
        // run rather than beside it.
        assert_eq!(
            drawn(
                "graph TD\n  A[Start] --> B{Choice}\n  B -->|yes| C[Do it]\n  B -->|no| D[Stop]\n",
                30
            ),
            [
                "     ┌───────┐",
                "     │ Start │",
                "     └───┬───┘",
                "         ▼",
                "    ╔═════════╗",
                "    ║ Choice  ║",
                "    ╚════╤════╝",
                "    ╭yes─┤",
                "    │    ╰─no──╮",
                "    ▼          ▼",
                "┌───────┐  ┌───────┐",
                "│ Do it │  │ Stop  │",
                "└───────┘  └───────┘",
            ]
        );
    }

    #[test]
    fn a_chain_across_the_page_is_three_boxes_and_two_arrows() {
        assert_eq!(
            drawn("flowchart LR\n  parse --> layout --> draw\n", 40),
            [
                "┌───────┐  ┌────────┐  ┌──────┐",
                "│ parse ├─▶│ layout ├─▶│ draw │",
                "└───────┘  └────────┘  └──────┘",
            ]
        );
    }

    #[test]
    fn a_label_across_the_page_is_written_along_the_leg_it_belongs_to() {
        assert_eq!(
            drawn("graph LR\n  A -->|yes| B\n  A -->|no| C\n", 40),
            [
                "                  ┌───┐",
                "          ╭──────▶│ B │",
                "┌───┐     │       └───┘",
                "│ A ├─yes─┴╮",
                "└───┘      │",
                "           │      ┌───┐",
                "           ╰─no──▶│ C │",
                "                  └───┘",
            ]
        );
    }

    #[test]
    fn a_fan_leaves_one_port_and_each_arrow_lands_on_its_own_box() {
        assert_eq!(
            drawn("graph TD\n  A --> B\n  A --> C\n  A --> D\n  A --> E\n", 40),
            [
                "          ┌───┐",
                "          │ A │",
                "          └─┬─┘",
                "  ╭─────────┤",
                "  │      ╭──┤",
                "  │      │  ├───╮",
                "  │      │  ╰───┼──────╮",
                "  ▼      ▼      ▼      ▼",
                "┌───┐  ┌───┐  ┌───┐  ┌───┐",
                "│ B │  │ C │  │ D │  │ E │",
                "└───┘  └───┘  └───┘  └───┘",
            ]
        );
    }

    #[test]
    fn a_diamond_routes_its_long_edge_through_a_column_of_its_own() {
        // `A --> D` spans two ranks, so it gets a slot of its own in the rank
        // it passes through: the `│` down the right of the middle rank. Without
        // it that connector would have to cross a box.
        assert_eq!(
            drawn(
                "graph TD\n  A --> B\n  A --> C\n  B --> D\n  C --> D\n  A --> D\n",
                40
            ),
            [
                "     ┌───┐",
                "     │ A │",
                "     └─┬─┘",
                "  ╭────┤",
                "  │    ├─╮",
                "  │    ╰─┼────╮",
                "  ▼      ▼    │",
                "┌───┐  ┌───┐  │",
                "│ B │  │ C │  │",
                "└─┬─┘  └─┬─┘  │",
                "  ╰────╮ │    │",
                "       ├─╯    │",
                "       ├──────╯",
                "       ▼",
                "     ┌───┐",
                "     │ D │",
                "     └───┘",
            ]
        );
    }

    #[test]
    fn two_components_that_never_meet_are_drawn_side_by_side() {
        assert_eq!(
            drawn("graph TD\n  A --> B\n  C --> D\n", 20),
            [
                "┌───┐  ┌───┐",
                "│ A │  │ C │",
                "└─┬─┘  └─┬─┘",
                "  ▼      ▼",
                "┌───┐  ┌───┐",
                "│ B │  │ D │",
                "└───┘  └───┘",
            ]
        );
    }

    #[test]
    fn wide_characters_are_measured_in_cells_rather_than_in_characters() {
        // Eight ideographs are sixteen cells, and every row of the box has to
        // agree about that or the drawing shears.
        let rows = drawn("graph TD\n  A[日本語版のノード] --> B[短い]\n", 40);
        assert_eq!(
            rows,
            [
                "┌───────────────────┐",
                "│ 日本語版のノード  │",
                "└─────────┬─────────┘",
                "          ▼",
                "      ┌───────┐",
                "      │ 短い  │",
                "      └───────┘",
            ]
        );
        assert_eq!(rows[0].width(), rows[1].width());
    }

    #[test]
    fn a_label_the_author_broke_is_drawn_on_the_lines_they_broke_it_into() {
        assert_eq!(
            drawn("graph TD\n  A[first<br/>second] --> B\n", 40),
            [
                "┌─────────┐",
                "│  first  │",
                "│ second  │",
                "└────┬────┘",
                "     ▼",
                "   ┌───┐",
                "   │ B │",
                "   └───┘",
            ]
        );
    }

    #[test]
    fn a_dotted_edge_and_a_thick_one_are_drawn_in_glyphs_of_their_own() {
        assert!(drawn("graph TD\n  A -.-> B\n", 20).iter().any(|r| r.contains('┊')));
        assert!(drawn("graph TD\n  A ==> B\n", 20).iter().any(|r| r.contains('┃')));
        // A one-row band has nowhere to draw a stroke, so a band carrying one
        // that is not solid is given a second row.
        assert_eq!(drawn("graph TD\n  A --> B\n", 20).len(), 7);
        assert_eq!(drawn("graph TD\n  A -.-> B\n", 20).len(), 8);
    }

    #[test]
    fn a_two_way_edge_is_marked_once_at_the_end_it_arrives_at() {
        assert!(drawn("graph TD\n  A <--> B\n", 20).iter().any(|r| r.contains('↕')));
        assert!(drawn("graph LR\n  A <--> B\n", 20).iter().any(|r| r.contains('↔')));
        assert!(!drawn("graph TD\n  A --> B\n", 20).iter().any(|r| r.contains('↕')));
    }

    // --- cycles ----------------------------------------------------------

    #[test]
    fn a_cycle_is_broken_and_the_edge_it_cost_is_written_under_the_drawing() {
        assert_eq!(
            drawn("graph TD\n  A --> B\n  B --> C\n  C -->|retry| A\n", 30),
            [
                "┌───┐",
                "│ A │",
                "└─┬─┘",
                "  ▼",
                "┌───┐",
                "│ B │",
                "└─┬─┘",
                "  ▼",
                "┌───┐",
                "│ C │",
                "└───┘",
                "",
                "↩ C ─ retry ─▶ A",
            ]
        );
    }

    #[test]
    fn a_self_loop_is_a_cycle_of_one_and_is_written_out_the_same_way() {
        assert_eq!(
            drawn("graph TD\n  A --> A\n", 20),
            ["┌───┐", "│ A │", "└───┘", "", "↩ A ─▶ A"]
        );
    }

    #[test]
    fn the_same_cycle_is_broken_at_the_same_edge_every_time() {
        // Which edge gets cut is arbitrary; being arbitrary twice for one file
        // is a diagram that redraws differently on a keystroke.
        let src = "graph TD\n  A --> B\n  B --> C\n  C --> A\n  C --> B\n";
        let first = drawn(src, 40);
        for _ in 0..4 {
            assert_eq!(drawn(src, 40), first);
        }
    }

    // --- the outline, and the threshold between the two ------------------

    #[test]
    fn the_drawing_gives_way_to_the_outline_the_column_before_it_would_not_fit() {
        let src = "graph TD\n  A[Start] --> B{Choice}\n  B -->|yes| C[Do it]\n  B -->|no| D[Stop]\n";
        let boxed = |rows: &[String]| rows.iter().any(|row| row.contains('╔'));
        assert!(boxed(&drawn(src, 20)));
        assert!(!boxed(&drawn(src, 19)));
        assert_eq!(
            drawn(src, 19),
            [
                "Start",
                "└─▶ Choice",
                "    ├─ yes ─▶ Do it",
                "    └─ no  ─▶ Stop",
            ]
        );
    }

    #[test]
    fn nothing_is_lost_at_any_width_this_draws_a_flowchart_at() {
        let src = "graph TD\n  A[Start] --> B{Choice}\n  B -->|yes| C[Do it]\n  B -->|no| D[Stop]\n";
        for width in 4..=60 {
            let Some(rows) = draw(src, width) else {
                continue;
            };
            let what = format!("the flowchart at {width}");
            assert_fits(&rows, width, &what);
            assert_keeps(&rows, &["Start", "Choice", "yes", "Do it", "no", "Stop"], &what);
        }
    }

    // --- declining -------------------------------------------------------

    #[test]
    fn a_word_wider_than_any_row_declines_rather_than_being_broken_in_half() {
        // Half a word is not the word, and the source always is — the rule
        // `tests::assert_keeps` states, enforced where it is decided.
        let src = "graph TD\n  A[supercalifragilistic] --> B\n";
        assert!(draw(src, 12).is_none());
        // Twenty columns is exactly the word: no room for a frame around it,
        // but the outline gives a row the whole width and loses nothing.
        assert_eq!(drawn(src, 20), ["supercalifragilistic", "└─▶ B"]);
        assert!(drawn(src, 30)[1].contains("supercalifragilistic"));
    }

    #[test]
    fn a_subgraph_or_a_click_declines_because_it_carries_text_this_cannot_draw() {
        for src in [
            "graph TD\n  subgraph one\n  A --> B\n  end\n",
            "graph TD\n  A --> B\n  click A \"http://x.dev\" \"open it\"\n",
        ] {
            assert!(draw(src, 60).is_none(), "{src:?} should have declined");
        }
    }

    #[test]
    fn styling_statements_are_skipped_in_silence_because_no_text_is_in_them() {
        for tail in [
            "style A fill:#f9f",
            "classDef big font-size:20px",
            "class A big",
            "linkStyle 0 stroke:red",
        ] {
            let rows = drawn(&format!("graph TD\n  A --> B\n  {tail}\n"), 40);
            assert_eq!(rows.len(), 7, "{tail} changed the drawing");
        }
        // A class attached to a node is styling too, and would take the node
        // with it if `:::` were read as part of the id.
        assert_eq!(
            parse("A:::big --> B").map(|g| g.nodes[0].label.clone()),
            Some("A".to_string())
        );
    }

    #[test]
    fn a_statement_this_cannot_read_declines_the_whole_diagram() {
        for src in [
            "graph TD\n  A[unclosed --> B\n",
            "graph TD\n  A@{ shape: rect } --> B\n",
            "graph TD\n  accTitle: a title\n  A --> B\n",
            "graph TD\n  A ==>\n",
        ] {
            assert!(draw(src, 60).is_none(), "{src:?} should have declined");
        }
    }

    #[test]
    fn a_diagram_past_the_caps_is_left_as_source_rather_than_laid_out() {
        let nodes: String = (0..super::super::MAX_NODES + 1)
            .map(|i| format!("  n{i}[node {i}]\n"))
            .collect();
        assert!(draw(&format!("graph TD\n{nodes}"), 60).is_none());

        let edges: String = (0..super::super::MAX_EDGES + 1)
            .map(|i| format!("  a --> b{}\n", i % 3))
            .collect();
        assert!(draw(&format!("graph TD\n{edges}"), 60).is_none());
    }

    #[test]
    fn every_glyph_the_drawing_uses_is_one_cell_wide() {
        // The layout is counted in cells, so a glyph a terminal draws in two
        // would shear every row it appears in. The arrowheads, `●` and `↕` are
        // the risk — all East Asian Ambiguous — and this is the assertion that
        // records which set was checked.
        for glyph in "─│┄┊━┃╭╮╰╯┌┐└┘╔╗╚╝═║┬┴├┤┼╤╧╟╢┏┓┗┛┣┫┳┻╋▲▼◀▶↕↔●╳↩".chars() {
            assert_eq!(
                unicode_width::UnicodeWidthChar::width(glyph),
                Some(1),
                "{glyph:?} is not one cell"
            );
        }
    }
}
