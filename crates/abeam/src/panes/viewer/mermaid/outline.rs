//! The flowchart a pane is too narrow to draw, written as an indented outline.
//!
//! The same argument `markdown::emit_table_as_records` makes about a grid: at
//! forty columns a four-cell-wide box is not a diagram, it is a puzzle, and the
//! reader is better served by the data laid out plainly than by a picture of it
//! that no longer fits. So the graph is walked depth-first and written one edge
//! per row, with the arrow and its label on the row:
//!
//! ```text
//! Start
//! └─▶ Choice?
//!     ├─ yes ─▶ Do it
//!     └─ no  ─▶ Stop
//! ```
//!
//! This is the fallback that must never itself fail. Labels wrap across the
//! full width here rather than being sized into a box, the indent is clamped
//! the way `markdown` clamps a nested list's, and the connector has three forms
//! — aligned label, plain arrow, bare marker — chosen by measuring, so that
//! there is no width from four columns up at which a flowchart parses and then
//! produces nothing.
//!
//! ## The tree is a tree, so a node is drawn once
//!
//! A flowchart is not one. A node reached twice would be drawn twice, and a
//! node reached in a cycle would be drawn forever, which is the failure mode
//! this exists to avoid — so the second arrival prints the label and a `↩`
//! rather than the subtree, and the reader follows the name back up the page.
//! Nothing is lost by that: every node is written where it is first reached,
//! and every edge is a row.

use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::Rows;
use super::flow::{Graph, Stroke, Tip, longest_word};
use crate::panes::viewer::theme::Theme;
use crate::text::wrap::{self, spans_width};

/// Cells one level of nesting costs. Four, so that a connector and a space fit
/// inside it and the labels below a branch line up under each other.
const STEP: usize = 4;

/// Cells kept clear to the right of the deepest indent. Past this, nesting
/// stops advancing and the structure comes from the connectors alone — the
/// same trade `markdown::prefixes` makes with `CRAMPED`, for the same reason:
/// a document that is all structure must not spend the whole pane saying so.
const KEEP: usize = 10;

/// Rows past which even the outline gives up and the source is shown.
///
/// The caps in [`super`] already bound the graph; this bounds the *rendering*
/// of one, which a four-column pane can multiply by the wrap factor of a long
/// label. Declining rather than truncating, because a truncated outline is the
/// fourth outcome the module note rules out, and at a thousand rows the fence's
/// own source is both shorter and true.
const ROW_CAP: usize = 1000;

pub fn render(graph: &Graph, width: usize, theme: &'static Theme) -> Option<Rows> {
    let mut out = Outline {
        graph,
        edges: adjacency(graph),
        seen: vec![false; graph.nodes.len()],
        rows: Rows::new(),
        cramped: false,
        width,
        theme,
    };

    // Roots first, in declaration order, so the outline reads the way the file
    // does. Everything else follows, because a node inside a cycle has no root
    // to be reached from and still has to appear.
    let mut incoming = vec![0usize; graph.nodes.len()];
    for edge in &graph.edges {
        incoming[edge.to] += 1;
    }
    let roots = (0..graph.nodes.len()).filter(|&n| incoming[n] == 0);
    for node in roots.chain(0..graph.nodes.len()) {
        if out.seen[node] {
            continue;
        }
        if !out.rows.is_empty() {
            out.rows.push(Vec::new());
        }
        out.seen[node] = true;
        let label = out.text(&graph.nodes[node].label);
        out.emit(vec![label], &[], &[]);
        out.walk(node, String::new());
    }

    // `cramped` is the one thing the outline cannot wrap its way out of: a
    // word wider than the row it is given comes out in pieces, and a diagram
    // that has broken `Choice` into `Choi` and `ce` has lost it. At that width
    // the fence's own source is the better answer, and it is always true.
    let usable = !out.cramped && !out.rows.is_empty() && out.rows.len() <= ROW_CAP;
    usable.then_some(out.rows)
}

/// Which of the three connectors a fan of edges is drawn with. See
/// [`Outline::form`] for how one is chosen, and the module note for why there
/// are three.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// `├─ yes ─▶ `, labels padded to a common width.
    Aligned(usize),
    /// `├─▶ `, with the label moved into the row's own text.
    Plain,
    /// `▶ `, with no indent left to spend.
    Bare,
}

struct Outline<'a> {
    graph: &'a Graph,
    edges: Vec<Vec<usize>>,
    seen: Vec<bool>,
    rows: Rows,
    /// Set when a row was handed a word it could not hold. See [`render`].
    cramped: bool,
    width: usize,
    theme: &'static Theme,
}

impl Outline<'_> {
    /// One node's outgoing edges, then their subtrees.
    ///
    /// Recursive, and bounded by the node cap in [`super`] rather than by the
    /// document: a node is marked seen *before* it is descended into, so every
    /// node is expanded at most once and the recursion cannot outlive the
    /// hundred-and-twenty-eight nodes a diagram is allowed.
    fn walk(&mut self, node: usize, trunk: String) {
        let edges = self.edges[node].clone();
        let form = self.form(trunk.width(), &edges);

        for (i, &e) in edges.iter().enumerate() {
            let edge = &self.graph.edges[e];
            let last = i + 1 == edges.len();
            let (to, label) = (edge.to, edge.label.clone());
            let known = self.seen[to];

            let (first, rest) = self.connector(&trunk, last, e, form);
            let mut content = vec![self.text(&self.graph.nodes[to].label)];
            if !matches!(form, Form::Aligned(_))
                && let Some(label) = &label
            {
                content.push(Span::styled(format!(" ({label})"), self.accent()));
            }
            if known {
                // Already written out above, with its own subtree under it.
                content.push(Span::styled(" ↩", self.dim()));
            }
            self.emit(content, &first, &rest);

            if !known {
                self.seen[to] = true;
                let step = if last { "    " } else { "│   " };
                // Past `KEEP` the indent stops growing rather than eating the
                // pane. It looks flat and it terminates, which is the correct
                // trade at a width where nothing else would fit either.
                let deeper = if trunk.width() + STEP + KEEP <= self.width {
                    format!("{trunk}{step}")
                } else {
                    trunk.clone()
                };
                self.walk(to, deeper);
            }
        }
    }

    /// Which connector a node's whole fan of edges is drawn with.
    ///
    /// Chosen once for the group rather than once per row, because the choice
    /// is visible: two siblings drawn in two different forms read as two
    /// different kinds of thing, and one of them is `└─▶ Stop` while the other
    /// is `▶ Do it`. And chosen by measuring the room each form would leave for
    /// the row's own longest word, not against a fixed margin — measuring
    /// against two spare cells is what made a twelve-column pane decline a
    /// diagram the ten-column one drew.
    fn form(&self, indent: usize, edges: &[usize]) -> Form {
        let mut widest = 0;
        let mut bare = 0;
        let mut joined = 0;
        for &e in edges {
            let edge = &self.graph.edges[e];
            let word = longest_word(&self.graph.nodes[edge.to].label);
            bare = bare.max(word);
            joined = joined.max(match edge.label.as_deref() {
                Some(label) => word.max(longest_word(&format!("({label})"))),
                None => word,
            });
            if let Some(label) = edge.label.as_deref() {
                widest = widest.max(label.width());
            }
        }
        if widest > 0 && indent + widest + 7 + bare <= self.width {
            Form::Aligned(widest)
        } else if indent + 4 + joined <= self.width {
            Form::Plain
        } else {
            Form::Bare
        }
    }

    /// The prefixes one edge's row is drawn with: the first line's, and the one
    /// its wrapped continuations hang under.
    fn connector(
        &self,
        trunk: &str,
        last: bool,
        edge: usize,
        form: Form,
    ) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let edge = &self.graph.edges[edge];
        let dash = match edge.stroke {
            Stroke::Solid => '─',
            Stroke::Dotted => '┄',
            Stroke::Thick => '━',
        };
        let tip = match edge.head {
            Tip::Arrow => '▶',
            Tip::Circle => '●',
            Tip::Cross => '╳',
            Tip::None => dash,
        };
        let corner = if last { '└' } else { '├' };
        // The last child closes its branch, so nothing below it continues the
        // line — which is what makes the indent readable as a tree at all.
        let branch = if last { ' ' } else { '│' };

        match form {
            // `├─ yes ─▶ `, with every sibling's label padded to one width so
            // the arrowheads line up: the one piece of alignment that makes a
            // fan of conditions read as a set rather than as unrelated rows.
            Form::Aligned(widest) => {
                let head = match edge.label.as_deref() {
                    Some(label) => vec![
                        Span::styled(format!("{trunk}{corner}{dash} "), self.dim()),
                        Span::styled(label.to_string(), self.accent()),
                        Span::styled(
                            format!("{} {dash}{tip} ", " ".repeat(widest - label.width())),
                            self.dim(),
                        ),
                    ],
                    // An unlabelled edge in a labelled fan keeps the width and
                    // spends it on line rather than on space, so the column of
                    // arrowheads survives.
                    None => vec![Span::styled(
                        format!(
                            "{trunk}{corner}{dash}{}{dash}{tip} ",
                            dash.to_string().repeat(widest + 2)
                        ),
                        self.dim(),
                    )],
                };
                let rest = vec![Span::styled(
                    format!("{trunk}{branch}{}", " ".repeat(widest + 6)),
                    self.dim(),
                )];
                (head, rest)
            }
            // `├─▶ `, and the label joins the text as `Do it (yes)`.
            Form::Plain => (
                vec![Span::styled(
                    format!("{trunk}{corner}{dash}{tip} "),
                    self.dim(),
                )],
                vec![Span::styled(format!("{trunk}{branch}   "), self.dim())],
            ),
            // Nothing left but the fact that this is an edge.
            Form::Bare => (
                vec![Span::styled(format!("{tip} "), self.dim())],
                vec![Span::styled("  ", self.dim())],
            ),
        }
    }

    fn emit(&mut self, content: Vec<Span<'static>>, first: &[Span<'static>], rest: &[Span<'static>]) {
        // Measured against the *continuation* prefix, which is the limit
        // `wrap` applies to every line but the first — and a word that will not
        // fit the first line is moved down to one that it does.
        let room = self.width.saturating_sub(spans_width(rest)).max(1);
        let longest = content
            .iter()
            .flat_map(|span| span.content.split_whitespace())
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0);
        self.cramped |= longest > room;
        self.rows.extend(
            wrap::wrap_spans(content, self.width, first, rest)
                .into_iter()
                .map(|line| line.spans),
        );
    }

    fn text(&self, label: &str) -> Span<'static> {
        // A `<br>` the author wrote is a space here: the outline's own wrapping
        // is what decides where this label breaks, and honouring both would
        // leave a two-word row in the middle of a column of full ones.
        Span::styled(label.replace('\n', " "), Style::default().fg(self.theme.fg))
    }

    fn dim(&self) -> Style {
        Style::default().fg(self.theme.dim)
    }

    fn accent(&self) -> Style {
        Style::default().fg(self.theme.accent)
    }
}

fn adjacency(graph: &Graph) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); graph.nodes.len()];
    for (e, edge) in graph.edges.iter().enumerate() {
        out[edge.from].push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::lex;
    use super::super::tests::{assert_fits, assert_keeps, flatten};
    use super::*;
    use crate::panes::viewer::theme::Mode;

    /// A flowchart body — the header is the caller's business — laid out as an
    /// outline whatever the width would otherwise have chosen.
    fn outline(body: &str, width: usize) -> Vec<String> {
        try_outline(body, width).unwrap_or_else(|| panic!("declined at {width}"))
    }

    fn try_outline(body: &str, width: usize) -> Option<Vec<String>> {
        let graph = Graph::parse(&lex::meaningful_lines(body)).expect("a body a test wrote");
        render(&graph, width, Mode::Dark.theme()).map(flatten)
    }

    const CHOICE: &str = "A[Start] --> B{Choice}\nB -->|yes| C[Do it]\nB -->|no| D[Stop]";

    #[test]
    fn a_tree_hangs_each_child_under_its_parent_with_the_label_on_the_arrow() {
        assert_eq!(
            outline(CHOICE, 30),
            [
                "Start",
                "└─▶ Choice",
                "    ├─ yes ─▶ Do it",
                "    └─ no  ─▶ Stop",
            ]
        );
    }

    #[test]
    fn a_narrower_pane_moves_the_label_into_the_row_it_labels() {
        // The connector shrinks to four cells and `yes` follows the node it
        // belongs to, which is the only place left that is still on the row.
        assert_eq!(
            outline(CHOICE, 10),
            [
                "Start",
                "└─▶ Choice",
                "├─▶ Do it",
                "│   (yes)",
                "└─▶ Stop",
                "    (no)",
            ]
        );
    }

    #[test]
    fn a_pane_with_no_room_for_a_branch_keeps_the_arrow_and_gives_up_the_tree() {
        assert_eq!(
            outline(CHOICE, 8),
            ["Start", "▶ Choice", "▶ Do it", "  (yes)", "▶ Stop", "  (no)"]
        );
    }

    #[test]
    fn one_fan_is_drawn_in_one_form_rather_than_a_row_of_each() {
        // Two siblings in two different connectors read as two different kinds
        // of thing. Eight columns is where `(yes)` stops fitting beside `└─▶ `
        // while `(no)` still would.
        let rows = outline(CHOICE, 8);
        assert!(rows[2].starts_with('▶') && rows[4].starts_with('▶'), "{rows:?}");
    }

    #[test]
    fn a_node_reached_twice_is_named_the_second_time_rather_than_drawn_again() {
        // Both paths through the diamond have to end somewhere the reader can
        // see; only one of them may own the subtree, or the outline is a copy
        // of itself.
        assert_eq!(
            outline("A --> B\nA --> C\nB --> D\nC --> D", 30),
            ["A", "├─▶ B", "│   └─▶ D", "└─▶ C", "    └─▶ D ↩"]
        );
    }

    #[test]
    fn a_cycle_terminates_and_says_where_it_came_back_to() {
        assert_eq!(
            outline("A --> B\nB --> C\nC --> A", 30),
            ["A", "└─▶ B", "    └─▶ C", "        └─▶ A ↩"]
        );
    }

    #[test]
    fn a_self_loop_is_a_cycle_of_one_and_terminates_the_same_way() {
        assert_eq!(outline("A --> A", 30), ["A", "└─▶ A ↩"]);
    }

    #[test]
    fn two_components_each_get_a_root_and_a_blank_row_between_them() {
        assert_eq!(
            outline("A --> B\nC --> D", 30),
            ["A", "└─▶ B", "", "C", "└─▶ D"]
        );
    }

    #[test]
    fn a_node_with_no_edges_at_all_is_still_a_row() {
        assert_eq!(outline("only[on its own]", 30), ["on its own"]);
    }

    #[test]
    fn every_stroke_and_every_tip_keeps_a_glyph_of_its_own() {
        assert_eq!(
            outline("A -.-> B\nA ==> C\nA --o D\nA --x E\nA --- F", 30),
            ["A", "├┄▶ B", "├━▶ C", "├─● D", "├─╳ E", "└── F"]
        );
    }

    #[test]
    fn a_label_wider_than_the_pane_wraps_under_itself_rather_than_overflowing() {
        assert_eq!(
            outline("A[a really quite long label on one node] --> B[short]", 16),
            ["a really quite", "long label on", "one node", "└─▶ short"]
        );
    }

    #[test]
    fn wide_characters_are_measured_in_cells_rather_than_in_characters() {
        assert_eq!(
            outline("A[日本語版のノード] --> B[短い]", 20),
            ["日本語版のノード", "└─▶ 短い"]
        );
        // Eight ideographs are sixteen cells, so there is no room for them and
        // a connector at twelve — and no way to break the word either.
        assert!(try_outline("A[日本語版のノード] --> B[短い]", 12).is_none());
    }

    #[test]
    fn a_word_that_will_not_fit_a_row_declines_rather_than_coming_out_in_halves() {
        let long = "A[supercalifragilistic] --> B";
        assert!(try_outline(long, 12).is_none());
        assert_eq!(outline(long, 20), ["supercalifragilistic", "└─▶ B"]);
    }

    #[test]
    fn nesting_stops_indenting_before_it_has_eaten_the_pane() {
        // Six deep in thirty columns: the indent stops at sixteen cells, which
        // is where four more would leave less than `KEEP` for the text. Flat
        // and readable beats nested and one letter wide.
        assert_eq!(
            outline("a --> b --> c --> d --> e --> f", 30),
            [
                "a",
                "└─▶ b",
                "    └─▶ c",
                "        └─▶ d",
                "            └─▶ e",
                "                └─▶ f",
            ]
        );
    }

    #[test]
    fn nothing_overflows_and_every_node_is_named_at_any_width_it_draws_at() {
        for width in 4..=60 {
            let Some(rows) = try_outline(CHOICE, width) else {
                continue;
            };
            let what = format!("the outline at {width}");
            assert_fits(&rows, width, &what);
            // By word rather than by label: `Do it` wraps inside its own row at
            // the narrow end, which is the outline working rather than failing.
            assert_keeps(&rows, &["Start", "Choice", "yes", "Do it", "no", "Stop"], &what);
        }
    }
}
