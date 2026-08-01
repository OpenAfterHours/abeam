//! Where the two panes go. One calculation, used by both drawing and resizing.
//!
//! The spike derived the pty size in a `pty_dims()` that re-computed the split
//! and the border inset independently of the function that drew them. Two
//! calculations that must agree is where "off-by-one here is what makes hosted
//! apps wrap strangely" comes from. There is one here, and it is called once
//! per frame.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Block;

/// Below this, a 40% right pane is too narrow to be worth anything while the
/// remaining 60% is actively bad for Claude. Collapsing is the right
/// degradation; squeezing is not.
pub const MIN_SPLIT_COLS: u16 = 60;

/// Outer rects, borders included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub left: Rect,
    /// `None` when zoomed, or when the window is too narrow to split.
    pub right: Option<Rect>,
}

pub fn split(area: Rect, zoom: bool) -> Split {
    if zoom || area.width < MIN_SPLIT_COLS {
        return Split {
            left: area,
            right: None,
        };
    }
    let parts =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    Split {
        left: parts[0],
        right: Some(parts[1]),
    }
}

/// The area inside a pane's border. The pty is sized from this and drawn into
/// this; that is the whole point of it being one function.
pub fn inner(pane: Rect) -> Rect {
    Block::bordered().inner(pane)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_is_the_area_inside_the_border() {
        let r = inner(Rect::new(0, 0, 80, 24));
        assert_eq!(r, Rect::new(1, 1, 78, 22));
    }

    #[test]
    fn the_split_covers_the_area_without_overlapping() {
        let area = Rect::new(0, 0, 120, 40);
        let s = split(area, false);
        let right = s.right.expect("wide enough to split");
        assert_eq!(s.left.x, 0);
        assert_eq!(right.x, s.left.x + s.left.width);
        assert_eq!(right.x + right.width, area.width);
    }

    #[test]
    fn a_narrow_window_collapses_instead_of_squeezing_claude() {
        let s = split(Rect::new(0, 0, MIN_SPLIT_COLS - 1, 24), false);
        assert!(s.right.is_none());
        assert_eq!(s.left.width, MIN_SPLIT_COLS - 1);

        // ...and zoom does the same thing deliberately.
        let s = split(Rect::new(0, 0, 200, 24), true);
        assert!(s.right.is_none());
        assert_eq!(s.left.width, 200);
    }
}
