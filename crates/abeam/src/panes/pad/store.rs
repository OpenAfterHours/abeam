//! Where the scratch pad's text goes while abeam is not running, and the first
//! file this program has ever written.
//!
//! Everything else abeam knows about a machine it reads. A config file, a
//! session record, `git status`, whatever the watcher reports — and
//! `crate::config` goes as far as compiling the TOML crate with its serializer
//! switched off, on the stated grounds that abeam reads that file and never
//! writes one. This module is the exception to a rule the whole crate has kept
//! so far, and what makes it worth being one is what the bytes are: sentences
//! somebody typed once, into a pane they opened because the agent was busy, and
//! that exist nowhere else. Every decision below is the careful reading of a
//! problem that would be over-thought if this were a cache.
//!
//! ## Where the file lives
//!
//! Windows: `%APPDATA%\abeam\scratch\<key>.md`. Unix:
//! `$XDG_DATA_HOME/abeam/scratch/<key>.md`, or
//! `$HOME/.local/share/abeam/scratch/<key>.md` when that variable is unset or
//! relative. `<key>` is [`crate::paths::workspace_key`], which is the workspace
//! root written down as a file name under the same rule that decides whether
//! two spellings are the same directory anywhere else in abeam.
//!
//! **Beside the user's profile and never in the repository**, which is
//! `crate::config`'s rule arriving from the other side. That module refuses to
//! *read* out of the workspace because a repository is the one directory in
//! this program somebody else gets to write to. This one refuses to *write*
//! into it for two reasons of its own, both of which a user would see within
//! seconds: a pad file under the workspace is an untracked file in the git pane
//! next door, so typing a note dirties `git status` on screen; and
//! `crate::app::route` queues every `*.md` under a workspace into the reader's
//! follow list, so the user's own typing would mark the reader's border unread
//! and swap the document out from under it. A note taken about the work is not
//! part of the work.
//!
//! **`XDG_DATA_HOME`, and not the `XDG_CONFIG_HOME` `crate::config` reads.**
//! The specification keeps those directories apart, and the difference is not
//! filing. `$XDG_CONFIG_HOME` holds what the user wrote for a program to read,
//! and a program that writes there is a program that will one day overwrite
//! somebody's hand-edited settings; `abeam.toml` belongs to that category and
//! this file does not, because nobody hand-edits a file named after sixteen hex
//! digits and abeam rewrites this one while it is being typed into.
//!
//! Of the two categories left it is data rather than state, and that is worth
//! an argument because state is the easier reflex: this file is written by the
//! program, is one per workspace, and is never opened by hand, which is what
//! most of `~/.local/state` holds. The specification's own test settles it. It
//! calls state the things that are not important *or portable* enough for the
//! data directory, and a pad fails only the second of those two — it is the one
//! thing in this whole program a person cannot regenerate. An agent's output
//! can be asked for again, a `git status` recomputed, a session record rebuilt;
//! a sentence somebody had at eleven o'clock while the agent was busy cannot,
//! and `~/.local/share` is where the backup tools people actually run are
//! pointed.
//!
//! The case against, kept because the next person deserves to see that this was
//! a judgement and not a default: the file is keyed by the absolute path of a
//! directory on *this* machine, so a pad restored onto another one lands
//! somewhere nothing will ever look for it. That is an argument about
//! portability, which is the weaker of the specification's two tests, and it is
//! an argument for making a restored pad findable rather than for not keeping
//! one. The fallback moves with the variable — `~/.local/share` rather than
//! `~/.config` — because a fallback that landed somewhere else would let the
//! variable's presence decide which category the file is in.
//!
//! Changing this later is not free whichever way it goes: every pad already on
//! disk stays exactly where it is, unreachable, while every pane opens empty.
//!
//! ## Saving cannot leave half a pad behind
//!
//! [`save_at`] writes a temporary file beside the real one, waits for the disk
//! to have it, and renames it over the top — so the pad is either last save's
//! text or this save's and never a prefix of one with the tail of the other.
//! Writing in place is one interrupted `write` away from a file that begins
//! with today's notes and ends with yesterday's, and the person that happens to
//! has no second copy, because the buffer it came from is in the process that
//! just died. The waiting is the half that is easy to leave out and that makes
//! the rest true: an unsynced rename can reach the disk before the bytes it
//! renames, which produces the empty pad the whole arrangement is against.
//!
//! ## Loading cannot fail, and says which kind of nothing it found
//!
//! [`load_at`] returns a [`Loaded`] and has no error case, because none of the
//! ways it can go wrong is a reason for abeam not to start. What it must never
//! do is answer them all with the same value, and it did: a file that is not
//! there, a file that is there and will not open, and a read that failed
//! part-way all came back as an empty pad.
//!
//! The belief that made that look safe is written out on [`Loaded::unreadable`]
//! along with the probe that broke it, because the reasoning was nearly right.
//! It ran: the pad opens empty, and if the file is really there then the next
//! *save* will fail too and say so. It does not. Reading a file needs read
//! access to it; renaming over one needs delete rights on it or its parent.
//! Those are different rights, and a file held open by a scanner for a fraction
//! of a second fails the first and passes the second — so the empty pad is
//! written over the notes, quietly, by an autosave nobody asked for.
//!
//! So there are three states now, and two of them stop this session saving at
//! all. The other is the cap: this may never hand back a document larger than
//! [`MAX_BYTES`], because a buffer over that cap is the one state the cap
//! exists to make impossible, so the read stops there and
//! [`Loaded::truncated`] says that it did. Both flags carry the same obligation
//! and it is one this module cannot enforce from where it sits — the pane must
//! refuse to save for the rest of the session. Showing somebody the first 64 KB
//! of their notes is survivable; writing that prefix back over the whole file
//! is not.
//!
//! ## Two windows, one pad
//!
//! Two terminals in one repository is an ordinary way to work, and two abeams
//! on one workspace share one pad file. Both read it once, both hold a buffer,
//! and each save writes the whole thing — so without something in the way they
//! overwrite each other for the rest of the session, with nothing on either
//! screen to say so. [`Stamp`] is that something: the length and modification
//! time as they were when the file was last read or written, re-asked
//! immediately before the rename and compared. It detects an ordinary accident
//! and is not a lock; see the type for why a stat that cannot be taken means
//! *go ahead*.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::buffer::MAX_BYTES;

/// abeam's own directory inside the profile root, which is shared with every
/// other program on the machine. The same name `crate::config` uses, because it
/// is the same program.
const DIR: &str = "abeam";

/// The pads' own directory inside that. Plural on purpose: there is one file
/// per workspace root and a person may have a dozen.
const SCRATCH: &str = "scratch";

/// The extension, which is a promise about the contents rather than decoration.
/// The pane renders this text as markdown, and a `.md` opens as markdown in
/// whatever the user reaches for when they want it outside abeam.
const EXT: &str = "md";

/// What names the profile on this platform, for the one message that has to
/// tell somebody why their pad has nowhere to go.
#[cfg(windows)]
const PROFILE: &str = "%APPDATA%";
#[cfg(unix)]
const PROFILE: &str = "$XDG_DATA_HOME and $HOME";

// ---------------------------------------------------------------------------
// where the file is
// ---------------------------------------------------------------------------

/// This workspace's pad, or `None` when this machine will not say where the
/// user's profile is.
///
/// **Asked once per pane and the answer kept**, which is why [`load_at`] and
/// [`save_at`] below take a path rather than the root this one starts from. A
/// path derived twice is a path that can be derived differently — `crate::paths`
/// is a whole module built on that one sentence, and the rule it holds this
/// answer to folds case and separators precisely so that two spellings of one
/// workspace cannot become two files. Deriving it in one place also puts a
/// `None` in the pane's hand at construction rather than at the first failed
/// save, which is the difference between telling somebody there is nowhere to
/// write before they type and telling them afterwards.
///
/// `%APPDATA%` and nothing behind it, for the reasons `crate::config::path`
/// gives at length: `USERPROFILE` would leave a bare `abeam` directory in
/// somebody's home, and `HOME` on Windows is git-bash's, which frequently names
/// a POSIX-shaped path no Windows program has ever written to. The difference
/// there was that abeam was choosing where to look for its own file; here it is
/// choosing where to *put* one, which is the same requirement with the stakes
/// the other way up.
#[cfg(windows)]
pub(super) fn path_for(root: &Path) -> Option<PathBuf> {
    Some(from_appdata(std::env::var_os("APPDATA").map(PathBuf::from))?.join(file(root)))
}

/// The Unix twin, over the two variables the XDG base directory specification
/// names for user data in the order it names them.
#[cfg(unix)]
pub(super) fn path_for(root: &Path) -> Option<PathBuf> {
    Some(
        from_xdg(
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        )?
        .join(file(root)),
    )
}

/// The name this root's pad has, wherever the directory above it turns out to
/// be.
fn file(root: &Path) -> String {
    format!("{}.{EXT}", crate::paths::workspace_key(root))
}

/// Windows' answer, over the variable handed in rather than read.
///
/// Split out for `crate::config::from_appdata`'s reason, which is that the
/// process environment belongs to the whole test binary: a test that set
/// `APPDATA` to prove this rule would be setting it for the three hundred and
/// fifty tests running beside it, several of which spawn children that inherit
/// it.
///
/// **A relative variable is refused rather than followed**, which is
/// `crate::config`'s rule unchanged and for exactly the same reason. Joining
/// onto a relative path leaves a relative path, so the write below stops being
/// a question about the user's profile and becomes one about wherever this
/// process happens to be standing — which `main` deliberately moves to
/// `%SystemRoot%` or `/`, and which before that line is the repository on
/// screen. An `APPDATA=.` left in a shell for some other program's benefit
/// would then drop an `abeam\scratch` directory into a clone, which is the one
/// place the module docs above spend a paragraph refusing to write to.
/// Absoluteness rather than mere blankness, because blank is only the loudest
/// way of being relative, and PowerShell leaves `$env:APPDATA = ""` behind when
/// somebody clears it.
///
/// Compiled on both platforms and gated only at its caller, so that a machine
/// of either kind can prove both rules. This is string arithmetic with no
/// filesystem in it, and the Unix rule is the one most likely to be broken by
/// somebody who cannot run it.
#[cfg_attr(
    unix,
    allow(dead_code, reason = "the other platform's rule, tested on both")
)]
fn from_appdata(appdata: Option<PathBuf>) -> Option<PathBuf> {
    Some(
        appdata
            .filter(|dir| dir.is_absolute())?
            .join(DIR)
            .join(SCRATCH),
    )
}

/// Unix's answer, over the two variables handed in rather than read.
///
/// `XDG_DATA_HOME` when it is set to something absolute, and `~/.local/share`
/// otherwise, which is the fallback the specification names rather than abeam's
/// own invention. Both are held to the absoluteness rule above, and the home
/// directory is the reason it is applied twice rather than once: a container or
/// a service unit can export an empty `HOME`, and `.local/share/abeam/scratch`
/// resolved against `/` is a directory belonging to nobody that root can write.
///
/// A **relative** `XDG_DATA_HOME` falls through to `HOME` rather than ending
/// the search, which is the one place this differs from simply refusing.
/// `crate::config::from_xdg` makes the whole argument and it carries over
/// without a word changed: the variable is discarded either way, so the only
/// question left is whether one bad variable costs the user their pad, and the
/// specification's own instruction is to consider a relative path invalid and
/// ignore it.
#[cfg_attr(
    windows,
    allow(dead_code, reason = "the other platform's rule, tested on both")
)]
fn from_xdg(data: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = match data.filter(|dir| dir.is_absolute()) {
        Some(data) => data,
        None => home
            .filter(|dir| dir.is_absolute())?
            .join(".local")
            .join("share"),
    };
    Some(base.join(DIR).join(SCRATCH))
}

// ---------------------------------------------------------------------------
// reading it
// ---------------------------------------------------------------------------

/// A pad as it came off the disk.
///
/// The same shape as `crate::panes::viewer::load::Loaded`, deliberately rather
/// than by coincidence: that module answered this exact question for documents
/// an agent chose, and two different shapes for "here is the text, and here is
/// why it is not all of the text" would be two things for a pane to get wrong.
///
/// [`Default`] is an empty pad, which is what a workspace nobody has written one
/// for has and equally what a machine that will not say where the profile is
/// has. Nothing above this line needs to tell those two apart while *reading* —
/// both open an empty pane — and the one place the difference matters is a save,
/// where it is the difference between a file and [`nowhere`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Loaded {
    /// What goes into the buffer, and never more than [`MAX_BYTES`] of it.
    pub text: String,
    /// There is something at that path and abeam could not read it, so
    /// [`text`](Self::text) is empty for a reason that is not "no pad yet".
    ///
    /// **A pane that sees this must not save for the rest of the session**,
    /// exactly as for [`truncated`](Self::truncated), and it is a field of its
    /// own only because the notice has to say a different thing. Writing here
    /// would replace a file whose contents abeam has never seen, with an empty
    /// pad or with whatever was typed after it opened.
    ///
    /// This was believed covered and was not, which is worth recording because
    /// the reasoning was nearly right. A *directory* where the file should be
    /// really does fail safely — the rename below cannot replace a directory,
    /// so the save fails loudly — and that is the case the tests had. The case
    /// they did not have is a file that is there and will not open, and it
    /// behaves the opposite way: reading needs read access to the file, while
    /// the rename needs delete rights on the target or its parent, and those
    /// are different rights. A handle opened with no sharing — what an
    /// antivirus scan, OneDrive, the search indexer or a backup agent holds for
    /// a fraction of a second — fails the read and passes the rename, and 61
    /// bytes of somebody's notes become 2. A redirected `%APPDATA%` on a domain
    /// machine, which [`save_at`] already has a paragraph about, makes that
    /// window wider rather than narrower.
    pub unreadable: bool,
    /// What the file looked like at the moment it was read, so that a later
    /// save can tell whether anything else has written to it since.
    pub stamp: Stamp,
    /// The file on disk was longer than [`MAX_BYTES`], and [`text`](Self::text)
    /// is the head of it cut at a line ending.
    ///
    /// **A pane that sees this must not save for the rest of the session**, and
    /// the contract is written here because this is where the fact is made
    /// while the pane is where it has to be honoured. A save writes the
    /// buffer's text over the file it was read from, so one save of a truncated
    /// pad deletes everything past the cut — somebody's notes, destroyed by the
    /// program they opened to read them, with the deletion looking from the
    /// outside exactly like an ordinary autosave. Drawing a notice and refusing
    /// costs them the feature for one session; the other way costs them the
    /// file.
    pub truncated: bool,
}

/// What a pad file looked like, for telling "nobody has touched it" from
/// "somebody has".
///
/// Two abeam windows on one workspace share one pad file — two terminals in one
/// repository is an ordinary way to work — and without this they overwrite each
/// other for ever with nothing on either screen to say so. The pane records one
/// of these when it reads and again after every save it makes; [`save_at`]
/// takes the last one and asks again immediately before the rename.
///
/// **This is detection of an ordinary accident and not a lock.** A window
/// remains between that last stat and the rename which nothing here closes, and
/// a real lock would mean a pad that cannot be written because some other
/// process died holding something. So [`Stamp::Unknown`] — a stat that could
/// not be taken at all — means *go ahead*: this must never be the reason a save
/// does not happen.
///
/// Length as well as time, because a modification time can be coarse — FAT's is
/// two seconds — and two saves inside one of those would otherwise look
/// identical.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Stamp {
    /// Nothing was at that path, which is every workspace nobody has written a
    /// pad for yet.
    #[default]
    Absent,
    /// It was there, this many bytes, last written then. The time is `None` on
    /// a platform that will not say, and the length still carries the question.
    Seen(u64, Option<SystemTime>),
    /// The question could not be asked. Never a refusal — see the type's docs.
    Unknown,
}

/// Whether the file has been written by somebody else since it was last seen.
///
/// The one place [`Stamp::Unknown`]'s meaning is spelled out in code, and it is
/// spelled out here rather than at the call site so that there is one answer.
fn moved(seen: Stamp, now: Stamp) -> bool {
    !matches!(seen, Stamp::Unknown) && !matches!(now, Stamp::Unknown) && seen != now
}

/// What is at `path` now, as far as one `metadata` call can say.
///
/// A `NotFound` is an answer rather than a failure: it says there is no file,
/// which is a fact this can be certain of and is exactly what a first save
/// expects to find. Every other error is [`Stamp::Unknown`].
fn stamp(path: &Path) -> Stamp {
    match std::fs::metadata(path) {
        Ok(meta) => Stamp::Seen(meta.len(), meta.modified().ok()),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Stamp::Absent,
        Err(_) => Stamp::Unknown,
    }
}

/// Whatever is in the pad at `path`, or nothing at all.
///
/// See the module docs for why there is no `Result` here, and
/// [`Loaded::truncated`] for the one thing a pane owes this function when the
/// file was too long. Called once, on the pane's first frame.
///
/// **A path rather than the root it came from**, which is [`path_for`]'s
/// paragraph and is what makes this function reachable at all from a test: the
/// pane derives its file once and hands the answer back here, so the suite can
/// name a directory it owns and nothing in it writes into the profile of the
/// person running it.
///
/// **A leading byte order mark is dropped here and line endings are not**,
/// which are two jobs that look like one. A mark is a fact about reading bytes
/// off a disk rather than a character in anybody's document — Notepad writes
/// one and says nothing — so it belongs to the module that does the reading,
/// and left in it is an invisible first character that the caret can sit in
/// front of and that stops `# heading` being a heading. Line endings are
/// already handled where they belong: `Buffer::from_text` normalises CRLF on
/// the way in, because what a line *is* is the buffer's subject and not this
/// one's. Do not add the second of those here as well.
///
/// **Lossy rather than strict.** A `read_to_string` refuses the whole file over
/// one byte that is not UTF-8, and what that byte most likely is on a pad is
/// somebody's editor having saved it as Latin-1, or a stray byte from a paste —
/// so the strict reading answers a file full of the user's notes with an empty
/// pane, and then the first save overwrites it. A curly quote that comes back
/// as a replacement character is one bad character in a sentence somebody can
/// still read. Read, and not fix: the first save writes the decoded text back,
/// so from then on the replacement character is what is in the file and the
/// byte it stood for is gone. The same is true of the line endings
/// `Buffer::from_text` folds — a pad last touched in Notepad comes back LF-only
/// and is rewritten that way — and of both it is worth being plain that this is
/// a one-way door rather than a display convenience. It is still the better of
/// the two failures, because the strict read shows an empty pane and then
/// overwrites the whole file.
///
/// **One byte past the cap rather than a `metadata` call**, which is
/// `crate::panes::viewer::load`'s trick and worth copying: the read itself is
/// what says whether there was more, so a size that went stale between the two
/// calls cannot decide it.
pub(super) fn load_at(path: &Path) -> Loaded {
    let unreadable = || Loaded {
        unreadable: true,
        stamp: Stamp::Unknown,
        ..Loaded::default()
    };

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        // The one silence, and the only one: there is no file, which is the
        // ordinary state of every workspace nobody has written a pad for.
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
        // Anything else is a file that is there and that abeam has not seen.
        // See `Loaded::unreadable` for the save this exists to stop.
        Err(_) => return unreadable(),
    };

    // Off the open handle rather than off the path, so that the length and the
    // time belong to the bytes read below and not to whatever the name pointed
    // at a moment earlier.
    let stamp = match file.metadata() {
        Ok(meta) => Stamp::Seen(meta.len(), meta.modified().ok()),
        Err(_) => Stamp::Unknown,
    };

    let mut bytes = Vec::new();
    if file
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return unreadable();
    }
    let over = bytes.len() > MAX_BYTES;

    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if let Some(rest) = text.strip_prefix('\u{feff}') {
        text = rest.to_string();
    }
    let cut = cap(&mut text);

    Loaded {
        text,
        truncated: over || cut,
        unreadable: false,
        stamp,
    }
}

/// Cut `text` back to [`MAX_BYTES`] at a line ending, saying whether it had to.
///
/// Applied to the decoded string and not to the bytes, which is the point of
/// its being a separate function: lossy decoding can make a file *grow*, since
/// every byte that is not UTF-8 becomes a three-byte replacement character. A
/// read that stopped exactly at the cap can therefore still arrive over it, and
/// a buffer over the cap is the state the cap exists to make impossible.
///
/// Backing off to the last complete line is `crate::panes::viewer::load`'s
/// argument again: a cut mid-word reads as corruption to the person looking at
/// it, and this text is about to be shown to the person who wrote it. A first
/// line longer than the whole cap has no line ending to back off to, so that
/// one is cut at the nearest character boundary instead.
fn cap(text: &mut String) -> bool {
    if text.len() <= MAX_BYTES {
        return false;
    }

    let mut end = MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(newline) = text[..end].rfind('\n') {
        end = newline + 1;
    }
    text.truncate(end);
    true
}

// ---------------------------------------------------------------------------
// writing it
// ---------------------------------------------------------------------------

/// Put this workspace's pad on disk, or say in a sentence why it is not there,
/// and the whole of the care this module exists for.
///
/// `Err` is a message for the pane to print, not a log line: the person reading
/// it is looking at text they typed that is not saved, so it names the file it
/// could not write and tells them the text is still in front of them. The one
/// failure with no file to name never arrives here at all — a pane whose
/// [`path_for`] answered `None` has nothing to call this with, and prints
/// [`nowhere`] in its place.
///
/// Write a temporary file in the same directory, get it onto the disk, then
/// rename it over the target. All three matter.
///
/// **The same directory**, because a rename across volumes is a copy and a
/// delete with a window in the middle, and `%APPDATA%` on a domain machine can
/// be a redirected network share.
///
/// **`sync_all` before the rename**, because a rename that reaches the disk
/// ahead of the bytes it renames is the zero-length pad this whole arrangement
/// exists to prevent — the metadata operation and the data are two different
/// journeys, and only a flush orders them. `fs::write` does not do this, which
/// made the paragraph above true of the program's *intent* and not of the
/// machine. What remains unclosed, and is named rather than papered over: the
/// directory entry itself is not synced, so a power cut in that last instant
/// can lose the rename. Losing the rename leaves the previous pad whole, which
/// is the direction to be wrong in.
///
/// **A rename**, because it is the one operation both platforms make
/// indivisible: after it the name points at the whole of the new file or the
/// whole of the old one, and there is no instant at which it points at half of
/// either. A `write` straight to the target is a truncate followed by a write,
/// and a machine that loses power between them leaves nothing.
///
/// **The temporary file is created 0600 on Unix rather than at the umask
/// default**, which is a decision and not a detail. These are somebody's
/// private notes; a default umask of 022 makes them world-readable, so a pad a
/// careful person had chmodded 600 would be opened up by abeam's own first
/// autosave. `rename(2)` carries the source's mode over, so creating the
/// temporary file narrow is what makes the pad narrow. It also overrides a
/// wider mode somebody chose deliberately, and between the two mistakes this is
/// the one that cannot leak anything.
///
/// **`fs::rename` replaces an existing file on Windows**, which is worth
/// writing down because the folklore says otherwise. The folklore is about
/// `MoveFileW` and the C runtime's `rename`; `std` calls `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`. This is a probe run on **Windows**, and every
/// line of it is a Windows finding rather than a portable one:
///
/// ```text
/// rename over existing:                 OK
/// rename while dest open (shared read): OK
/// rename over read-only dest:           ERR PermissionDenied (os error 5)
/// ```
///
/// So there is no `remove_file` first and no retry loop. The read-only row is
/// the half that does *not* carry across: `rename(2)` is a directory operation
/// and asks for write permission on the directory and nothing at all on the
/// target, so a pad somebody chmodded 0444 on Unix is replaced without a word.
/// The Windows failure is honest by comparison, and deleting the target first
/// would fail in the same place for the same reason — so the answer there is
/// the message, which names the file and lets the user clear the bit.
///
/// **A rename detaches a link**, which is the other thing it costs. If the pad
/// is a symlink to somewhere else, or a hard link to a second name, the rename
/// replaces the link with an ordinary file and the other name stops changing —
/// silently, on the first autosave. Nobody is likely to have done that to a
/// file named after sixteen hex digits, and the alternative is writing through
/// the link, which gives back every failure the temporary file exists to avoid.
/// It is recorded because the symptom — a second name that quietly stops
/// updating — names no cause at all on its own.
///
/// The temporary file is named after the pad and this process, so two abeams
/// open on one workspace cannot be writing the same temporary file; the rename
/// then picks a winner between two whole saves rather than interleaving them.
/// It is removed on every failure this function returns from, because a
/// `scratch` directory slowly filling with `.tmp` files is a second bug
/// reported as the first. A kill between the write and the rename leaves one
/// behind that nothing sweeps: it is one file per killed process per workspace,
/// it is named after the pad it was going to be, and a sweep would be code that
/// deletes files in the user's profile on a guess about which of them are ours.
pub(super) fn save_at(path: &Path, text: &str, seen: Stamp) -> Result<Stamp, String> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(format!(
            "abeam could not save the scratch pad: {} does not name a file.",
            path.display()
        ));
    };

    // The first save of a session is the one that finds no `scratch` directory,
    // and on a fresh profile no `abeam` either.
    std::fs::create_dir_all(dir).map_err(|why| refused(path, &why))?;

    let temp = dir.join(format!(
        "{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    if let Err(why) = whole(&temp, text) {
        let _ = std::fs::remove_file(&temp);
        return Err(refused(path, &why));
    }

    // Asked as late as it can be asked and still be asked at all. See `Stamp`:
    // this catches the second abeam window, it is not a lock, and the window
    // between this line and the next is real and unclosed.
    if moved(seen, stamp(path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(elsewhere(path));
    }

    if let Err(why) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(refused(path, &why));
    }
    Ok(stamp(path))
}

/// The whole text, on the disk, before anything names it.
///
/// See [`save_at`] for why the flush is here and why the file is created narrow
/// rather than at the umask default.
fn whole(temp: &Path, text: &str) -> std::io::Result<()> {
    let mut file = private(temp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// A new file only its owner can read.
///
/// The one place in this module with a per-platform body rather than a
/// per-platform *rule*, so it is not the `from_appdata`/`from_xdg` shape and
/// cannot be: there is no argument to hand in and nothing to compute — one
/// platform has a mode to set at creation and the other has no such concept,
/// and what each does is only observable on itself. The Unix half is asserted
/// by a `#[cfg(unix)]` test.
#[cfg(unix)]
fn private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

/// Windows' half, where a new file inherits the directory's ACL and there is no
/// mode to ask for. `%APPDATA%` is already per-user, which is the protection
/// the Unix side has to spell out.
#[cfg(windows)]
fn private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::create(path)
}

/// A pad that could not be written, said to the person who typed it.
fn refused(path: &Path, why: &std::io::Error) -> String {
    format!(
        "abeam could not save the scratch pad to {}: {why}. What you typed is \
         still on screen — copy it somewhere before you quit.",
        path.display()
    )
}

/// A save refused because somebody else has written to the file.
///
/// Named as a refusal rather than a failure, because nothing went wrong: the
/// text is intact, the file is intact, and the two are different. What the
/// reader needs is the cause, since the remedy — close the other window, or
/// copy this out — is not one abeam can pick for them.
fn elsewhere(path: &Path) -> String {
    format!(
        "abeam did not save the scratch pad: {} has changed on disk since this \
         pad was read, which usually means a second abeam window has the same \
         workspace open. Writing now would delete whatever that one saved. What \
         you typed is still on screen — copy it somewhere before you quit.",
        path.display()
    )
}

/// A pad with nowhere to go at all, which is a machine that will not say where
/// the profile is rather than anything the user did.
///
/// Standing on the pane's opening screen rather than arriving with a failed
/// save, which is what makes it present tense. [`path_for`] answered `None`
/// when the pane was built, so this is known before anybody has typed — and the
/// version of this sentence that arrived two seconds *after* somebody filled
/// the pad was telling them about words that were already gone.
pub(super) fn nowhere() -> String {
    format!(
        "abeam has nowhere in your profile to keep this pad: nothing absolute \
         is set for {PROFILE}. Nothing typed here will be saved, so copy \
         anything worth keeping before you close it."
    )
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Nothing here writes the process environment, for `crate::config`'s reason:
/// this suite is one binary with three hundred and fifty other tests in it,
/// several of which spawn children that inherit whatever it has been doing to
/// `APPDATA`. So the platform rules are tested through [`from_appdata`] and
/// [`from_xdg`], which take their variables as arguments, and the reading and
/// writing through [`load_at`] and [`save_at`], which take a path — and which
/// therefore also keep the suite out of the profile of whoever is running it.
/// The pane's own tests go through the same two, for the same reason.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// Two directories this platform calls absolute.
    ///
    /// Spelled for the machine running the test rather than for the rule under
    /// test, because `Path::is_absolute` is the platform's own question and not
    /// a portable one: `/home/philm` has no drive on it, so Windows does not
    /// call it absolute, and a Unix rule tested on Windows with a Unix path
    /// would be testing the wrong half.
    #[cfg(windows)]
    const ABS: &str = r"C:\Users\philm\AppData\Roaming";
    #[cfg(windows)]
    const ELSEWHERE: &str = r"D:\state";
    #[cfg(unix)]
    const ABS: &str = "/home/philm";
    #[cfg(unix)]
    const ELSEWHERE: &str = "/var/data-for-me";

    /// What a rule put on the end of the directory it was given, as words — so
    /// that neither the separator nor the drive letter is in the assertion.
    fn under(path: Option<PathBuf>, base: &str) -> Vec<String> {
        path.expect("an absolute variable is followed")
            .strip_prefix(base)
            .expect("the answer is under the directory it was given")
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect()
    }

    // --- where the file is -------------------------------------------------

    #[test]
    fn the_pad_lives_under_the_profile_on_both_platforms() {
        // Windows: one variable, and abeam's own directory inside it.
        assert_eq!(
            under(from_appdata(Some(PathBuf::from(ABS))), ABS),
            ["abeam", "scratch"]
        );

        // Unix: the data variable when it is set, used as the data home it
        // says it is...
        assert_eq!(
            under(from_xdg(Some(PathBuf::from(ABS)), None), ABS),
            ["abeam", "scratch"]
        );
        // ...and `~/.local/share` when it is not, which is the specification's
        // own fallback and the two components that say this is user data rather
        // than the configuration `crate::config` puts in `~/.config`.
        assert_eq!(
            under(from_xdg(None, Some(PathBuf::from(ABS))), ABS),
            [".local", "share", "abeam", "scratch"]
        );
        // The variable wins over the home directory when both are there, which
        // is what "data home" means.
        assert_eq!(
            under(
                from_xdg(Some(PathBuf::from(ELSEWHERE)), Some(PathBuf::from(ABS))),
                ELSEWHERE
            ),
            ["abeam", "scratch"]
        );

        // Nothing said, nowhere to write — which the caller turns into a
        // message rather than a panic.
        assert_eq!(from_appdata(None), None);
        assert_eq!(from_xdg(None, None), None);
    }

    #[test]
    fn a_relative_variable_is_refused_rather_than_resolved_against_wherever_we_stand() {
        // Joining onto a relative path leaves a relative path, so the write
        // stops being a question about the user's profile and becomes one about
        // the process's directory — which for most of abeam's life is
        // `%SystemRoot%` or `/`, and before that line is the repository on
        // screen. `APPDATA=.` would put the pad inside a clone, which is the
        // one directory this module refuses to write to.
        for relative in [".", "abeam", "..", "", "  "] {
            assert_eq!(
                from_appdata(Some(PathBuf::from(relative))),
                None,
                "`{relative}` is not somewhere abeam may write a pad"
            );
            assert_eq!(from_xdg(Some(PathBuf::from(relative)), None), None);
            // And on the home side too, which is where an empty variable
            // actually turns up: a container or a service unit can export
            // `HOME=`.
            assert_eq!(from_xdg(None, Some(PathBuf::from(relative))), None);
        }

        // ...and a relative `XDG_DATA_HOME` is *ignored* rather than fatal: it
        // is discarded, which is the whole of the hazard, and the home
        // directory then answers as though the variable had never been set.
        // Costing somebody their notes over a variable set for another
        // program's benefit is the alternative.
        assert_eq!(
            under(
                from_xdg(Some(PathBuf::from(".")), Some(PathBuf::from(ABS))),
                ABS
            ),
            [".local", "share", "abeam", "scratch"],
            "a relative XDG_DATA_HOME is ignored, not followed and not fatal"
        );
    }

    #[test]
    fn the_pad_is_a_markdown_file_named_after_the_workspace() {
        // Read from the environment rather than written to it, so this says
        // what this machine would really do without changing what the tests
        // beside it see. A machine with no profile variable at all answers
        // `None`, and that is the same answer `save` turns into a sentence.
        let root = Path::new(ABS);
        if let Some(path) = path_for(root) {
            assert!(path.is_absolute(), "{}", path.display());
            assert_eq!(
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                Some(format!("{}.md", crate::paths::workspace_key(root)))
            );
            assert!(path.ends_with(Path::new(DIR).join(SCRATCH).join(file(root))));
        }
    }

    // --- a pad that is there, and one that is not --------------------------

    #[test]
    fn a_pad_that_was_never_written_is_an_empty_one() {
        let dir = TempDir::new("pad-absent");
        assert_eq!(
            load_at(&dir.path().join(file(dir.path()))),
            Loaded::default()
        );
        // Absent rather than unknown, because "there is no file" is something a
        // stat can be certain of — and a first save compares against it.
        assert_eq!(Loaded::default().stamp, Stamp::Absent);
    }

    #[test]
    fn a_pad_that_cannot_be_read_does_not_stop_abeam_starting() {
        // A directory where the file should be — the shape a bad backup or an
        // interrupted sync leaves behind, and the one an `expect` here would
        // turn into a program that will not start.
        let dir = TempDir::new("pad-unreadable");
        let path = dir.path().join(file(dir.path()));
        std::fs::create_dir_all(&path).expect("a directory in the file's place");

        let got = load_at(&path);
        assert_eq!(got.text, "");
        assert!(
            got.unreadable,
            "an empty pad and a pad that would not open are not the same answer"
        );
        assert!(!got.truncated);
    }

    /// The failure a probe found and an assumption had missed, built rather
    /// than described.
    ///
    /// A handle opened with no sharing at all is what an antivirus scan,
    /// OneDrive, the search indexer or a backup agent holds for a fraction of a
    /// second, and while it is held `File::open` fails. The old answer to that
    /// was an empty pad, and the save that followed it succeeded — because the
    /// rename needs delete rights on the target's parent and not read access to
    /// the target — so somebody's notes were replaced by nothing at all.
    ///
    /// Windows only, because `share_mode` is how that platform expresses this
    /// and Unix has no equivalent to open with. The rule it proves is not
    /// platform-specific: anything other than `NotFound` is a file abeam has
    /// not seen.
    #[test]
    #[cfg(windows)]
    fn a_pad_held_open_by_another_program_says_so_rather_than_looking_empty() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new("pad-locked");
        let path = dir.path().join(file(dir.path()));
        let notes = "the thing I must not forget before standup\n";
        std::fs::write(&path, notes).expect("write the pad");

        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("hold the pad open with no sharing");

        let got = load_at(&path);
        assert!(got.unreadable, "a locked pad came back looking like no pad");
        assert_eq!(got.text, "");

        drop(held);
        // ...and the file is exactly as it was. What made this critical is that
        // the pane used to follow that empty read with a save.
        assert_eq!(load_at(&path).text, notes);
    }

    #[test]
    fn a_pad_saved_is_the_pad_that_comes_back() {
        let dir = TempDir::new("pad-roundtrip");
        // Two directories below the fixture, because the first save of a
        // session is the one that finds neither of them there.
        let path = dir.path().join(DIR).join(SCRATCH).join(file(dir.path()));

        let note = "ask about the retry budget\n\n- and the 30s timeout\n";
        save_at(&path, note, Stamp::Absent).expect("a pad this test just made a directory for");

        let got = load_at(&path);
        assert_eq!(got.text, note);
        assert!(!got.truncated);
    }

    #[test]
    fn saving_again_replaces_the_pad_rather_than_leaving_half_of_the_old_one() {
        let dir = TempDir::new("pad-overwrite");
        let path = dir.path().join(file(dir.path()));

        let long = "the first note, which is the longer of the two\nand two lines\n";
        let short = "the second\n";
        let first = save_at(&path, long, Stamp::Absent).expect("a first pad");
        // The call that fails outright on a platform whose rename refuses a
        // destination that is already there, and whose result would still hold
        // the tail of the longer text if the write went straight to the target.
        save_at(&path, short, first).expect("a second pad over the first");

        assert_eq!(load_at(&path).text, short);

        // ...and nothing beside it. A temporary file left behind is a `scratch`
        // directory that fills up quietly and gets reported as something else.
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read the fixture directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(left, [file(dir.path())]);
    }

    #[test]
    fn a_pad_that_is_not_utf8_comes_back_readable_rather_than_empty() {
        // Somebody's editor saved it as Latin-1, or a paste brought one bad
        // byte in. A strict read answers a file full of notes with an empty
        // pane, and the next save overwrites it.
        let dir = TempDir::new("pad-lossy");
        let path = dir.path().join(file(dir.path()));
        std::fs::write(&path, b"caf\xe9 at 3\n").expect("write the bytes");

        let text = load_at(&path).text;
        assert!(text.starts_with("caf"), "{text:?}");
        assert!(text.ends_with(" at 3\n"), "{text:?}");
    }

    #[test]
    fn a_pad_that_opens_with_a_byte_order_mark_does_not_open_with_a_character() {
        // Notepad writes one and says nothing about it, and every pad on a
        // Windows machine is one round trip through some other editor away
        // from having one. Left in, it is an invisible first character on the
        // first line: the caret can be put in front of it, `is_empty` is false
        // for a pad nobody has typed in, and `# heading` is a paragraph
        // beginning with a `#` rather than a heading.
        let dir = TempDir::new("pad-bom");
        let path = dir.path().join(file(dir.path()));
        std::fs::write(&path, "\u{feff}# a heading\n".as_bytes()).expect("write the pad");

        let got = load_at(&path);
        assert_eq!(got.text, "# a heading\n");
        assert!(!got.truncated);
    }

    /// Private notes, and a mode that says so.
    ///
    /// `fs::write` creates at the umask default, which on most machines is 022
    /// and so world-readable — meaning abeam's own first autosave would open up
    /// a pad somebody had deliberately chmodded 600. `rename(2)` carries the
    /// source's mode over, so creating the temporary file narrow is what makes
    /// the pad narrow.
    #[test]
    #[cfg(unix)]
    fn a_pad_is_created_readable_only_by_the_person_who_typed_it() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("pad-private");
        let path = dir.path().join(file(dir.path()));
        save_at(&path, "nobody else's business\n", Stamp::Absent).expect("a pad");

        let mode = std::fs::metadata(&path)
            .expect("stat the pad")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the pad is {:o}", mode & 0o777);
    }

    // --- two windows, one pad ---------------------------------------------

    #[test]
    fn a_stamp_nobody_could_take_never_stops_a_save() {
        // The rule this type turns on, stated where it can be read without
        // arranging a filesystem to state it. A stat that could not be taken is
        // not evidence of anything, and this is detection of an accident rather
        // than a lock: it must never be the reason a save does not happen.
        let seen = Stamp::Seen(61, None);
        assert!(!moved(Stamp::Unknown, seen));
        assert!(!moved(seen, Stamp::Unknown));
        assert!(!moved(Stamp::Unknown, Stamp::Unknown));

        // Everything else is compared as written down.
        assert!(!moved(seen, seen));
        assert!(!moved(Stamp::Absent, Stamp::Absent));
        assert!(moved(Stamp::Absent, seen), "somebody else created it");
        assert!(moved(seen, Stamp::Absent), "somebody else deleted it");
        assert!(
            moved(Stamp::Seen(61, None), Stamp::Seen(2, None)),
            "the length alone carries it, which is what a coarse mtime needs"
        );
    }

    #[test]
    fn a_pad_a_second_window_has_written_is_refused_rather_than_overwritten() {
        let dir = TempDir::new("pad-two-windows");
        let path = dir.path().join(file(dir.path()));

        // This window reads nothing and writes its first note.
        let mine = save_at(&path, "mine\n", Stamp::Absent).expect("a first pad");

        // The other window saves while this one is still holding what it read.
        let theirs = "theirs, and longer than mine\n";
        std::fs::write(&path, theirs).expect("somebody else's save");

        let why = save_at(&path, "mine, with more added\n", mine)
            .expect_err("this window must not write over a file that has moved");
        assert!(why.contains("changed on disk"), "{why}");
        assert!(why.contains("still on screen"), "{why}");
        assert_eq!(
            load_at(&path).text,
            theirs,
            "the other window's notes were overwritten anyway"
        );

        // ...and nothing was left lying beside it.
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("read the fixture")
                .count(),
            1
        );
    }

    #[test]
    fn a_windows_own_saves_are_not_mistaken_for_somebody_elses() {
        // The other half, and the one that decides whether this is usable at
        // all: an autosave every two seconds must not start refusing itself
        // after the first one. Each save hands back what it wrote, and that is
        // what the next one compares against.
        let dir = TempDir::new("pad-own-saves");
        let path = dir.path().join(file(dir.path()));

        let mut stamp = Stamp::Absent;
        for note in ["one\n", "one, two\n", "one, two, three\n"] {
            stamp = save_at(&path, note, stamp).expect("a save of this window's own");
        }
        assert_eq!(load_at(&path).text, "one, two, three\n");

        // And what a load hands back is the same currency, so a pane that reads
        // an existing pad can go on saving it.
        let reread = load_at(&path).stamp;
        save_at(&path, "and four\n", reread).expect("a save after a read");
        assert_eq!(load_at(&path).text, "and four\n");
    }

    // --- a pad that is bigger than the buffer will hold --------------------

    #[test]
    fn a_pad_longer_than_the_cap_comes_back_flagged_and_the_file_is_untouched() {
        let dir = TempDir::new("pad-too-big");
        let path = dir.path().join(file(dir.path()));

        // Whole lines, so that what comes back can be compared with what went
        // in rather than with wherever an arbitrary cut happened to land.
        let line = "a line of somebody's notes, long enough to be one\n";
        let pad = line.repeat(MAX_BYTES / line.len() + 32);
        assert!(pad.len() > MAX_BYTES, "the fixture proves nothing");
        std::fs::write(&path, &pad).expect("write the pad");

        let got = load_at(&path);
        assert!(got.truncated, "a pad over the cap has to say so");
        assert!(
            got.text.len() <= MAX_BYTES,
            "...and must still fit the buffer it is going into"
        );
        assert!(
            pad.starts_with(&got.text),
            "what came back is the head of it"
        );
        assert!(got.text.ends_with('\n'), "cut at a line and not mid-word");

        // The file itself is exactly as it was. Reading never writes, and the
        // pane's refusal to save is what has to keep it that way for the rest
        // of the session — which is the whole reason `truncated` is on the
        // struct rather than being dealt with quietly here.
        assert_eq!(
            std::fs::read(&path).expect("the file is still there").len(),
            pad.len()
        );
    }

    #[test]
    fn a_pad_exactly_at_the_cap_is_not_flagged_and_one_byte_more_is() {
        // The boundary the notice turns on. An off-by-one on this side of it
        // puts a warning on a document that is perfectly fine and then refuses
        // to save it for the rest of the session, which is a bug that looks
        // from the outside like the feature being broken.
        let dir = TempDir::new("pad-at-cap");
        let path = dir.path().join(file(dir.path()));

        std::fs::write(&path, "x".repeat(MAX_BYTES)).expect("write the pad");
        let got = load_at(&path);
        assert!(!got.truncated);
        assert_eq!(got.text.len(), MAX_BYTES);

        // One byte more, and with no line ending anywhere in it — so this is
        // also the pad whose first line is longer than the whole cap, which is
        // the one there is nothing to back off to.
        std::fs::write(&path, "x".repeat(MAX_BYTES + 1)).expect("write the pad");
        let got = load_at(&path);
        assert!(got.truncated);
        assert_eq!(got.text.len(), MAX_BYTES);
    }

    #[test]
    fn a_pad_with_nowhere_to_go_says_so_in_a_sentence() {
        // Both messages name the file or the variable, and both end by telling
        // the user the text is still in front of them — which is the only
        // useful thing anybody can do about either.
        let path = Path::new(ABS).join("scratch.md");
        let why = refused(
            &path,
            &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access is denied."),
        );
        assert!(why.contains(&path.display().to_string()), "{why}");
        assert!(why.contains("Access is denied."), "{why}");
        assert!(why.contains("still on screen"), "{why}");

        // The refusal that is not a failure: the file is fine, the text is
        // fine, and the reader needs the cause rather than an apology.
        let why = elsewhere(&path);
        assert!(why.contains(&path.display().to_string()), "{why}");
        assert!(why.contains("changed on disk"), "{why}");
        assert!(why.contains("still on screen"), "{why}");

        // And the one with no file to name, which stands on the opening screen
        // rather than arriving after a save — so it is present tense and tells
        // somebody what to do before they have typed, not after.
        let why = nowhere();
        assert!(why.contains(PROFILE), "{why}");
        assert!(why.contains("nowhere"), "{why}");
        assert!(why.contains("copy"), "{why}");
    }
}
