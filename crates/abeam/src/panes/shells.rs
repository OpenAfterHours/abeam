//! The shell tabs belonging to one workspace.
//!
//! [`ShellPane`] deliberately owns exactly one pty. This collection is the
//! small piece above it that gives the application more than one shell without
//! teaching a terminal pane about tabs, identity, or its neighbours. Shells
//! have stable ids so an action confirmed in one frame, or a command handed
//! over from another pane, cannot drift to a different child after selection
//! changes.

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::pane::{Handled, Pane};

use super::ShellPane;

/// The identity of a shell within this abeam process.
///
/// Positions are deliberately not identities: closing shell 1 moves shell 2
/// into its slot, but does not make an outstanding action for shell 1 apply to
/// it. Values are never reused during the lifetime of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShellId(u64);

/// The outcome of trying to hand a command to a particular shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellCommand {
    /// The text was typed at the prompt, without being submitted.
    Sent,
    /// The shell exists but is still cold, or its live child has not enabled
    /// the safe bracketed-paste mode yet. A caller may retry this briefly.
    Pending,
    /// The id is stale, or its shell was drawn and could not start or exited.
    /// Retrying cannot make this particular destination usable again.
    Unavailable,
}

/// Process-wide rather than per collection because workspaces are themselves
/// transient. A workspace can disappear and later be rediscovered at the same
/// root while an action still carries an old [`ShellId`]; reusing `1` in its
/// replacement would make that stale action valid again.
static NEXT_SHELL_ID: AtomicU64 = AtomicU64::new(1);

impl fmt::Display for ShellId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

struct Shell {
    id: ShellId,
    pane: ShellPane,
}

/// Zero or more independent shells for one workspace, one of them selected.
pub struct ShellSessions {
    root: PathBuf,
    program: Option<String>,
    shells: Vec<Shell>,
    active: Option<ShellId>,
}

impl ShellSessions {
    /// Make a workspace's shell collection.
    ///
    /// The first shell is present but cold, preserving the old lazy-spawn
    /// behaviour: a workspace that never shows this pane never starts a child.
    pub fn new(root: PathBuf, program: Option<String>) -> Self {
        let mut sessions = Self {
            root,
            program,
            shells: Vec::new(),
            active: None,
        };
        sessions.create();
        sessions
    }

    /// Start a fresh, cold shell and select it.
    ///
    /// As with the initial shell, the process itself starts only when the pane
    /// is rendered. Returning the id lets callers target later work at the
    /// shell they created rather than whichever shell happens to be selected.
    pub fn create(&mut self) -> ShellId {
        // Relaxed is enough: allocation orders no other memory, it only has to
        // answer differently to every caller. `fetch_update` keeps the counter
        // from wrapping and making an ancient id valid again.
        let id = ShellId(
            NEXT_SHELL_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("this process exhausted every shell id"),
        );
        self.shells.push(Shell {
            id,
            pane: ShellPane::new(self.root.clone(), self.program.clone()),
        });
        self.active = Some(id);
        id
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.shells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shells.is_empty()
    }

    pub fn active_id(&self) -> Option<ShellId> {
        self.active
    }

    /// Select this shell, if it still exists. Returns whether selection moved.
    #[cfg(test)]
    pub fn activate(&mut self, id: ShellId) -> bool {
        if self.active == Some(id) || !self.shells.iter().any(|shell| shell.id == id) {
            return false;
        }
        self.active = Some(id);
        true
    }

    /// Select the preceding shell, wrapping at the beginning.
    pub fn select_previous(&mut self) -> bool {
        self.select_offset(-1)
    }

    /// Select the following shell, wrapping at the end.
    pub fn select_next(&mut self) -> bool {
        self.select_offset(1)
    }

    /// Close exactly `id` and drop its pty, which tears down its process tree.
    ///
    /// When the selected shell closes, its right-hand neighbour is selected;
    /// if it had none, selection moves left. Closing the last shell deliberately
    /// leaves the collection empty -- rendering it must not create a new one.
    pub fn close(&mut self, id: ShellId) -> bool {
        let Some(index) = self.index_of(id) else {
            return false;
        };
        let was_active = self.active == Some(id);
        self.shells.remove(index);

        if self.shells.is_empty() {
            self.active = None;
        } else if was_active {
            let neighbour = index.min(self.shells.len() - 1);
            self.active = Some(self.shells[neighbour].id);
        }
        true
    }

    /// Whether any shell in this workspace still has a live child.
    ///
    /// This is intentionally collection-wide: quit protection and worktree
    /// retention must not overlook a child just because its tab is hidden.
    pub fn is_live(&self) -> bool {
        self.shells.iter().any(|shell| shell.pane.is_live())
    }

    /// Whether the selected shell, rather than any hidden shell, is live.
    pub fn active_is_live(&self) -> bool {
        self.active_shell().is_some_and(ShellPane::is_live)
    }

    /// Whether the selected shell has not attempted to start a child yet.
    ///
    /// The app uses this while a close confirmation is visible so asking to
    /// discard an unused shell does not run its profile and create the very
    /// process the user is trying not to start.
    pub fn active_is_cold(&self) -> bool {
        self.active_shell().is_some_and(ShellPane::is_cold)
    }

    /// Type a command into exactly `id`, without submitting it.
    pub fn send_command(&mut self, id: ShellId, text: &str) -> ShellCommand {
        let Some(index) = self.index_of(id) else {
            return ShellCommand::Unavailable;
        };
        let shell = &mut self.shells[index];
        if shell.pane.send_command(text) {
            ShellCommand::Sent
        } else if shell.pane.is_cold() || shell.pane.is_live() {
            ShellCommand::Pending
        } else {
            ShellCommand::Unavailable
        }
    }

    fn index_of(&self, id: ShellId) -> Option<usize> {
        self.shells.iter().position(|shell| shell.id == id)
    }

    fn active_index(&self) -> Option<usize> {
        self.active.and_then(|id| self.index_of(id))
    }

    fn active_shell(&self) -> Option<&ShellPane> {
        let index = self.active_index()?;
        Some(&self.shells[index].pane)
    }

    fn active_shell_mut(&mut self) -> Option<&mut ShellPane> {
        let index = self.active_index()?;
        Some(&mut self.shells[index].pane)
    }

    fn select_offset(&mut self, offset: isize) -> bool {
        let len = self.shells.len();
        if len < 2 {
            return false;
        }
        let Some(index) = self.active_index() else {
            return false;
        };
        let next = (index as isize + offset).rem_euclid(len as isize) as usize;
        self.active = Some(self.shells[next].id);
        true
    }
}

impl Pane for ShellSessions {
    fn title(&self) -> String {
        let Some(shell) = self.active_shell() else {
            return "no shell".to_string();
        };
        let title = shell.title();
        if self.shells.len() == 1 {
            return title;
        }

        let position = self.active_index().expect("an active shell has a position") + 1;
        // Keep the useful index near the front where a narrow border will not
        // clip it. The ordinary live/cold title already starts with `shell`.
        if let Some(rest) = title.strip_prefix("shell") {
            format!("shell {position}/{}{rest}", self.shells.len())
        } else {
            format!("shell {position}/{} · {title}", self.shells.len())
        }
    }

    fn render(&mut self, f: &mut Frame, inner: Rect) {
        if let Some(index) = self.active_index() {
            self.shells[index].pane.render(f, inner);
            return;
        }
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        f.render_widget(
            Paragraph::new(vec![
                Line::from("No shell is open."),
                Line::from(""),
                Line::from("Use F1, S or F1, C to start a fresh shell."),
            ]),
            inner,
        );
    }

    fn tick(&mut self) -> bool {
        let active = self.active;
        let mut active_dirty = false;
        for shell in &mut self.shells {
            // Hidden shells still need polling so their output is drained and
            // exited children are reaped. Their dirtiness cannot change the
            // frame currently on screen, though, so only surface the selected
            // shell's redraw request.
            let dirty = shell.pane.tick();
            if Some(shell.id) == active {
                active_dirty |= dirty;
            }
        }
        active_dirty
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Handled> {
        match self.active_shell_mut() {
            Some(shell) => shell.handle_key(key),
            None => Ok(Handled::No),
        }
    }

    fn handle_mouse(&mut self, ev: &MouseEvent) -> Result<Handled> {
        match self.active_shell_mut() {
            Some(shell) => shell.handle_mouse(ev),
            None => Ok(Handled::No),
        }
    }

    fn scroll_key(&mut self, key: KeyEvent) -> Result<Handled> {
        match self.active_shell_mut() {
            Some(shell) => shell.scroll_key(key),
            None => Ok(Handled::No),
        }
    }

    fn takes_input(&self) -> bool {
        self.active_shell().is_some_and(Pane::takes_input)
    }

    fn exit_hint(&self) -> &'static str {
        self.active_shell().map_or("esc→agent", Pane::exit_hint)
    }

    fn action_hint(&self) -> Option<&'static str> {
        self.active_shell().and_then(Pane::action_hint)
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.active_shell().and_then(Pane::cursor)
    }

    fn on_resize(&mut self, inner: Rect) -> Result<()> {
        match self.active_shell_mut() {
            Some(shell) => shell.on_resize(inner),
            None => Ok(()),
        }
    }

    fn selected_text(&self, first: u16, last: u16) -> Option<String> {
        self.active_shell()
            .and_then(|shell| shell.selected_text(first, last))
    }

    fn handle_paste(&mut self, text: &str) -> Result<Handled> {
        match self.active_shell_mut() {
            Some(shell) => shell.handle_paste(text),
            None => Ok(Handled::No),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sessions() -> ShellSessions {
        ShellSessions::new(PathBuf::from("."), Some("abeam-no-such-shell".to_string()))
    }

    fn draw(sessions: &mut ShellSessions, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| sessions.render(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn starts_with_one_cold_shell_and_new_ids_are_never_reused() {
        let mut sessions = sessions();
        let first = sessions.active_id().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions.title(), "shell");
        assert!(sessions.active_is_cold());

        let second = sessions.create();
        assert_ne!(first, second);
        assert_eq!(sessions.active_id(), Some(second));
        assert_eq!(sessions.title(), "shell 2/2");

        assert!(sessions.close(second));
        let third = sessions.create();
        assert_ne!(first, third);
        assert_ne!(second, third);
        assert_eq!(sessions.title(), "shell 2/2");
    }

    #[test]
    fn ids_are_not_reused_by_another_collection_at_the_same_root() {
        let first = sessions().active_id().unwrap();
        let second = sessions().active_id().unwrap();
        assert_ne!(
            first, second,
            "recreating a workspace must not validate an action for its old shell"
        );
    }

    #[test]
    fn navigation_wraps_and_activation_is_by_stable_id() {
        let mut sessions = sessions();
        let first = sessions.active_id().unwrap();
        let second = sessions.create();
        let third = sessions.create();

        assert!(sessions.select_next());
        assert_eq!(sessions.active_id(), Some(first), "next wraps at the end");
        assert!(sessions.select_previous());
        assert_eq!(
            sessions.active_id(),
            Some(third),
            "previous wraps at the start"
        );
        assert!(sessions.activate(second));
        assert_eq!(sessions.active_id(), Some(second));
        assert!(!sessions.activate(second), "already selected is not a move");
        assert!(
            !sessions.activate(ShellId(u64::MAX)),
            "a stale id is refused"
        );
    }

    #[test]
    fn closing_the_active_shell_prefers_its_right_neighbour_then_its_left() {
        let mut sessions = sessions();
        let first = sessions.active_id().unwrap();
        let second = sessions.create();
        let third = sessions.create();

        sessions.activate(second);
        assert!(sessions.close(second));
        assert_eq!(sessions.active_id(), Some(third));
        assert_eq!(sessions.title(), "shell 2/2");

        assert!(sessions.close(third));
        assert_eq!(sessions.active_id(), Some(first));
        assert_eq!(sessions.title(), "shell");
        assert!(
            !sessions.close(second),
            "closing a stale id changes nothing"
        );
    }

    #[test]
    fn closing_an_inactive_shell_does_not_redirect_the_active_one() {
        let mut sessions = sessions();
        let first = sessions.active_id().unwrap();
        let second = sessions.create();
        let third = sessions.create();
        sessions.activate(second);

        assert!(sessions.close(first));
        assert_eq!(sessions.active_id(), Some(second));
        assert_eq!(sessions.title(), "shell 1/2");
        assert!(sessions.close(third));
        assert_eq!(sessions.active_id(), Some(second));
    }

    #[test]
    fn the_last_close_stays_empty_even_when_rendered() {
        let mut sessions = sessions();
        let only = sessions.active_id().unwrap();
        assert!(sessions.close(only));
        assert!(sessions.is_empty());
        assert_eq!(sessions.active_id(), None);
        assert_eq!(sessions.title(), "no shell");

        let screen = draw(&mut sessions, 46, 8);
        assert!(screen.contains("No shell is open."), "got: {screen}");
        assert!(screen.contains("F1, S or F1, C"), "got: {screen}");
        assert!(sessions.is_empty(), "drawing recreated the shell");
        assert!(!sessions.tick());
        assert!(!sessions.takes_input());
        assert_eq!(sessions.cursor(), None);
        assert_eq!(sessions.exit_hint(), "esc→agent");
    }

    #[test]
    fn events_and_commands_are_safely_declined_while_empty_or_cold() {
        let mut sessions = sessions();
        let cold = sessions.active_id().unwrap();
        assert_eq!(
            sessions.send_command(cold, "git status"),
            ShellCommand::Pending
        );
        assert_eq!(
            sessions.send_command(ShellId(u64::MAX), "git status"),
            ShellCommand::Unavailable
        );

        sessions.close(cold);
        let replacement = sessions.create();
        assert_ne!(cold, replacement);
        assert_eq!(
            sessions.send_command(cold, "git status"),
            ShellCommand::Unavailable,
            "a command for the closed shell was retargeted to its replacement"
        );
        sessions.close(replacement);
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(sessions.handle_key(key).unwrap(), Handled::No);
        assert_eq!(sessions.handle_paste("text").unwrap(), Handled::No);
        assert_eq!(sessions.selected_text(0, 1), None);
        sessions.on_resize(Rect::new(0, 0, 40, 8)).unwrap();
    }

    #[test]
    fn a_drawn_shell_that_failed_to_start_is_unavailable_not_pending() {
        let mut sessions = sessions();
        let id = sessions.active_id().unwrap();
        draw(&mut sessions, 46, 8);

        assert!(!sessions.active_is_cold());
        assert_eq!(sessions.title(), "no shell · enter retries");
        assert_eq!(
            sessions.send_command(id, "git status"),
            ShellCommand::Unavailable,
            "a failed destination would otherwise be retried until the hand-off deadline"
        );
    }
}
