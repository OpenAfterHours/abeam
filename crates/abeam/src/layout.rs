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
/// remaining 60% is actively bad for the agent. Collapsing is the right
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

/// The rows an agent needs *inside* its border before a pane of its own is
/// worth drawing at all.
///
/// **A number reached by adding up what the rows are spent on, which is the
/// only honest way to argue one short of measuring it.** Every agent abeam
/// hosts draws the same furniture and it is not small: the composer is a
/// three-row box, the hint line under it is a fourth row, and the status or
/// spinner line above it is a fifth. Those five are there whatever the agent
/// is doing. Twelve leaves seven rows of transcript — enough to show a
/// permission prompt's question and its options, which is the one thing that
/// must never be off screen, because an agent stopped on a question nobody can
/// see is the exact stall `crate::agentstate` exists to keep off the queue's
/// path.
///
/// Below that the pane is mostly somebody else's border. At ten inside, the
/// transcript is four rows; at eight it is two — and two rows of an agent's
/// work under three rows of its own composer is worth *less* than the single
/// row this stack collapses to, because that row still says which agent it is
/// and whether it is working and does not pretend to be a window on anything.
///
/// **The arithmetic this implies, said out loud, because it decides who ever
/// sees two agents at once.** A whole pane is this plus its border, so two of
/// them want 28 rows and three want 42. A 24-row terminal — the default on a
/// great many machines — therefore never draws two agents whole, whatever the
/// user does: it draws one and a title row. That is the floor working rather
/// than failing, and it is the number to argue with if the feature feels
/// smaller than it sounded.
///
/// **What would make it wrong is a measurement nobody has made.** It is
/// reasoned from the layout these agents draw rather than timed against them,
/// and the number that would settle it is the tallest permission dialog any
/// hosted agent puts up. If one of them wants fourteen, this constant is where
/// that goes: the rest of [`stack`] is written in terms of it and knows no
/// other number.
///
/// It is also not true of *every* agent, which the paragraph above overstated.
/// `abeam +pwsh` hosts a shell: no composer, no hint line, and a prompt that is
/// useful in four rows. The number is set by the agents this program exists for
/// rather than by everything it can host, and a shell in a collapsed pane loses
/// less than a permission dialog would.
pub const MIN_AGENT_ROWS: u16 = 12;

/// Where the agents go down the left column: one rect each, in list order.
///
/// **Vertical, because the other axis has nothing to give.** At 120 columns
/// [`split`] leaves 72 on the left, and two agents abreast is 36 each — below
/// what any of these agents can draw. A 40-row window gives two agents about
/// nineteen rows apiece, which is a pane. Rows are the cheaper axis.
///
/// **A third function in this module, under this module's own rule.** Each pty
/// is sized from the rect that drew it, so a stack worked out a second time on
/// the way to a resize is a resize that disagrees with the frame — which is
/// "off-by-one here is what makes hosted apps wrap strangely" one pane along.
/// There is one calculation and [`crate::app::App::ui`] calls it once.
///
/// **The floor is [`MIN_AGENT_ROWS`], and it is [`MIN_SPLIT_COLS`]'s rule on
/// the other axis: collapsing is the right degradation; squeezing is not.**
/// Below it the stack stops expanding and starts collapsing — a pane that
/// cannot be drawn whole shrinks to its **title row** instead of disappearing.
/// One row per agent keeps the roster and the busy signal on screen, which is
/// most of what watching several agents means, and it degrades a row at a time
/// as `n` grows rather than falling off a cliff when the window runs out.
///
/// **`at` is an input and the proposal's sketch did not have one, which is the
/// single correction this makes to it.** Which panes collapse cannot be decided
/// from `n` alone: the pane holding the keys has to be one that is drawn, or
/// the reader is typing into a title row with no cursor and no screen. So the
/// rule is *the pane with the keys first, then list order from the top* — which
/// also keeps `agents[0]` on screen wherever there is room for two, and
/// `agents[0]` is the border the session's own facts and the queue's countdown
/// are drawn on.
///
/// **The floor that rule holds at, exactly, because the obvious statement of it
/// is false.** `inner(rects[at])` has at least one row **iff
/// `area.height >= n + 2`** — a title row each, and two more for the focused
/// pane's own border. Below that *no* pane has an inside: the window is shorter
/// than the number of agents plus their chrome, and there is nothing left to
/// degrade to. At or above it the focused pane is the only one that can have an
/// inside when just one can, and it has [`MIN_AGENT_ROWS`] of it as soon as the
/// column will carry a whole pane at all.
///
/// **What that costs at the very bottom is worth naming rather than implying.**
/// Below `n + 2` rows, `crate::app::App::ui` finds no rect to put the cursor in
/// and draws none, while keystrokes still reach the focused pane's pty at
/// whatever size it was last drawn at — typing into a child with nothing on
/// screen. A three-row window with two agents in it is the whole of the
/// reachable case, and the honest answer to it is a taller window; what would
/// be dishonest is a doc claiming the focused pane is always drawn.
///
/// **Panes never swap places, at any height, and that is a property worth
/// having on purpose.** The rects come out in list order and only their heights
/// change, so growing or shrinking the window, or moving the cursor, never
/// moves a pane past another one: what was second from the top stays second
/// from the top. A stack that reordered itself to keep the focused pane whole
/// would be the "pane that yanks itself into view" `app.rs` opens by refusing,
/// one axis along.
///
/// The cost of that is real and is not the collapsing itself. With three agents
/// and room for two whole panes, `agents[0]` and the focused pane are the two,
/// so **any two later agents can never be read side by side** — moving the
/// cursor to `agents[2]` collapses `agents[1]` on the way past. Comparing two
/// panes neither of which is the session's is a thing somebody will want, and
/// this layout cannot express it; what it would take is a second cursor, or a
/// rule that pins panes rather than picking them, and either is a feature
/// rather than a tweak.
///
/// The rects tile `area` exactly — no overlap, no gap, nothing outside it — at
/// every height, including the ones too short to give every pane even a row.
///
/// **Each pane pays for a whole border, so two adjacent panes spend two rows
/// on the line between them.** Sharing that line would mean panes with three
/// sides, which is a second idea of what a border is and therefore a second
/// [`inner`] — the one thing this module exists to prevent. One row per
/// boundary, bought back if anything ever needs it badly enough to earn a
/// second inset function, and not before.
pub fn stack(area: Rect, n: usize, at: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    // Clamped rather than trusted. `App::at_agent` is kept inside the vector by
    // `App::set_agent`, but this function is pure and unit-tested precisely so
    // it can be asked things the app never asks, and an index past the end
    // would panic in the indexing below rather than draw something.
    let at = at.min(n - 1);
    let rows = u32::from(area.height);
    let panes = u32::try_from(n).unwrap_or(u32::MAX);

    // How many can be drawn whole: the largest number the rows will carry, and
    // one at minimum — the pane with the keys, which is what the fully
    // collapsed case degrades to.
    //
    // The test is put to [`inner`] rather than written as `MIN_AGENT_ROWS + 2`,
    // because what a border costs in rows is that function's answer and this
    // module exists so that nobody works it out a second time.
    let full = (1..=n)
        .rev()
        .find(|&k| {
            let taken = u32::try_from(k).unwrap_or(u32::MAX);
            let each = rows.saturating_sub(panes - taken) / taken;
            let each = u16::try_from(each).unwrap_or(u16::MAX);
            inner(Rect::new(0, 0, 1, each)).height >= MIN_AGENT_ROWS
        })
        .unwrap_or(1);

    // The pane with the keys is in the set before anything else can be; the
    // rest join it from the top of the list.
    let mut whole = vec![false; n];
    whole[at] = true;
    let mut room = full - 1;
    for pane in whole.iter_mut() {
        if room == 0 {
            break;
        }
        if !*pane {
            *pane = true;
            room -= 1;
        }
    }

    let mut heights = vec![0u32; n];
    if rows < panes {
        // Fewer rows than agents — a two-row window with three of them in it.
        // There is no degradation left below one row each, so the only decision
        // worth making is who is last to go, and it is the same order: the pane
        // with the keys keeps its row, then the list from the top.
        let mut left = rows;
        if left > 0 {
            heights[at] = 1;
            left -= 1;
        }
        for (ix, height) in heights.iter_mut().enumerate() {
            if left == 0 {
                break;
            }
            if ix != at {
                *height = 1;
                left -= 1;
            }
        }
    } else {
        // A title row each, and everything above that shared out among the
        // panes being drawn whole. The remainder is handed out a row at a time
        // to the earliest of them, which is what makes these tile the area
        // exactly instead of leaving a row of the window unpainted.
        for height in heights.iter_mut() {
            *height = 1;
        }
        let spare = rows - panes;
        let taken = u32::try_from(full).unwrap_or(1);
        let each = spare / taken;
        let mut extra = spare % taken;
        for (ix, height) in heights.iter_mut().enumerate() {
            if whole[ix] {
                *height += each + u32::from(extra > 0);
                extra = extra.saturating_sub(1);
            }
        }
    }

    let mut out = Vec::with_capacity(n);
    let mut y = area.y;
    for height in heights {
        let height = u16::try_from(height).unwrap_or(u16::MAX);
        out.push(Rect::new(area.x, y, area.width, height));
        y += height;
    }
    out
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
    fn a_narrow_window_collapses_instead_of_squeezing_the_agent() {
        let s = split(Rect::new(0, 0, MIN_SPLIT_COLS - 1, 24), false);
        assert!(s.right.is_none());
        assert_eq!(s.left.width, MIN_SPLIT_COLS - 1);

        // ...and zoom does the same thing deliberately.
        let s = split(Rect::new(0, 0, 200, 24), true);
        assert!(s.right.is_none());
        assert_eq!(s.left.width, 200);
    }

    /// Every row of the column belongs to exactly one pane.
    ///
    /// **The assertion `split`'s own test makes, over a whole list rather than
    /// a pair**, and it is worth more here for a reason that is not tidiness: a
    /// gap is a row of the previous frame nobody paints over, and an overlap is
    /// two ptys told they own the same rows. Both are silent. Swept over
    /// heights either side of the floor and over agent counts either side of
    /// what the window will carry, because the arithmetic that can leave a row
    /// behind is the remainder, and the remainder only exists when the rows do
    /// not divide.
    #[test]
    fn the_stack_covers_the_column_without_overlapping_or_leaving_gaps() {
        for height in [0, 1, 2, 3, 5, 13, 14, 15, 24, 27, 28, 40, 41, 60, 97] {
            for n in 1..=6usize {
                for at in 0..n {
                    let area = Rect::new(7, 3, 72, height);
                    let rects = stack(area, n, at);
                    assert_eq!(rects.len(), n, "one rect per agent");

                    let mut y = area.y;
                    for rect in &rects {
                        assert_eq!(rect.x, area.x, "{height}/{n}/{at}: {rect:?}");
                        assert_eq!(rect.width, area.width, "{height}/{n}/{at}: {rect:?}");
                        assert_eq!(
                            rect.y, y,
                            "{height}/{n}/{at}: a gap or an overlap at {rect:?}"
                        );
                        y += rect.height;
                    }
                    assert_eq!(
                        y,
                        area.y + area.height,
                        "{height}/{n}/{at}: the stack does not reach the bottom \
                         of the column"
                    );
                }
            }
        }
    }

    /// Below the floor the stack collapses panes rather than squeezing them
    /// all.
    ///
    /// `MIN_SPLIT_COLS`' rule on the other axis, and the same shape of test:
    /// three agents in a window that has room for one of them whole, and the
    /// other two are a title row each rather than three unreadable thirds.
    #[test]
    fn a_short_window_collapses_agents_instead_of_squeezing_them() {
        // Two whole panes would want `2 * (MIN_AGENT_ROWS + 2)` between them,
        // plus a row for the third; one short of that is the last height at
        // which only one can be drawn.
        let height = 2 * (MIN_AGENT_ROWS + 2) + 1 - 1;
        let rects = stack(Rect::new(0, 0, 72, height), 3, 0);

        assert!(
            inner(rects[0]).height >= MIN_AGENT_ROWS,
            "the pane with the keys was squeezed: {rects:?}"
        );
        assert_eq!(rects[1].height, 1, "not collapsed to its title row");
        assert_eq!(rects[2].height, 1, "not collapsed to its title row");

        // And one row more is what buys the second whole pane, which is what
        // makes the floor a floor rather than a coincidence.
        let rects = stack(Rect::new(0, 0, 72, height + 1), 3, 0);
        assert!(inner(rects[0]).height >= MIN_AGENT_ROWS, "{rects:?}");
        assert!(inner(rects[1]).height >= MIN_AGENT_ROWS, "{rects:?}");
        assert_eq!(rects[2].height, 1, "{rects:?}");
    }

    /// The pane with the keys is the one that is drawn — down to the exact
    /// height at which nothing can be.
    ///
    /// The reason `at` is an argument at all. A collapsed pane is a title row:
    /// no cursor, no screen, and typing into it goes somewhere the typist
    /// cannot see.
    ///
    /// **Swept from zero rather than asserted at one comfortable height, which
    /// is what the first version of this did and what let a false claim stand.**
    /// "The focused pane is always drawn" is not true and cannot be: at `n + 1`
    /// rows there is a title row each and one over, and two rows have no inside.
    /// What is true is the floor in [`stack`]'s doc — an inside iff
    /// `height >= n + 2` — and the only way to find out that the obvious
    /// statement was wrong is to ask every height including the ones nobody
    /// would choose.
    #[test]
    fn the_agent_with_the_keys_is_the_one_that_is_drawn() {
        for n in 1..=6usize {
            for at in 0..n {
                let floor = u16::try_from(n).expect("a small n") + 2;
                for height in 0..=2 * floor {
                    let rects = stack(Rect::new(0, 0, 72, height), n, at);
                    let focused = inner(rects[at]).height;

                    if height < floor {
                        // Nothing has an inside, the focused pane included.
                        // This is the degradation `stack`'s doc names, not a
                        // case where somebody else got the rows.
                        assert!(
                            rects.iter().all(|rect| inner(*rect).height == 0),
                            "{height}/{n}/{at}: a pane has an inside below the \
                             floor: {rects:?}"
                        );
                        // ...and the focused pane is still the *last* to lose
                        // its title row. Below `n` rows somebody gets nothing
                        // at all, and it is never the pane with the keys: a
                        // stack that drew every pane but that one would be a
                        // window with no evidence in it that the agent taking
                        // the keystrokes exists.
                        assert!(
                            height == 0 || rects[at].height >= 1,
                            "{height}/{n}/{at}: the pane with the keys is the \
                             one that vanished: {rects:?}"
                        );
                        continue;
                    }
                    assert!(
                        focused >= 1,
                        "{height}/{n}/{at}: the focused pane has no inside at \
                         or above the floor: {rects:?}"
                    );
                    // ...and nobody is drawn *whole* while the focused pane is
                    // a title row, which is the half that would let a reader
                    // type into nothing with a readable pane beside it.
                    //
                    // **Within one row, and the `+ 1` is the guarantee rather
                    // than slack in the test.** The remainder left over when
                    // the rows do not divide is handed to the earliest whole
                    // panes, so a focused pane late in the list is legitimately
                    // one row shorter than an earlier one — `n = 2, at = 1` at
                    // 29 rows gives `[15, 14]`, and neither of those is a bug.
                    // Asserting equality here would go red on that the moment
                    // somebody widened the sweep, which is a test failing on
                    // correct code and the most expensive kind.
                    for (ix, rect) in rects.iter().enumerate() {
                        assert!(
                            ix == at || inner(*rect).height <= focused + 1,
                            "{height}/{n}/{at}: pane {ix} is more than one row \
                             taller than the one with the keys: {rects:?}"
                        );
                    }
                }

                // And once the column will carry a whole pane, the focused one
                // is whole rather than merely non-empty.
                let tall = (MIN_AGENT_ROWS + 2) + u16::try_from(n).expect("a small n");
                let rects = stack(Rect::new(0, 0, 72, tall), n, at);
                assert!(
                    inner(rects[at]).height >= MIN_AGENT_ROWS,
                    "{n}/{at}: the focused pane was squeezed: {rects:?}"
                );
            }
        }
    }

    /// One agent gets the column it has always had.
    ///
    /// The session everybody is running today. If the stack takes a row off it
    /// for a pane that does not exist, every agent on every screen loses a row
    /// to a feature nobody asked for.
    #[test]
    fn one_agent_is_the_whole_column() {
        let area = Rect::new(0, 0, 72, 40);
        assert_eq!(stack(area, 1, 0), vec![area]);
    }
}
