//! The rules that hold for every diagram this module draws, at every width.
//!
//! The per-family tests live beside the code that draws each family. These are
//! the cross-cutting ones — the three outcomes in the module note, made into
//! assertions — and they are here rather than there because a rule that only
//! one drawer is tested against is a rule the other one is free to break.

use crate::panes::viewer::theme::{Mode, DARK};
use super::*;

/// Rendered rows as plain strings, or `None` if the diagram declined.
pub fn draw(source: &str, width: usize) -> Option<Vec<String>> {
    render(source, width, Mode::Dark.theme()).map(flatten)
}

pub fn flatten(rows: Rows) -> Vec<String> {
    rows.iter()
        .map(|row| row.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

/// The invariant the pane depends on, and the one a layout bug breaks first.
pub fn assert_fits(rows: &[String], width: usize, what: &str) {
    for row in rows {
        assert!(
            unicode_width::UnicodeWidthStr::width(row.as_str()) <= width,
            "{what}: {row:?} is {} cells, pane is {width}",
            unicode_width::UnicodeWidthStr::width(row.as_str())
        );
    }
}

/// Every word of every label, still on screen somewhere.
///
/// Deliberately by word rather than by whole label: a label wraps inside its
/// own box and wraps again in the outline, so the string is not intact
/// anywhere — but a *word* survives both, and a drawing that has dropped a node
/// has dropped its words with it. This is how the "never a fourth outcome" rule
/// in the module note is enforced rather than merely stated.
pub fn assert_keeps(rows: &[String], labels: &[&str], what: &str) {
    let all = rows.join("\n");
    for label in labels {
        for word in label.split_whitespace() {
            assert!(
                all.contains(word),
                "{what}: {word:?} from {label:?} is not in the drawing:\n{all}"
            );
        }
    }
}

/// One of each shape of input, with everything the reader must still be able to
/// find afterwards. Kept together so a change to either drawer is checked
/// against the other's corpus too.
pub const CORPUS: &[(&str, &[&str])] = &[
    (
        "graph TD\n  A[Start] --> B{Choice}\n  B -->|yes| C[Do it]\n  B -->|no| D[Stop]\n",
        &["Start", "Choice", "yes", "Do it", "no", "Stop"],
    ),
    (
        "flowchart LR\n  parse --> layout --> draw\n",
        &["parse", "layout", "draw"],
    ),
    (
        "graph TD\n  A[a really quite long label on one node] --> B[short]\n",
        &["a really quite long label on one node", "short"],
    ),
    (
        "sequenceDiagram\n  Alice->>Bob: Hello Bob\n  Bob-->>Alice: Hi Alice\n",
        &["Alice", "Bob", "Hello Bob", "Hi Alice"],
    ),
    (
        "sequenceDiagram\n  participant W as Watcher\n  participant V as Viewer\n  W->>V: file changed\n",
        &["Watcher", "Viewer", "file changed"],
    ),
];

/// Pathological shapes that must produce *something or nothing*, never a panic
/// and never an over-wide row. No labels are asserted: several of these are not
/// valid mermaid, and declining them is a correct answer.
const HOSTILE: &[&str] = &[
    // A cycle, which a layered layout has to break rather than rank forever.
    "graph TD\n  A --> B\n  B --> C\n  C --> A\n",
    // A self-loop, which is a cycle of one.
    "graph TD\n  A --> A\n",
    // Nothing but a header.
    "graph TD\n",
    "sequenceDiagram\n",
    // Half-typed, because the watcher shows us the file mid-write.
    "graph TD\n  A[unclosed --> B\n",
    "sequenceDiagram\n  Alice->>\n",
    // One node, no edges.
    "graph LR\n  only[on its own]\n",
    // A label wider than any pane.
    "graph TD\n  A[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa] --> B\n",
    // Wide characters, which `str::len` is wrong about twice over.
    "graph TD\n  A[日本語版のノード] --> B[短い]\n",
    "sequenceDiagram\n  日本->>語版: メッセージ\n",
    // Every direction, since two of them are the other two reversed.
    "graph BT\n  A --> B\n",
    "graph RL\n  A --> B\n",
    // A fan, which is what makes ordering matter.
    "graph TD\n  A --> B\n  A --> C\n  A --> D\n  A --> E\n  A --> F\n  A --> G\n",
    // Disconnected components.
    "graph TD\n  A --> B\n  C --> D\n",
];

#[test]
fn a_diagram_type_this_does_not_draw_declines_rather_than_guessing() {
    // Each of these is real mermaid that the reader's browser draws and this
    // does not. The fence keeps its source, which is never wrong.
    for src in [
        "pie title Votes\n  \"a\" : 10\n",
        "classDiagram\n  Animal <|-- Duck\n",
        "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n",
        "gantt\n  title A\n",
        "mindmap\n  root\n",
        "stateDiagram-v2\n  [*] --> Still\n",
        "journey\n  title My day\n",
        "",
        "   \n\n",
        "%% only a comment\n",
    ] {
        assert!(draw(src, 60).is_none(), "{src:?} should have declined");
    }
}

#[test]
fn the_keyword_is_read_the_way_mermaid_reads_it() {
    assert!(draw("graph TD\n A-->B\n", 60).is_some());
    assert!(draw("flowchart TD\n A-->B\n", 60).is_some());
    assert!(draw("graph TD; A-->B\n", 60).is_some());
    // Case-sensitive, because mermaid is: drawing something the reader's own
    // browser refuses to draw would put abeam and the file in disagreement.
    assert!(draw("Graph TD\n A-->B\n", 60).is_none());
    assert!(draw("sequencediagram\n A->>B: x\n", 60).is_none());
    // A direction mermaid does not have is not a flowchart with a default.
    assert!(draw("graph XY\n A-->B\n", 60).is_none());
    // ...but no direction at all is `TD`, which is mermaid's own default.
    assert!(draw("graph\n A-->B\n", 60).is_some());
}

#[test]
fn the_fence_info_string_is_matched_on_its_first_word_only() {
    assert!(is_mermaid("mermaid"));
    assert!(is_mermaid("Mermaid"));
    assert!(is_mermaid("mermaid title=flow"));
    assert!(!is_mermaid("mermaidjs"));
    assert!(!is_mermaid("rust"));
    assert!(!is_mermaid(""));
}

#[test]
fn nothing_overflows_the_pane_and_nothing_is_lost_at_any_width() {
    // Four columns is the narrowest this accepts at all; sixty is a generous
    // right pane. Every width between has to hold both rules at once.
    for (src, labels) in CORPUS {
        for width in 4..=60 {
            let Some(rows) = draw(src, width) else {
                continue;
            };
            let what = format!("{src:?} at {width}");
            assert_fits(&rows, width, &what);
            assert_keeps(&rows, labels, &what);
        }
    }
}

#[test]
fn a_pane_too_narrow_for_anything_declines_rather_than_drawing_rubble() {
    for width in 0..4 {
        for (src, _) in CORPUS {
            assert!(draw(src, width).is_none(), "{src:?} drew at {width}");
        }
    }
}

#[test]
fn hostile_input_terminates_without_panicking_or_overflowing() {
    for src in HOSTILE {
        for width in 4..=60 {
            if let Some(rows) = draw(src, width) {
                assert_fits(&rows, width, &format!("{src:?} at {width}"));
            }
        }
    }
}

#[test]
fn a_diagram_past_the_caps_is_shown_as_source_rather_than_laid_out() {
    // The draw path is a frame's worth of budget, not a graph-drawing package.
    let many_nodes = (0..MAX_NODES * 2)
        .map(|i| format!("  n{i}[node {i}]\n"))
        .collect::<String>();
    assert!(draw(&format!("graph TD\n{many_nodes}"), 60).is_none());

    let many_edges = (0..MAX_EDGES * 2)
        .map(|i| format!("  a --> b{}\n", i % 4))
        .collect::<String>();
    assert!(draw(&format!("graph TD\n{many_edges}"), 60).is_none());

    let huge = "graph TD\n".to_string() + &"  A --> B\n".repeat(MAX_BYTES);
    assert!(draw(&huge, 60).is_none());
}

#[test]
fn both_palettes_draw_the_same_diagram_with_the_same_text() {
    // The pane paints its own page, so every colour here is owned (see
    // `viewer::theme`). What must not differ between the two is the *layout*:
    // a reader pressing F3 is changing the light, not the diagram.
    for (src, _) in CORPUS {
        let dark = render(src, 48, Mode::Dark.theme()).map(flatten);
        let light = render(src, 48, Mode::Light.theme()).map(flatten);
        assert_eq!(dark, light, "{src:?} lays out differently by palette");
    }
}

#[test]
fn every_span_a_diagram_emits_names_a_colour_from_the_palette() {
    // The page is painted, so a span with no foreground inherits `Theme::fg`,
    // which is the body colour and correct for label text. What must never
    // appear is a colour from *outside* the palette: the pane's background is
    // absolute RGB and an ANSI name resolved against the terminal's profile
    // would land unreadably on it.
    let palette = [
        DARK.fg, DARK.dim, DARK.code, DARK.link, DARK.accent, DARK.ok, DARK.warn, DARK.danger,
        DARK.special, DARK.info,
    ];
    for (src, _) in CORPUS {
        let Some(rows) = render(src, 48, Mode::Dark.theme()) else {
            continue;
        };
        for span in rows.iter().flatten() {
            if let Some(fg) = span.style.fg {
                assert!(
                    palette.contains(&fg),
                    "{src:?} draws {:?} in {fg:?}, which is not in the palette",
                    span.content
                );
            }
        }
    }
}
