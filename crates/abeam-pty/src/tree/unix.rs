//! The process group the child already leads, so that killing a hosted shell
//! kills what the shell started.
//!
//! `SIGKILL` — which is where `portable_pty`'s `Child::kill` ends up, and the
//! most one process can do to another — ends one process. It does not end the
//! `cargo build` that process launched, and once the parent has gone there is
//! nothing left to walk: the orphan belongs to `init` now, and `init` will not
//! tell us it was ever ours.
//!
//! `killpg` is the operating system's answer. It signals every process in a
//! process group in one call, and the group is the one relationship that
//! survives the parent, because it is not a relationship between processes at
//! all — it is a number they each carry.
//!
//! # Why the child's pid is the number to signal
//!
//! Because the child calls `setsid()` before it execs anything. That is
//! portable-pty's doing, in the `pre_exec` hook in `src/unix.rs` — line 257 of
//! the 0.9.0 this was read against. It makes the child a session leader, and a
//! session leader is by definition the leader of a new process group whose id
//! is its own pid. Everything the child starts inherits that group. So the pid
//! stored here *is* a pgid, and there is no call to make to find that out.
//!
//! Which is a fact about a dependency, not about Unix, and it is the one to
//! check first if this file ever stops working. Were portable-pty to drop that
//! `setsid` — or make it conditional, as it already does the `TIOCSCTTY` call
//! ten lines below it — the child would stay in abeam's own group and its pid
//! would name no group at all. This file would then go from sweeping the tree
//! to sweeping nothing, and eventually, once that number came round again as
//! somebody else's group, to sweeping the wrong one. Nothing here could tell:
//! `killpg` answers `ESRCH` for a group that is not there and success for one
//! that is, and neither answer says whose it was. What would tell is
//! `a_dropped_session_does_not_leave_the_childs_children_running` in
//! `tests/session.rs`, which is why that test asserts on a real grandchild and
//! not on anything this file returns.
//!
//! Deliberately *not* `MasterPty::process_group_leader()`, which portable-pty
//! does offer and which looks like exactly this. It is `tcgetpgrp` on the
//! master, and it answers a different question: which group has the terminal in
//! the *foreground* at this instant. Under a job-control shell that is whatever
//! is running right now and the shell's own group a moment later — a number
//! that changes while you are reading it is not a number to kill by.
//!
//! # Signalling a group whose number has been reused
//!
//! A pgid is a pid and pids come round again. Sending `SIGKILL` to a group that
//! now belongs to a stranger is the worst thing this file could do, so it is
//! worth being exact about what stops it.
//!
//! A pid is not released while anything still refers to it, and membership of a
//! process group is such a reference — including membership by a leader that has
//! exited and not yet been reaped. So at the moment [`Group::drop`] runs there
//! are two cases and no third. Either something in the group is still alive, in
//! which case the number is still ours and still names the group we mean; or
//! nothing is, in which case there was nothing here to kill and the sweep is a
//! signal into an empty group.
//!
//! The exposure is that second case, and the window is not the instant it looks
//! like. Reaping is what releases the number, and `PtySession::try_wait` is a
//! public method a host calls every frame, so the leader is reaped the moment it
//! exits rather than at teardown; after that only its descendants hold the group
//! up, and once they have gone too the number is free. What is long is the gap
//! between that and this line. `abeam::app` deliberately keeps abeam alive after
//! the hosted agent has exited for as long as a shell pane is live, because
//! leaving would kill the `cargo build` somebody has running in it. So the
//! sequence is: the agent exits, `try_wait` reaps it, the group empties — and
//! then abeam sits there for as long as the user leaves that pane open, which is
//! the pane the `cargo build` is in, spawning thousands of pids.
//!
//! To be wrong, then, everything in the group has to have gone *and* the pid
//! space has to have wrapped round far enough to reissue the number, before this
//! line runs. On the kernel's default `pid_max` of 32768 that is tens of
//! thousands of spawns inside a window whose length the user chooses, which is
//! reachable rather than academic; where a distribution has raised `pid_max` into
//! the millions it goes back out of reach. Nobody has hit it here and nothing
//! this file could do would close it — the number is the only handle Unix hands
//! back. The fix, if it ever bites, is a `pidfd`: a handle to the process that
//! stays valid and unambiguous after it has been reaped, taken at spawn time. It
//! would have to be taken in portable-pty rather than here, and it is Linux-only,
//! which is why this port does not attempt it.
//!
//! It is also why `PtySession::drop` sweeps here *before* it lets `master` go,
//! rather than leaving both to the order the fields happen to be declared in.
//! Closing the master is what starts the kernel's own hangup, which is to say it
//! is the thing that empties this group; a sweep afterwards is a sweep of
//! whatever is left of it.
//!
//! # `SIGKILL`, and nothing more polite first
//!
//! For parity with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, which does not
//! negotiate either. The obvious alternative — `SIGTERM`, a moment to shut down,
//! then `SIGKILL` for whatever ignored it — is the right shape for a process
//! supervisor and the wrong shape here twice over: `Drop` has nowhere to wait,
//! and this runs on the way out of abeam, where the only thing a grace period
//! reliably buys is a slower exit. A build has nothing to save.
//!
//! # What this does not reach
//!
//! Anything that left the group. A descendant that calls `setsid` itself leads
//! a group of its own and this call cannot see it, and so does every job an
//! interactive shell starts, because job control puts each one in its own group
//! by design. `PtySession::drop` kills the child before it gets here for that
//! reason among others: the `SIGHUP` that goes first is what a job-control shell
//! answers by hanging up its own jobs, and that is the only route to them.

use portable_pty::Child;

pub struct Group(libc::pid_t);

impl Group {
    /// The group `child` leads, holding it and everything it goes on to start —
    /// the shape `crate::tree` exports, and the one thing anything outside this
    /// module calls.
    ///
    /// `None` if the child will not say what its pid is, or says one that is
    /// not a pid. Neither check is ceremony. `killpg` reads a `0` as *the
    /// calling process's own group* — abeam, every other session it is hosting,
    /// and whatever started abeam — so a number that is not positively a pid is
    /// not a number this file will signal.
    pub fn holding(child: &(dyn Child + Send + Sync)) -> Option<Group> {
        // `process_id` is a `u32` and `killpg` wants a `pid_t`. A pid that does
        // not fit is not a pid we have any business signalling.
        let pid = libc::pid_t::try_from(child.process_id()?).ok()?;
        (pid > 0).then_some(Group(pid))
    }
}

impl Drop for Group {
    /// This is the kill. Every process still carrying the child's group id goes
    /// here, which is the whole of what this file is for.
    fn drop(&mut self) {
        // Best effort, like everything in this module. The failures are `ESRCH`
        // (the group is already gone, so there was nothing to do) and `EPERM`
        // (someone in it changed uid, and no error return would have helped).
        unsafe { libc::killpg(self.0, libc::SIGKILL) };
    }
}
