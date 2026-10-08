//! Reading a file to edit it, and writing it back: the first code in abeam that
//! writes inside a repository.
//!
//! Until this module abeam wrote exactly one kind of file, and it wrote it into
//! the user's profile: `crate::panes::pad::store`'s scratch pad, whose module
//! docs open by calling it the exception to a rule the whole crate had kept.
//! Every other path abeam touches it reads. The files view is about to let
//! somebody type into the document on screen and press `Ctrl+S`, and the file
//! that save replaces is not abeam's — it is in a repository, beside the work,
//! where the agent next door is writing too and where git will show every byte
//! abeam gets wrong. So this is the second exception, and a larger one, and the
//! rules it keeps are written down here once rather than rediscovered at each
//! call site.
//!
//! **A save gives back the file it was given.** Opening a file to edit it is a
//! stricter question than opening it to read it: `crate::panes::viewer::load`
//! decodes lossily and folds line endings, which is right for a page somebody
//! is reading and wrong for one about to be written back, because the first
//! save would make the lossy reading true on disk. So [`open`](fn@open)
//! refuses whatever it could not give back byte for byte — text that is not
//! UTF-8, line endings that are not all one kind, a binary, a file too large
//! to hold whole — and records the rest: the line ending, the byte order mark,
//! the final newline. [`Format::encode`] puts all three back. A refusal is a
//! reason to show the file read-only, never a reason to show it differently.
//!
//! **Never half a file.** [`save`](fn@save) writes a temporary file beside the
//! target, gets it onto the disk, and only then puts it in the target's place
//! — the pad store's arrangement, for the pad store's reason, and the
//! mechanism itself ([`Temp`]) is now shared by both. The difference is the
//! last step. The pad renames over a file that is abeam's own; a file in a
//! repository has an ACL, attributes and a creation time on Windows and
//! permission bits on Unix that belong to its owner, so the replacement keeps
//! them (`ReplaceFileW` on Windows; the target's mode copied onto the
//! temporary file on Unix) and a file marked read-only is refused rather than
//! replaced.
//!
//! **Never over somebody else's write.** The agent is writing in this
//! repository while the user types. Immediately before the replacement the
//! target is read again and compared, byte for byte, with what the editor was
//! opened on ([`Baseline`]); anything else there — a change, a deletion, a file
//! that appeared where none was — refuses the save and writes nothing. The
//! caller may then save again with [`Options::force`], which is the second
//! `Ctrl+S` and the only way through. Like the pad store's `Stamp`, this
//! detects an ordinary accident and is not a lock: the instant between the
//! comparison and the replacement is real and nothing here closes it.
//!
//! **Never outside the workspace.** A path is judged by what it resolves to,
//! links and junctions followed, under `crate::paths`'s comparison rule — at
//! [`open`](fn@open) and again at [`save`](fn@save), because a link can be
//! repointed in between.
//!
//! **Nothing left behind.** The temporary file is removed on every way out of a
//! save that failed, because a repository slowly filling with them is a second
//! bug reported as the first, and they would show up in `git status` beside the
//! file. There is one exception and [`save`](fn@save) names it: a replacement
//! Windows abandoned half way, where the temporary file is the only copy of
//! the text.
//!
//! **Recovery copies stay out of the repository.** Unsaved text is kept for a
//! later session by [`drafts`], beside the pad in the profile, for the pad's
//! reasons: inside the workspace it would be an untracked file in the git pane
//! and a change the watcher reports.
//!
//! - [`open`](mod@open) is the strict read, and [`Format`] the way back to
//!   bytes;
//! - [`baseline`] is what a save compares the disk with;
//! - [`save`](mod@save) is the replacement;
//! - [`drafts`] is the recovery copy;
//! - [`profile`] is where in the profile abeam's own files live, the pad's and
//!   the drafts' alike.

pub mod baseline;
pub mod drafts;
pub mod open;
pub(crate) mod profile;
pub mod save;

pub use baseline::{Baseline, Fingerprint};
pub use open::{Format, Opened, Refusal, open};
pub use save::{Options, SaveError, Saved, save};

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

// ---------------------------------------------------------------------------
// the temporary file
// ---------------------------------------------------------------------------

/// What the name of every temporary file abeam writes beside a file it is
/// about to replace ends with.
///
/// The whole name is `.<file>.<pid>.abeam-save~` — [`temp_name`] builds it and
/// [`is_save_temp`] recognises it — and each piece is there for somebody:
///
/// - the leading dot keeps it out of a plain `ls` for the instant it exists;
/// - the file's own name says what it was going to be, to a person who finds
///   one left behind by a killed process and has to decide whether to delete
///   it;
/// - the process id keeps two abeams saving the same file from reaching for
///   one name between them;
/// - `abeam-save` says whose it is, in words, so that nobody has to guess;
/// - the trailing `~` is the mark editors have used for their own backup files
///   for forty years, so a `.gitignore` written with `*~` in it — many are —
///   already keeps a stray one out of a commit.
///
/// When that name is already taken the next is `.<file>.<pid>-1.abeam-save~`,
/// and so on: see [`Temp::write`] for why a taken name is never reused.
///
/// The watcher filters the pattern out (`crate::watch`), so a save reports the
/// file it saved and not the scaffolding it saved through.
pub(crate) const SAVE_SUFFIX: &str = ".abeam-save~";

/// The `n`th name a save of a file called `name` may write its temporary file
/// under: `n` of zero is the plain one, and each after it carries a counter.
///
/// A name and not a path, so that the caller puts it in the target's own
/// directory: a rename across volumes is a copy and a delete with a window in
/// the middle — the pad store's paragraph, and here the reason is sharper:
/// `ReplaceFileW` refuses outright to work across volumes.
pub(crate) fn temp_name(name: &OsStr, n: u32) -> OsString {
    let mut temp = OsString::from(".");
    temp.push(name);
    let pid = std::process::id();
    match n {
        0 => temp.push(format!(".{pid}{SAVE_SUFFIX}")),
        n => temp.push(format!(".{pid}-{n}{SAVE_SUFFIX}")),
    }
    temp
}

/// Whether `path` names one of [`temp_name`]'s files, whoever's process wrote
/// it and whichever counter it carries.
///
/// Asked of the name's bytes rather than of a decoded string, because the
/// watcher asks it of every path in every batch and a file name that is not
/// valid Unicode is still a file name: the prefix and suffix are ASCII, so
/// they mean the same thing in any encoding the platform uses.
pub(crate) fn is_save_temp(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.as_encoded_bytes();
        name.len() > 1 + SAVE_SUFFIX.len()
            && name.starts_with(b".")
            && name.ends_with(SAVE_SUFFIX.as_bytes())
    })
}

/// How many names [`Temp::write`] will try before it gives up: enough for
/// any number of leftovers a person could plausibly have, few enough that a
/// directory that refuses every name is reported rather than walked for ever.
const NAMES: u32 = 64;

/// Who may read a temporary file, and therefore the file it becomes.
#[derive(Debug, Clone)]
pub(crate) enum Perms {
    /// Only its owner: 0600 on Unix, set as the file is created, so that the
    /// text is never on disk any wider than that. On Windows the directory's
    /// ACL, which in `%APPDATA%` is already per-user. The pad's and the
    /// recovery copies' — private notes, kept in the profile.
    Private,
    /// What any editor's new file gets: the umask's default on Unix, the
    /// directory's ACL on Windows. A file being created in a repository.
    Default,
    /// What the file this one will replace has — its permission bits, and its
    /// owner and group where the platform will allow it — so that a save
    /// leaves them as they were: a script stays executable, a group-writable
    /// file stays group-writable, and abeam imposes nothing.
    ///
    /// Unix only in effect. On Windows `std::fs::Permissions` is a read-only
    /// flag and nothing else, and `ReplaceFileW` carries the target's ACL and
    /// attributes across itself; copying the flag would only make the
    /// temporary file read-only and the replacement fail.
    #[cfg_attr(
        windows,
        // `#[allow]` and not `#[expect]`: the condition is a `cfg`, so on the
        // platform this *is* read an expectation would be unfulfilled.
        allow(dead_code, reason = "read by the Unix half of `apply` only; see above")
    )]
    Like(std::fs::Metadata),
}

/// A temporary file with all of its bytes on the disk, removed when it is
/// dropped unless it has been put somewhere.
///
/// The removal is the point of the type. A save has half a dozen ways out
/// between writing the file and placing it — a conflict, a refusal, a failed
/// rename, a retry that ran out — and every one of them must leave the
/// directory as it found it. A `remove_file` before each `return` is a rule
/// kept by hand at every one of those exits, and the pad store kept it that
/// way, correctly, for three exits. The files view has more, so the rule is
/// kept by `Drop` instead, and a new exit cannot forget it.
///
/// What `Drop` removes is only ever a file this value created — see
/// [`Temp::write`] — because a removal that reached anything else would be the
/// one way this type could destroy somebody's text rather than protect it.
#[derive(Debug)]
pub(crate) struct Temp {
    path: PathBuf,
    /// Whether dropping this removes the file: true from the moment the file
    /// exists until something else owns its name.
    armed: bool,
}

impl Temp {
    /// Create a new file under the first of `name(0)`, `name(1)`, … that is
    /// free, write all of `bytes` into it, and wait for the disk to have them.
    ///
    /// **A name that is taken is never opened, truncated or removed.** It used
    /// to be: the file was created with `truncate`, so whatever already had the
    /// name was emptied and then, on a failure, deleted. And what can have the
    /// name is not only debris. A save Windows abandoned half way leaves its
    /// temporary file behind as the *only* copy of the text
    /// (`crate::disk::save` says so in `SaveError::Stranded`), under exactly
    /// the name the user's next `Ctrl+S` reaches for; a process id is reused,
    /// on Windows within minutes, next to a killed process's leftover; and a
    /// person may simply have a file called that. So the file is created with
    /// `create_new`, which is one indivisible check-and-create on both
    /// platforms — it refuses a name that is there, a link included, rather
    /// than following it — and a refusal moves on to the next name rather than
    /// taking this one.
    ///
    /// **The flush is the half that is easy to leave out and that makes the
    /// rest true**, which is the pad store's argument and holds unchanged: an
    /// unsynced rename can reach the disk before the bytes it renames, so a
    /// power cut leaves a file of the right name and no contents — the one
    /// outcome the whole arrangement exists to prevent. `fs::write` does not
    /// flush. What stays unclosed, as there: the directory entry itself is not
    /// synced, so a power cut in the last instant can lose the rename, which
    /// leaves the previous file whole.
    ///
    /// Armed only once the file exists, so a failure to create it removes
    /// nothing — there is nothing of this call's to remove. Any failure after
    /// that removes the file on the way out.
    pub(crate) fn write(
        name: impl Fn(u32) -> PathBuf,
        bytes: &[u8],
        perms: Perms,
    ) -> io::Result<Temp> {
        let mut n = 0;
        let (mut file, path) = loop {
            let path = name(n);
            match create(&path, &perms) {
                Ok(file) => break (file, path),
                // Taken by anything at all. `AlreadyExists` is how a file
                // answers; Windows answers for a directory on the name with
                // `ERROR_ACCESS_DENIED`, so whether something is there is asked
                // of the name rather than read off the error.
                Err(why)
                    if n + 1 < NAMES
                        && (why.kind() == io::ErrorKind::AlreadyExists
                            || std::fs::symlink_metadata(&path).is_ok()) =>
                {
                    n += 1;
                }
                Err(why) => return Err(why),
            }
        };
        let temp = Temp { path, armed: true };
        file.write_all(bytes)?;
        apply(&file, perms)?;
        file.sync_all()?;
        Ok(temp)
    }

    /// Where the file is.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Rename it over `target`, which `std` does with `MoveFileExW` and
    /// `MOVEFILE_REPLACE_EXISTING` on Windows and `rename(2)` on Unix: the name
    /// then points at the whole of the new file or the whole of the old one and
    /// at no moment at half of either. Removed if the rename fails.
    pub(crate) fn rename_over(mut self, target: &Path) -> io::Result<()> {
        std::fs::rename(&self.path, target)?;
        self.armed = false;
        Ok(())
    }

    /// Something else has put the file in place, or the file is the only copy
    /// of what it holds and must stay where it is: either way it is no longer
    /// this value's to remove. Hands back where it was written.
    pub(crate) fn release(mut self) -> PathBuf {
        self.armed = false;
        std::mem::take(&mut self.path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if self.armed {
            // Best effort, and it has to be: this runs on the way out of a
            // failure that is already being reported, and a second failure
            // here has nobody to tell.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The new file, opened for writing, created at the narrowest mode it will
/// need — and only if nothing at all is at `path` yet. See [`Temp::write`].
///
/// One place with a per-platform body rather than a per-platform *rule*, for
/// the reason the pad store gave when this was its `private`: one platform has
/// a mode to set at creation and the other has no such concept, and what each
/// does is only observable on itself. The Unix halves are asserted by
/// `#[cfg(unix)]` tests — the pad's 0600 there, a project file's kept bits in
/// [`save`](fn@save).
#[cfg(unix)]
fn create(path: &Path, perms: &Perms) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Narrow first for a file that will be widened to its target's bits,
    // because the text is in it before the bits are copied on.
    if matches!(perms, Perms::Private | Perms::Like(_)) {
        options.mode(0o600);
    }
    options.open(path)
}

/// Windows' half, where a new file inherits the directory's ACL and there is
/// no mode to ask for.
#[cfg(windows)]
fn create(path: &Path, _perms: &Perms) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// The target's owner and bits, onto the open file — `fchown`, then `fchmod`,
/// so the umask that narrowed the creation has no say in the bits.
///
/// Owner first, because Linux clears the set-user-ID and set-group-ID bits on a
/// change of owner, so the other order would lose them. The owner is best
/// effort and the bits are not: root may give a file to anybody, a member of a
/// group may give it to that group, and anybody else saving a file they do not
/// own keeps it as theirs, which `crate::disk::save` says out loud rather than
/// refusing the save over. Bits that cannot be set are an error, because a
/// file whose mode was not kept is the thing this variant exists to prevent.
#[cfg(unix)]
fn apply(file: &File, perms: Perms) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    match perms {
        Perms::Like(meta) => {
            // Both, or failing that the group alone: one call asking for an
            // owner it may not give fails whole, group and all.
            if std::os::unix::fs::fchown(file, Some(meta.uid()), Some(meta.gid())).is_err() {
                let _ = std::os::unix::fs::fchown(file, None, Some(meta.gid()));
            }
            file.set_permissions(meta.permissions())
        }
        Perms::Private | Perms::Default => Ok(()),
    }
}

/// Nothing to copy: see [`Perms::Like`].
#[cfg(windows)]
fn apply(_file: &File, _perms: Perms) -> io::Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// a file somebody else has open
// ---------------------------------------------------------------------------

/// The pauses between attempts when Windows says another program has a file
/// open: five tries, a little under a quarter of a second of waiting in all.
/// Long enough for a scanner to let go of a file it opened as the agent wrote
/// it, short enough that a file held for good is reported before the user
/// wonders whether the key worked. Shared by the strict read and the save, so
/// that "a moment" means one thing in both.
pub(crate) const PAUSES: [Duration; 4] = [
    Duration::from_millis(15),
    Duration::from_millis(30),
    Duration::from_millis(60),
    Duration::from_millis(120),
];

/// Whether `why` is Windows saying another program has the file open without
/// sharing it — `ERROR_SHARING_VIOLATION`, which Defender, the search indexer
/// and OneDrive all cause for a moment at a time.
#[cfg(windows)]
pub(crate) fn held(why: &io::Error) -> bool {
    why.raw_os_error() == Some(winapi::shared::winerror::ERROR_SHARING_VIOLATION as i32)
}

/// Never: a file open elsewhere stops nothing on Unix.
#[cfg(unix)]
pub(crate) fn held(_why: &io::Error) -> bool {
    false
}

/// `f`, tried again after each of [`PAUSES`] for as long as it fails in a way
/// `transient` calls a moment's trouble, and its last answer either way.
pub(crate) fn patiently<T>(
    transient: impl Fn(&io::Error) -> bool,
    mut f: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut pauses = PAUSES.iter();
    loop {
        match f() {
            Err(why) if transient(&why) => match pauses.next() {
                Some(pause) => std::thread::sleep(*pause),
                None => return Err(why),
            },
            done => return done,
        }
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// The names a save of `target` would try, as `Temp::write` takes them.
    fn names(target: &Path) -> impl Fn(u32) -> PathBuf + '_ {
        move |n| target.with_file_name(temp_name(target.file_name().expect("a name"), n))
    }

    #[test]
    fn a_temporary_file_is_named_after_its_target_beside_it_and_recognised() {
        let dir = TempDir::new("disk-temp-name");
        let target = dir.path().join("README.md");
        let temp = names(&target)(0);
        assert_eq!(temp.parent(), Some(dir.path()), "the same directory");
        let name = temp.file_name().expect("a name").to_string_lossy().into_owned();
        let pid = std::process::id();
        assert_eq!(name, format!(".README.md.{pid}.abeam-save~"));
        assert!(is_save_temp(&temp));
        let second = names(&target)(2);
        assert_eq!(
            second.file_name().map(|name| name.to_string_lossy().into_owned()),
            Some(format!(".README.md.{pid}-2.abeam-save~"))
        );
        assert!(is_save_temp(&second), "a counted name is still recognised");

        // Another process's is still ours, and nothing else is.
        assert!(is_save_temp(&dir.path().join(".notes.md.4242.abeam-save~")));
        for not in [
            "README.md",
            ".README.md",
            "README.md.abeam-save~",
            ".abeam-save~",
            "README.md~",
            ".README.md.swp",
        ] {
            assert!(!is_save_temp(&dir.path().join(not)), "{not}");
        }
    }

    #[test]
    fn a_temporary_file_that_is_dropped_is_gone_and_one_renamed_is_kept() {
        let dir = TempDir::new("disk-temp-drop");
        let target = dir.path().join("a.txt");
        let temp = Temp::write(names(&target), b"some text\n", Perms::Private)
            .expect("write the temporary file");
        let at = temp.path().to_path_buf();
        assert_eq!(std::fs::read(&at).expect("it is there"), b"some text\n");
        drop(temp);
        assert!(!at.exists(), "dropped, and so removed");

        let temp = Temp::write(names(&target), b"kept\n", Perms::Private).expect("write it again");
        temp.rename_over(&target).expect("rename it into place");
        assert_eq!(std::fs::read(&target).expect("the target"), b"kept\n");
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("read the fixture").count(),
            1,
            "and nothing beside it"
        );
    }

    #[test]
    fn a_name_that_is_taken_is_passed_over_and_what_has_it_is_untouched() {
        // A file — the only copy of a stranded save, or anybody's — and a
        // directory, on the first two names. Neither is opened, emptied or
        // removed: the temporary file goes under the third name, and when it
        // is dropped only it goes.
        let dir = TempDir::new("disk-temp-taken");
        let target = dir.path().join("a.md");
        let names = names(&target);
        std::fs::write(names(0), b"the only copy\n").expect("a file on the first name");
        std::fs::create_dir_all(names(1)).expect("a directory on the second");

        let temp = Temp::write(&names, b"new\n", Perms::Private).expect("a third name");
        assert_eq!(temp.path(), names(2));
        drop(temp);
        assert_eq!(std::fs::read(names(0)).expect("still there"), b"the only copy\n");
        assert!(names(1).is_dir(), "the directory is still there");
        assert!(!names(2).exists(), "and only what this call made was removed");
    }

    #[test]
    fn a_moment_of_trouble_is_waited_out_and_anything_else_is_not() {
        let mut tries = 0;
        let busy = || io::Error::other("busy");
        let got = patiently(
            |why| why.to_string() == "busy",
            || {
                tries += 1;
                if tries < 3 { Err(busy()) } else { Ok(tries) }
            },
        );
        assert_eq!(got.expect("waited out"), 3);

        let mut tries = 0;
        let got: io::Result<()> = patiently(
            |_| true,
            || {
                tries += 1;
                Err(busy())
            },
        );
        assert!(got.is_err());
        assert_eq!(tries, PAUSES.len() + 1, "one try, and one after every pause");

        let mut tries = 0;
        let got: io::Result<()> = patiently(
            |_| false,
            || {
                tries += 1;
                Err(busy())
            },
        );
        assert!(got.is_err());
        assert_eq!(tries, 1, "a failure that is not transient is not retried");
    }
}
