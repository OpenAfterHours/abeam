//! Putting an edited file back where it was, as itself.
//!
//! [`save`] is the pad store's `save_at` grown up for a file that is not
//! abeam's. The shape is the same — a temporary file beside the target, all of
//! it on the disk, then one operation that puts it in the target's place — and
//! the mechanism is literally shared, as `crate::disk::Temp`. What is new is
//! everything a repository file has that a pad file does not.
//!
//! ## What a file keeps
//!
//! A rename keeps nothing of the file it replaces: the new file is the
//! temporary one under another name, with the temporary one's permissions,
//! owner, ACL and creation time. That is right for the pad, which abeam made
//! and whose mode abeam chooses, and wrong for a file somebody else made.
//!
//! **On Windows the replacement is `ReplaceFileW`**, which exists for exactly
//! this: it gives the new file the old one's ACL, attributes, creation time,
//! short name and object ID, and swaps the data in under them. A hidden file
//! stays hidden and a file whose ACL was narrowed by hand stays narrowed —
//! neither of which `MoveFileExW`, what `std::fs::rename` calls, would do. It
//! is called with no flags. `REPLACEFILE_IGNORE_MERGE_ERRORS` would let a save
//! succeed that had quietly not carried the ACL across, which is the one thing
//! this call is here to do; and the merge can only fail for want of rights over
//! the replacement file, which abeam has just created and so owns.
//!
//! **On Unix it is `rename(2)`, with the target's permission bits copied onto
//! the temporary file first**, and its owner and group where the platform
//! allows it (always for root, the group for a member of it). A script stays
//! executable and a group-writable file stays group-writable; the pad's 0600
//! is the pad's and is not imposed here. What a rename cannot carry is said
//! rather than papered over: extended attributes and ACLs set with `setfacl`
//! are the temporary file's, and somebody who is not root, saving a file
//! somebody else owns but that they may write, ends up owning it — which every
//! editor that saves by renaming does too.
//!
//! **On both, a hard link is broken.** The replacement is a new file under the
//! old name — `ReplaceFileW` swaps the data in under the old *name's* identity
//! and does nothing for any other name — so a file with a second name keeps
//! the old contents under that one, which quietly stops changing. Writing in
//! place would keep the link and give back every failure the temporary file
//! exists to avoid; nothing in a repository is usually hard-linked, and the
//! symptom names no cause on its own, which is why it is written here.
//!
//! **A read-only file is refused, not replaced.** On Windows that is the
//! attribute, on which `ReplaceFileW` would fail anyway, and an ACL denying
//! this user write access, on which — it turns out — it would not: the file
//! is replaced and the deny merged onto the new one. So the file is opened for
//! writing first, which writes nothing, and refused if that is. On Unix the
//! same hole is wider, because `rename(2)` asks nothing of the target — only
//! of the directory — so a file chmodded 0444 would be replaced without a
//! word, which is the pad store's own finding. So the mode is asked (no write
//! bit for anybody) and so is `access(2)` (no write permission for this user),
//! and either refuses. The first is what refuses root, for whom `access` says
//! yes to any mode.
//!
//! **A link is followed to the file it names**, and the save replaces that
//! file and leaves the link a link — provided the file is inside the workspace
//! under `crate::paths::under`, judged on the resolved path. A link to
//! somewhere outside is refused, and so is a link to nothing, rather than
//! being replaced by a regular file.
//!
//! ## The conflict check
//!
//! Immediately before the replacement the target is read again and compared,
//! byte for byte, with the [`Baseline`] the caller hands in — what the editor
//! was opened on, or what the last save wrote. A difference of any kind is
//! [`SaveError::Changed`] and nothing is written: a file that changed, a file
//! that was deleted, a file that appeared where the baseline says there was
//! none. [`Options::force`] skips the comparison, which is the second `Ctrl+S`
//! the files view offers; it is the only way past it, and a caller owes the
//! user a sentence saying what it will overwrite before it offers it.
//!
//! The comparison is not a lock. Between it and the replacement there is an
//! instant in which the agent can still write, and nothing here closes it: a
//! real lock would be a file nobody can save because some other process died
//! holding something.
//!
//! ## A new file never replaces anything
//!
//! A file that was not there when the comparison looked is put in place by an
//! operation that *fails* if a file has appeared since — `MoveFileExW` without
//! `MOVEFILE_REPLACE_EXISTING` on Windows, `link(2)` and then an `unlink` of
//! the temporary name on Unix — and that failure is [`SaveError::Changed`]. A
//! rename would have closed the comparison's window by replacing whatever
//! arrived in it, which for a new file is somebody else's whole file.
//!
//! One residue, on Unix: a filesystem that cannot make hard links — FAT on a
//! USB stick, some network mounts — says so, and there the placement falls
//! back to a rename and the window is the comparison's again.
//!
//! ## Retrying, on Windows only
//!
//! Windows refuses to replace — or to read, for the comparison — a file that
//! another process has open without sharing it, and on a working machine
//! somebody usually has: Defender scanning the file the agent just wrote, the
//! search indexer, OneDrive, a backup agent. Each holds it for a fraction of a
//! second. So three errors are retried, a few times, with short pauses that add
//! up to well under a second (`crate::disk::PAUSES`): `ERROR_ACCESS_DENIED` (5)
//! and `ERROR_SHARING_VIOLATION` (32), which is how a held file answers, and
//! `ERROR_UNABLE_TO_REMOVE_REPLACED` (1175), which `ReplaceFileW` returns when
//! it could not move the old file aside — both files then keep their names,
//! so trying again is safe. The comparison is made again before every attempt,
//! because the wait is exactly when somebody else might write.
//!
//! A refusal still standing at the end is asked once more what it is, because
//! `ERROR_ACCESS_DENIED` is also what an ACL that will never lift answers:
//! the file is opened for writing, with every kind of sharing offered. Denied
//! again is [`SaveError::Denied`], said as a permission; anything else is
//! [`SaveError::Busy`] — the text is still in the editor and the user can
//! press the key again in a moment.
//!
//! Unix has no equivalent — a file open elsewhere does not stop a rename — so
//! nothing there is retried.
//!
//! ## The one temporary file that is kept
//!
//! `ReplaceFileW` has two failures that happen *after* it has moved the old
//! file out of the way: `ERROR_UNABLE_TO_MOVE_REPLACEMENT` (1176) and
//! `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (1177). Either way the target's name is
//! free and the new text exists only in the temporary file — the old file is
//! gone after the first, and after the second is somewhere under a name Windows
//! chose. Removing the temporary file, as every other failure does, would
//! delete the file the user was saving. So the save finishes the job by
//! placing it into the empty name as a new file is placed — never over
//! anything that arrived meanwhile — and if even that fails the temporary file
//! is left where it is and [`SaveError::Stranded`] says where.
//!
//! A save finished that way has the text right and nothing else: the file is
//! the temporary one, so none of the ACL, attributes or creation time came
//! across, and after 1177 the old file is still on disk under a name nobody
//! asked for. [`Saved::notice`] says so rather than leaving it silent. Neither
//! failure has been seen here; both are documented outcomes of the call, and
//! keeping the temporary file is the difference between a failed save and a
//! deleted file.
//!
//! ## Directories made for a new file
//!
//! [`Options::create_dirs`] makes the directories a new file's path names, one
//! at a time, and remembers which of them it made — a directory that turned
//! out to be there already is somebody's and is not on the list. If the save
//! then does not happen, those are removed again, newest first and each only
//! while it is still empty, so a refused save leaves no trace of having tried.

use std::io;
use std::path::{Component, Path, PathBuf};

use super::baseline::Baseline;
use super::{PAUSES, Perms, Temp, temp_name};
use crate::paths;

/// How a save may go about it. The default is the ordinary `Ctrl+S`: compare
/// first, and never make a directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Skip the comparison and write over whatever is there: the second
    /// `Ctrl+S`, after the first was refused with [`SaveError::Changed`].
    pub force: bool,
    /// Make the directories a new file's path names, if they are missing. The
    /// files view's `a` uses it for a name with a `/` in it; an existing
    /// file's save does not, because a directory that vanished under an open
    /// file is a change on disk and not something to quietly put back.
    pub create_dirs: bool,
}

/// A save that happened.
#[derive(Debug)]
pub struct Saved {
    /// The file written — the resolved path, so a link's target and not the
    /// link.
    pub path: PathBuf,
    /// The target as the caller spelled it, which for a link is the link.
    ///
    /// Both spellings, because the files view needs both. A sentence about
    /// the file names it the way the user does. And the watcher reports a path
    /// the way it walked to it, which is not always the resolved one: a file
    /// reached through a linked *directory* is reported under the link's
    /// spelling where `notify` follows links, while a linked *file* changes
    /// only its target's entry. Telling this save's echo from somebody else's
    /// write is a comparison against either.
    pub requested: PathBuf,
    /// The bytes just written, for the next save to compare against.
    pub baseline: Baseline,
    /// Something about the save that went worse than asked and that the user
    /// should be told, though the text is safely written. `None` for every
    /// ordinary save.
    pub notice: Option<Notice>,
}

/// A save that wrote the text but could not keep everything else.
///
/// Only one shape of it today, and it is Windows': `ReplaceFileW` gave up
/// after moving the old file aside, and the save was finished by placing the
/// temporary file into the empty name. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// What `ReplaceFileW` said, in Windows' words.
    pub why: String,
    /// The old file is still on disk, under a name Windows chose
    /// (`ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`), rather than gone.
    pub left_behind: bool,
}

impl Notice {
    /// The sentence the files view shows after the save, naming `path`.
    pub fn message(&self, path: &Path) -> String {
        let path = path.display();
        let why = &self.why;
        if self.left_behind {
            format!(
                "Saved {path}, but not as asked: Windows could not swap it in place ({why}), so \
                 it is a new file without the old one's permissions or attributes, and the old \
                 version is still in that directory under a name Windows chose."
            )
        } else {
            format!(
                "Saved {path}, but not as asked: Windows could not swap it in place ({why}), so \
                 it is a new file without the old one's permissions or attributes."
            )
        }
    }
}

/// A save that did not happen, and in every case but one left the disk exactly
/// as it was. The exception is [`SaveError::Stranded`].
#[derive(Debug)]
pub enum SaveError {
    /// The target is not what the baseline says: changed, deleted, or
    /// created since. Nothing was written. [`Options::force`] overwrites.
    Changed,
    /// The file is marked read-only, or this user may not write it.
    ReadOnly,
    /// Windows refused for as long as the retries waited, and a probe for
    /// write access was refused too: a permission, not a moment. Windows only.
    Denied(io::Error),
    /// The path resolves to somewhere outside the workspace root.
    Outside,
    /// The path is a directory or a device, or does not name a file at all.
    NotAFile,
    /// The path is a link to something that does not exist.
    DanglingLink,
    /// A new file's directory does not exist, and [`Options::create_dirs`]
    /// was not set.
    NoDirectory,
    /// Windows kept refusing for as long as the retries waited, and the file
    /// was not refused as a permission when asked again: another program
    /// holding it open without sharing it. Windows only.
    Busy(io::Error),
    /// Anything else the platform refused, in its words.
    Io(io::Error),
    /// Windows moved the old file away and would not put the new one in its
    /// place, and the placement that should have finished the job failed too:
    /// the new text is at `temp`, kept. See the module docs.
    Stranded { temp: PathBuf, why: io::Error },
}

impl SaveError {
    /// The sentence the files view shows, naming `path` — the file as the user
    /// knows it, which for a link is the link.
    ///
    /// Worded for a file and not for the pad, whose sentences stay in the pad
    /// store: here the text is in an editor the user can see and go on using,
    /// so what they need is the cause and what will change it. No key is
    /// named, because which key does what is the files view's to say.
    pub fn message(&self, path: &Path) -> String {
        let path = path.display();
        match self {
            SaveError::Changed => format!(
                "Not saved: {path} has changed on disk since it was read, and saving now would \
                 write over that change."
            ),
            SaveError::ReadOnly => format!(
                "Not saved: {path} is read-only, and abeam will not replace a file it could not \
                 write in place."
            ),
            SaveError::Outside => {
                format!("Not saved: {path} resolves to somewhere outside this workspace.")
            }
            SaveError::NotAFile => format!("Not saved: {path} is not a regular file."),
            SaveError::DanglingLink => {
                format!("Not saved: {path} is a link to a file that does not exist.")
            }
            SaveError::NoDirectory => {
                format!("Not saved: the directory {path} would go in does not exist.")
            }
            SaveError::Denied(why) => format!(
                "Not saved: you do not have permission to change {path} ({why}). Your text is \
                 still here."
            ),
            SaveError::Busy(why) => format!(
                "Not saved: another program is holding {path} open ({why}). Your text is still \
                 here; try again in a moment."
            ),
            SaveError::Io(why) => format!("Not saved: {path}: {why}. Your text is still here."),
            SaveError::Stranded { temp, why } => format!(
                "Not saved: Windows moved {path} aside and would not put the new text in its \
                 place ({why}). The new text is in {}.",
                temp.display()
            ),
        }
    }
}

/// Write `bytes` as the file at `target`, inside `root`, if what is there now
/// is still `expect` — or anything at all, with [`Options::force`].
///
/// `target` is the file as the caller knows it; it is resolved here again
/// rather than trusted from [`open`](fn@crate::disk::open), because a link
/// can be repointed while somebody types. `bytes` is the whole file, as
/// [`Format::encode`](crate::disk::Format::encode) produced it.
///
/// See the module docs for what is kept, what is refused, what is retried and
/// the one failure that leaves something behind.
pub fn save(
    target: &Path,
    root: &Path,
    bytes: &[u8],
    expect: &Baseline,
    options: Options,
) -> Result<Saved, SaveError> {
    let root = paths::resolve_root(root);
    // Declared before the temporary file, so that on every way out the file is
    // dropped — and removed — before the directories it is in.
    let mut dest = destination(target, &root, expect, options)?;
    let name = dest.path.file_name().ok_or(SaveError::NotAFile)?.to_owned();
    let names = |n| dest.path.with_file_name(temp_name(&name, n));
    let temp = Temp::write(names, bytes, dest.perms).map_err(SaveError::Io)?;

    let mut pauses = PAUSES.iter();
    let notice = loop {
        match attempt(&temp, &dest.path, expect, options.force) {
            Ok(()) => {
                // In place: the name is the target's now, and there is nothing
                // left to remove.
                let _ = temp.release();
                break None;
            }
            // Each `return` below drops the temporary file, which removes it,
            // and then any directory this save made for it.
            Err(Attempt::Changed) => return Err(SaveError::Changed),
            Err(Attempt::Failed(why)) if platform::transient(&why) => match pauses.next() {
                Some(pause) => std::thread::sleep(*pause),
                None => return Err(platform::gave_up(&dest.path, why)),
            },
            Err(Attempt::Failed(why)) => return Err(SaveError::Io(why)),
            Err(Attempt::Vacated { why, left_behind }) => {
                break Some(finish(temp, &dest.path, why, left_behind)?);
            }
        }
    };

    dest.made.keep();
    Ok(Saved {
        path: dest.path,
        requested: target.to_path_buf(),
        baseline: Baseline::Present(bytes.into()),
        notice,
    })
}

/// The end of a save `ReplaceFileW` abandoned after moving the old file aside:
/// the temporary file, which is now the only copy of the text, placed into the
/// empty name as a new file is — never over anything that arrived since — and
/// kept where it is, whatever happens, if it cannot be.
fn finish(
    temp: Temp,
    target: &Path,
    why: io::Error,
    left_behind: bool,
) -> Result<Notice, SaveError> {
    let placed = platform::place(temp.path(), target);
    let kept = temp.release();
    match placed {
        Ok(()) => Ok(Notice {
            why: why.to_string(),
            left_behind,
        }),
        Err(again) => Err(SaveError::Stranded {
            temp: kept,
            why: io::Error::new(again.kind(), format!("{why}; then {again}")),
        }),
    }
}

/// Where a save will write, what the temporary file must be created as, and
/// which directories were made to hold it.
struct Dest {
    path: PathBuf,
    perms: Perms,
    made: Made,
}

/// The directories a save made for a new file, removed again — newest first,
/// and each only while it is still empty — when they are dropped without
/// being kept. `remove_dir` refuses a directory with anything in it, which is
/// what makes "only while empty" a property of the call rather than a check
/// with a window after it.
#[derive(Debug, Default)]
struct Made(Vec<PathBuf>);

impl Made {
    /// The save happened: the directories are the file's now.
    fn keep(&mut self) {
        self.0.clear();
    }
}

impl Drop for Made {
    fn drop(&mut self) {
        for dir in self.0.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// How one attempt at the replacement went.
enum Attempt {
    Changed,
    Failed(io::Error),
    /// `ReplaceFileW` moved the old file away and did not put the new one in
    /// its place; `left_behind` when the old file is still on disk under
    /// another name. Windows only; see the module docs.
    #[cfg_attr(
        unix,
        // `#[allow]` and not `#[expect]`: the condition is a `cfg`, so on the
        // platform this *is* used an expectation would be unfulfilled.
        allow(dead_code, reason = "constructed by the Windows replacement only")
    )]
    Vacated { why: io::Error, left_behind: bool },
}

/// Compare, and if the disk is still what was expected, replace.
fn attempt(temp: &Temp, target: &Path, expect: &Baseline, force: bool) -> Result<(), Attempt> {
    if !force && !expect.is_on_disk(target).map_err(Attempt::Failed)? {
        return Err(Attempt::Changed);
    }
    match std::fs::symlink_metadata(target) {
        Ok(_) => platform::replace(temp.path(), target),
        // Nothing there: a new file, or one the second `Ctrl+S` is putting
        // back after somebody deleted it. Placed so that a file that appeared
        // since the comparison is refused rather than replaced — see the
        // module docs — and that refusal is the comparison's own answer.
        Err(why) if why.kind() == io::ErrorKind::NotFound => {
            platform::place(temp.path(), target).map_err(|why| match why.kind() {
                io::ErrorKind::AlreadyExists => Attempt::Changed,
                _ => Attempt::Failed(why),
            })
        }
        Err(why) => Err(Attempt::Failed(why)),
    }
}

/// The file `target` names, resolved and checked, before anything is written.
fn destination(
    target: &Path,
    root: &Path,
    expect: &Baseline,
    options: Options,
) -> Result<Dest, SaveError> {
    match std::fs::symlink_metadata(target) {
        Ok(_) => existing(target, root),
        Err(why) if why.kind() == io::ErrorKind::NotFound => {
            // Deleted under the editor. Said now, before a directory is made
            // for it, rather than after the comparison finds the same thing.
            if !options.force && matches!(expect, Baseline::Present(_)) {
                return Err(SaveError::Changed);
            }
            created(target, root, options.create_dirs)
        }
        Err(why) => Err(SaveError::Io(why)),
    }
}

/// A file that is there: follow it to what it names, and refuse anything that
/// is not a writable regular file inside the workspace.
fn existing(target: &Path, root: &Path) -> Result<Dest, SaveError> {
    let resolved = paths::resolve(target).map_err(|why| match why.kind() {
        // `symlink_metadata` found something and resolving it found nothing:
        // a link whose target is gone.
        io::ErrorKind::NotFound => SaveError::DanglingLink,
        _ => SaveError::Io(why),
    })?;
    if !paths::under(root, &resolved) {
        return Err(SaveError::Outside);
    }
    let meta = std::fs::metadata(&resolved).map_err(SaveError::Io)?;
    if !meta.is_file() {
        return Err(SaveError::NotAFile);
    }
    if read_only(&resolved, &meta) {
        return Err(SaveError::ReadOnly);
    }
    Ok(Dest {
        path: resolved,
        perms: Perms::Like(meta),
        made: Made::default(),
    })
}

/// A file that is not there yet: find the directory it goes in, inside the
/// workspace, making it if asked to.
///
/// The directory is judged before anything is made, on the nearest part of it
/// that exists, resolved; and what is to be made under that must be plain
/// names — no `..`, nothing that could climb back out of the directory just
/// judged. It is judged again, resolved, once it exists, because that is the
/// directory the file is really going into. Each directory is made on its own
/// and remembered only if this call made it; a refusal on the way out of here
/// removes them again, as a failed save later does.
fn created(target: &Path, root: &Path, create_dirs: bool) -> Result<Dest, SaveError> {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(SaveError::NotAFile);
    };
    let Some(have) = parent.ancestors().find(|dir| dir.is_dir()) else {
        return Err(SaveError::NoDirectory);
    };
    let base = paths::resolve(have).map_err(SaveError::Io)?;
    if !paths::under(root, &base) {
        return Err(SaveError::Outside);
    }
    let mut made = Made::default();
    if have != parent {
        if !create_dirs {
            return Err(SaveError::NoDirectory);
        }
        let rest = parent.strip_prefix(have).map_err(|_| SaveError::Outside)?;
        if !rest
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(SaveError::Outside);
        }
        let mut at = have.to_path_buf();
        for part in rest.components() {
            at.push(part);
            match std::fs::create_dir(&at) {
                Ok(()) => made.0.push(at.clone()),
                // Somebody else's, made in the meantime: used, not owned.
                Err(why) if why.kind() == io::ErrorKind::AlreadyExists && at.is_dir() => {}
                Err(why) => return Err(SaveError::Io(why)),
            }
        }
    }
    let dir = paths::resolve(parent).map_err(SaveError::Io)?;
    if !paths::under(root, &dir) {
        return Err(SaveError::Outside);
    }
    Ok(Dest {
        path: dir.join(name),
        perms: Perms::Default,
        made,
    })
}

/// The attribute, or an ACL that will not let this user write the file.
///
/// The second is asked by opening the file for writing — which writes nothing
/// — because `ReplaceFileW` does not ask it: a file whose ACL denies its owner
/// `WRITE_DATA` is replaced without complaint, the deny merged onto the new
/// file, which is Unix's 0444 finding over again. A refusal that is somebody
/// else's handle rather than a permission is not read-only, and is left to
/// the retries.
#[cfg(windows)]
fn read_only(path: &Path, meta: &std::fs::Metadata) -> bool {
    use winapi::shared::winerror::ERROR_ACCESS_DENIED;

    meta.permissions().readonly()
        || std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .is_err_and(|why| why.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32))
}

/// No write bit for anybody, or no write permission for this user. See the
/// module docs for why both, and why it is asked at all when `rename(2)` would
/// not.
#[cfg(unix)]
fn read_only(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::ffi::OsStrExt;

    if meta.permissions().readonly() {
        return true;
    }
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and
    // `access` reads it and nothing else.
    if unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0 {
        return false;
    }
    // Only a refusal that is about permission. A file that vanished in the
    // last microsecond is the comparison's business, not this.
    matches!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::EACCES | libc::EPERM | libc::EROFS)
    )
}

#[cfg(windows)]
mod platform {
    //! `ReplaceFileW` and `MoveFileExW`, declared here by hand.
    //!
    //! By hand because winapi 0.3.9 — the version in this tree — declares
    //! `ReplaceFileW` without its return type, so the one call in this module
    //! whose failure must be seen would have it thrown away. `MoveFileExW` is
    //! correct there, and is declared here anyway: it lives in `winbase`, and
    //! enabling that module for it would put the broken declaration back in the
    //! build beside this one. The types and the error codes are winapi's; the
    //! declarations are the SDK's (`winbase.h`).

    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, Prefix};

    use winapi::shared::minwindef::{BOOL, DWORD, LPVOID};
    use winapi::shared::winerror::{
        ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_UNABLE_TO_MOVE_REPLACEMENT,
        ERROR_UNABLE_TO_MOVE_REPLACEMENT_2, ERROR_UNABLE_TO_REMOVE_REPLACED,
    };
    use winapi::um::winnt::LPCWSTR;

    use super::{Attempt, SaveError};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: LPCWSTR,
            replacement: LPCWSTR,
            backup: LPCWSTR,
            flags: DWORD,
            exclude: LPVOID,
            reserved: LPVOID,
        ) -> BOOL;
        fn MoveFileExW(existing: LPCWSTR, new: LPCWSTR, flags: DWORD) -> BOOL;
    }

    /// Put `temp` in `target`'s place, keeping `target`'s ACL, attributes and
    /// creation time. No backup file and no flags; see the module docs.
    pub(super) fn replace(temp: &Path, target: &Path) -> Result<(), Attempt> {
        let replaced = wide(target);
        let replacement = wide(temp);
        // SAFETY: both strings are NUL-terminated and outlive the call; the
        // backup name, `lpExclude` and `lpReserved` are documented as
        // optional and null.
        let done = unsafe {
            ReplaceFileW(
                replaced.as_ptr(),
                replacement.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if done != 0 {
            return Ok(());
        }
        let why = io::Error::last_os_error();
        match why.raw_os_error().map(|code| code as DWORD) {
            Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT) => Err(Attempt::Vacated {
                why,
                left_behind: false,
            }),
            Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2) => Err(Attempt::Vacated {
                why,
                left_behind: true,
            }),
            _ => Err(Attempt::Failed(why)),
        }
    }

    /// Move `temp` to `target` only if nothing is there: `MoveFileExW` with
    /// no flags, and so without `MOVEFILE_REPLACE_EXISTING`. A file that is
    /// there fails it with `ERROR_ALREADY_EXISTS`, which `std` reads as
    /// [`io::ErrorKind::AlreadyExists`].
    pub(super) fn place(temp: &Path, target: &Path) -> io::Result<()> {
        let from = wide(temp);
        let to = wide(target);
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let done = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
        if done != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Whether a failure is somebody else holding the file for a moment. See
    /// the module docs for the three.
    pub(super) fn transient(why: &io::Error) -> bool {
        matches!(
            why.raw_os_error().map(|code| code as DWORD),
            Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_UNABLE_TO_REMOVE_REPLACED)
        )
    }

    /// What a refusal still standing after every retry is: asked once more of
    /// `target`, by opening it for writing with every kind of sharing offered
    /// — which another program's handle cannot refuse as a permission, and an
    /// ACL can. Opening for writing writes nothing.
    pub(super) fn gave_up(target: &Path, why: io::Error) -> SaveError {
        let denied = |why: &io::Error| why.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
        if denied(&why)
            && std::fs::OpenOptions::new()
                .write(true)
                .open(target)
                .is_err_and(|probe| denied(&probe))
        {
            return SaveError::Denied(why);
        }
        SaveError::Busy(why)
    }

    /// `path` as the NUL-terminated UTF-16 a `W` function takes, made verbatim
    /// when it is long.
    ///
    /// `std` does the same for every path it hands Windows, and this call does
    /// not go through `std`: a path past `MAX_PATH` — which a repository's
    /// `node_modules` reaches without trying — fails a plain `W` call unless
    /// the program has opted into long paths, and abeam has not. Verbatim
    /// turns off Windows' own clean-up of the spelling, which is safe only
    /// because what arrives here is `crate::paths::resolve`'s answer: no `/`,
    /// no `.` or `..`, nothing left to clean. The threshold is `std`'s — 248
    /// UTF-16 units, `MAX_PATH` less room for an 8.3 name — so a path is
    /// spelled one way to both.
    pub(super) fn wide(path: &Path) -> Vec<u16> {
        let plain: Vec<u16> = path.as_os_str().encode_wide().collect();
        let mut out: Vec<u16> = if plain.len() < 248 {
            plain
        } else {
            match path.components().next() {
                Some(Component::Prefix(prefix)) => match prefix.kind() {
                    Prefix::Disk(_) => r"\\?\".encode_utf16().chain(plain).collect(),
                    // `\\server\share\…` is `\\?\UNC\server\share\…`: one of
                    // the two leading separators gives way to the prefix.
                    Prefix::UNC(..) => {
                        r"\\?\UNC".encode_utf16().chain(plain.into_iter().skip(1)).collect()
                    }
                    _ => plain,
                },
                _ => plain,
            }
        };
        out.push(0);
        out
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::path::Path;

    use super::{Attempt, SaveError};

    /// `rename(2)`. The bits were copied onto `temp` when it was written.
    pub(super) fn replace(temp: &Path, target: &Path) -> Result<(), Attempt> {
        std::fs::rename(temp, target).map_err(Attempt::Failed)
    }

    /// Move `temp` to `target` only if nothing is there: `link(2)`, which
    /// fails with `EEXIST` — `AlreadyExists` — in one indivisible step if a
    /// file has appeared, and then the temporary name taken away.
    ///
    /// A filesystem that cannot make hard links says so with one of a short
    /// list of errors, and there this falls back to a rename, which is the
    /// residue the module docs name. If the temporary name cannot be taken
    /// away after the link, the file is saved and has a second name beside it,
    /// unmistakably abeam's; that is left rather than the save reported as
    /// failed.
    pub(super) fn place(temp: &Path, target: &Path) -> io::Result<()> {
        match std::fs::hard_link(temp, target) {
            Ok(()) => {
                let _ = std::fs::remove_file(temp);
                Ok(())
            }
            Err(why) if no_links(&why) => std::fs::rename(temp, target),
            Err(why) => Err(why),
        }
    }

    /// The errors `link(2)` answers with on a filesystem that has no hard
    /// links — FAT's `EPERM`, a network mount's `EOPNOTSUPP` — rather than
    /// because of anything about these two names.
    fn no_links(why: &io::Error) -> bool {
        why.raw_os_error().is_some_and(|code| {
            [libc::EPERM, libc::EOPNOTSUPP, libc::ENOTSUP, libc::ENOSYS].contains(&code)
        })
    }

    /// Nothing: a file open elsewhere does not stop a rename here.
    pub(super) fn transient(_why: &io::Error) -> bool {
        false
    }

    /// Never reached, since nothing here is retried; the same answer as
    /// Windows' for a refusal that was not a permission.
    pub(super) fn gave_up(_target: &Path, why: io::Error) -> SaveError {
        SaveError::Busy(why)
    }
}

#[cfg(test)]
mod tests {
    // The waits are Windows' tests'; Unix has nothing that waits.
    #[cfg(windows)]
    use std::time::Duration;

    use super::*;
    use crate::disk::open::{Eol, Format, open};
    use crate::testutil::TempDir;

    /// Every name in `dir`, sorted, so that "and nothing else is there" is one
    /// assertion.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read the fixture")
            .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn present(bytes: &[u8]) -> Baseline {
        Baseline::Present(bytes.to_vec().into())
    }

    /// The whole of what the files view will do, end to end: open a CRLF file
    /// with a mark, change a word, save, and find exactly that on disk.
    #[test]
    fn a_file_opened_edited_and_saved_comes_back_with_its_endings_and_its_mark() {
        let dir = TempDir::new("disk-save-crlf");
        let path = dir.write("notes.md", b"\xEF\xBB\xBF# Notes\r\n\r\nthe old text\r\n");
        let opened = open(&path, dir.path()).expect("editable");
        assert_eq!(opened.format, Format { eol: Eol::CrLf, bom: true });

        let text = opened.text.replace("old", "new");
        let bytes = opened.format.encode(&text);
        let saved = save(&path, dir.path(), &bytes, &opened.baseline, Options::default())
            .expect("nothing has touched it");
        assert_eq!(
            std::fs::read(&path).expect("read it back"),
            b"\xEF\xBB\xBF# Notes\r\n\r\nthe new text\r\n"
        );
        assert_eq!(saved.baseline, present(&bytes));
        assert!(paths::same_dir(&saved.path, &path));
        assert_eq!(listing(dir.path()), ["notes.md"], "and nothing beside it");

        // The baseline a save hands back is what the next one compares with,
        // so a run of saves does not start refusing itself after the first.
        let mut baseline = saved.baseline;
        for word in ["one", "two", "three"] {
            let bytes = opened.format.encode(&text.replace("new", word));
            baseline = save(&path, dir.path(), &bytes, &baseline, Options::default())
                .expect("an own save is not a conflict")
                .baseline;
        }
        assert!(baseline.is_on_disk(&path).expect("readable"));
    }

    #[test]
    fn a_file_changed_on_disk_is_refused_and_a_forced_save_overwrites_it() {
        let dir = TempDir::new("disk-save-changed");
        let path = dir.write("a.md", b"mine\n");
        let opened = open(&path, dir.path()).expect("editable");

        // The agent rewrites it to the same length while the user types:
        // what a stamp of length and time could miss.
        std::fs::write(&path, b"them\n").expect("somebody else's write");
        let edited = b"mine, edited\n";
        let refused = save(&path, dir.path(), edited, &opened.baseline, Options::default());
        assert!(matches!(refused, Err(SaveError::Changed)), "{refused:?}");
        assert_eq!(std::fs::read(&path).expect("read"), b"them\n", "nothing was written");
        assert_eq!(listing(dir.path()), ["a.md"], "and the temporary file is gone");

        let force = Options { force: true, ..Options::default() };
        save(&path, dir.path(), b"mine, edited\n", &opened.baseline, force).expect("forced");
        assert_eq!(std::fs::read(&path).expect("read"), b"mine, edited\n");
        assert_eq!(listing(dir.path()), ["a.md"]);
    }

    #[test]
    fn a_file_deleted_under_the_editor_is_a_change_and_is_not_put_back_unasked() {
        let dir = TempDir::new("disk-save-deleted");
        let path = dir.write("a.md", b"text\n");
        let opened = open(&path, dir.path()).expect("editable");
        std::fs::remove_file(&path).expect("the agent tidies up");

        let refused = save(&path, dir.path(), b"text\n", &opened.baseline, Options::default());
        assert!(matches!(refused, Err(SaveError::Changed)), "{refused:?}");
        assert!(listing(dir.path()).is_empty(), "nothing was created");

        let force = Options { force: true, ..Options::default() };
        save(&path, dir.path(), b"text\n", &opened.baseline, force).expect("put back");
        assert_eq!(std::fs::read(&path).expect("read"), b"text\n");
    }

    #[test]
    fn a_new_file_that_appeared_meanwhile_is_a_change_too() {
        let dir = TempDir::new("disk-save-appeared");
        let path = dir.path().join("new.md");
        let saved = save(&path, dir.path(), b"first\n", &Baseline::Absent, Options::default())
            .expect("a new file");
        assert_eq!(saved.baseline, present(b"first\n"));
        assert_eq!(listing(dir.path()), ["new.md"]);

        // Somebody else made the same name between the editor opening and the
        // save: an `Absent` baseline is not a licence to replace it.
        let other = dir.path().join("other.md");
        std::fs::write(&other, b"theirs\n").expect("somebody else's file");
        let refused = save(&other, dir.path(), b"mine\n", &Baseline::Absent, Options::default());
        assert!(matches!(refused, Err(SaveError::Changed)), "{refused:?}");
        assert_eq!(std::fs::read(&other).expect("read"), b"theirs\n");
        let force = Options { force: true, ..Options::default() };
        save(&other, dir.path(), b"mine\n", &Baseline::Absent, force).expect("forced");
        assert_eq!(std::fs::read(&other).expect("read"), b"mine\n");
        assert_eq!(listing(dir.path()), ["new.md", "other.md"]);
    }

    #[test]
    fn a_new_files_directory_is_made_only_when_asked_and_only_inside() {
        let dir = TempDir::new("disk-save-dirs");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("a repository");
        let nested = root.join("notes").join("2026").join("today.md");

        let refused = save(&nested, &root, b"x\n", &Baseline::Absent, Options::default());
        assert!(matches!(refused, Err(SaveError::NoDirectory)), "{refused:?}");
        assert!(listing(&root).is_empty(), "no directory was made unasked");

        let make = Options { create_dirs: true, ..Options::default() };
        save(&nested, &root, b"x\n", &Baseline::Absent, make).expect("made");
        assert_eq!(std::fs::read(&nested).expect("read"), b"x\n");
        assert_eq!(listing(&root.join("notes").join("2026")), ["today.md"]);

        // A path that climbs out, through a directory that does not exist yet
        // and through one that does.
        let climbing = root.join("new").join("..").join("..").join("escaped.md");
        let refused = save(&climbing, &root, b"x\n", &Baseline::Absent, make);
        assert!(matches!(refused, Err(SaveError::Outside)), "{refused:?}");
        let climbed = root.join("..").join("escaped.md");
        let refused = save(&climbed, &root, b"x\n", &Baseline::Absent, make);
        assert!(matches!(refused, Err(SaveError::Outside)), "{refused:?}");
        assert_eq!(listing(dir.path()), ["repo"], "nothing was written outside");
        assert_eq!(listing(&root), ["notes"], "and nothing stray inside");
    }

    #[test]
    fn what_is_outside_the_workspace_or_not_a_file_is_refused_before_anything_is_written() {
        let dir = TempDir::new("disk-save-where");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("sub")).expect("a repository");
        let outside = dir.path().join("secret.md");
        std::fs::write(&outside, b"not the work\n").expect("a file outside");

        let seen = present(b"not the work\n");
        let refused = save(&outside, &root, b"x\n", &seen, Options::default());
        assert!(matches!(refused, Err(SaveError::Outside)), "{refused:?}");
        let force = Options { force: true, ..Options::default() };
        let refused = save(&outside, &root, b"x\n", &Baseline::Absent, force);
        assert!(matches!(refused, Err(SaveError::Outside)), "force is not a way out: {refused:?}");
        assert_eq!(std::fs::read(&outside).expect("read"), b"not the work\n");

        let refused = save(&root.join("sub"), &root, b"x\n", &Baseline::Absent, force);
        assert!(matches!(refused, Err(SaveError::NotAFile)), "{refused:?}");
        assert_eq!(listing(&root), ["sub"]);
        assert_eq!(listing(dir.path()), ["repo", "secret.md"]);
    }

    #[test]
    fn a_read_only_file_is_refused_and_left_alone() {
        let dir = TempDir::new("disk-save-readonly");
        let path = dir.write("locked.md", b"leave me\n");
        let writable = std::fs::metadata(&path).expect("stat").permissions();
        let mut perms = writable.clone();
        #[cfg(unix)]
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o444);
        #[cfg(windows)]
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).expect("make it read-only");

        let force = Options { force: true, ..Options::default() };
        for options in [Options::default(), force] {
            let refused = save(&path, dir.path(), b"changed\n", &present(b"leave me\n"), options);
            assert!(matches!(refused, Err(SaveError::ReadOnly)), "{refused:?}");
        }
        assert_eq!(std::fs::read(&path).expect("read"), b"leave me\n");
        assert_eq!(listing(dir.path()), ["locked.md"]);
        assert!(std::fs::metadata(&path).expect("stat").permissions().readonly());

        // Writable again, so the fixture can be removed on every platform.
        std::fs::set_permissions(&path, writable).expect("make it writable");
    }

    /// A link inside the workspace to a file inside it is saved through; one
    /// to a file outside it, or to nothing, is refused. Skipped, saying so,
    /// where this machine will not make a symlink.
    #[test]
    fn a_link_is_saved_through_to_its_target_only_when_that_is_inside() {
        let dir = TempDir::new("disk-save-link");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("a repository");
        let real = root.join("real.md");
        std::fs::write(&real, b"the work\n").expect("a file inside");
        let outside = dir.path().join("secret.md");
        std::fs::write(&outside, b"not the work\n").expect("a file outside");

        let link = crate::testutil::symlink_file;
        if !link(&real, &root.join("in.md"))
            || !link(&outside, &root.join("out.md"))
            || !link(&root.join("gone.md"), &root.join("dangling.md"))
        {
            eprintln!("skipped: this machine will not make a symlink");
            return;
        }

        let through = root.join("in.md");
        let saved = save(&through, &root, b"edited\n", &present(b"the work\n"), Options::default())
            .expect("a link to the work");
        assert!(paths::same_dir(&saved.path, &real), "the target was written");
        assert_eq!(std::fs::read(&real).expect("read"), b"edited\n");
        assert!(
            std::fs::symlink_metadata(&through).expect("stat").file_type().is_symlink(),
            "and the link is still a link"
        );

        let force = Options { force: true, ..Options::default() };
        let refused = save(&root.join("out.md"), &root, b"x\n", &Baseline::Absent, force);
        assert!(matches!(refused, Err(SaveError::Outside)), "{refused:?}");
        assert_eq!(std::fs::read(&outside).expect("read"), b"not the work\n");
        let refused = save(&root.join("dangling.md"), &root, b"x\n", &Baseline::Absent, force);
        assert!(matches!(refused, Err(SaveError::DanglingLink)), "{refused:?}");
        assert!(!root.join("gone.md").exists(), "nothing was made at the far end");
        assert_eq!(listing(&root), ["dangling.md", "in.md", "out.md", "real.md"]);
    }

    /// The link test above, with junctions: they need no privilege, so on a
    /// Windows machine that will not make a symlink — this one, as it was
    /// written — this is what proves a path is judged by where it resolves.
    /// `crate::paths`'s own junction test is the precedent, and the same
    /// `mklink /J`.
    #[test]
    #[cfg(windows)]
    fn a_junction_is_followed_before_the_workspace_is_asked() {
        let junction = |link: &Path, target: &Path| {
            std::process::Command::new("cmd.exe")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdin(std::process::Stdio::null())
                .output()
                .is_ok_and(|out| out.status.success())
        };
        let dir = TempDir::new("disk-save-junction");
        let root = dir.path().join("repo");
        let real = root.join("real");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&real).expect("a directory inside");
        std::fs::create_dir_all(&elsewhere).expect("a directory outside");
        std::fs::write(real.join("notes.md"), b"the work\n").expect("a file inside");
        std::fs::write(elsewhere.join("secret.md"), b"not the work\n").expect("a file outside");
        assert!(
            junction(&root.join("out"), &elsewhere) && junction(&root.join("in"), &real),
            "this test needs `mklink /J`, which needs no elevation"
        );

        // Out of the workspace through a junction: refused to open, to save,
        // and to create in, with or without directories to make.
        let secret = root.join("out").join("secret.md");
        assert_eq!(open(&secret, &root).map(|_| ()), Err(crate::disk::Refusal::Outside));
        let force = Options { force: true, create_dirs: true };
        for target in [
            secret.clone(),
            root.join("out").join("new.md"),
            root.join("out").join("sub").join("new.md"),
        ] {
            let refused = save(&target, &root, b"x\n", &Baseline::Absent, force);
            assert!(matches!(refused, Err(SaveError::Outside)), "{target:?}: {refused:?}");
        }
        assert_eq!(listing(&elsewhere), ["secret.md"], "nothing was made outside");
        assert_eq!(std::fs::read(elsewhere.join("secret.md")).expect("read"), b"not the work\n");

        // Inside the workspace through a junction: the real file is written,
        // and the junction is still a junction.
        let through = root.join("in").join("notes.md");
        let opened = open(&through, &root).expect("a junction to the work");
        assert!(paths::same_dir(&opened.path, &real.join("notes.md")));
        save(&through, &root, b"edited\n", &opened.baseline, Options::default())
            .expect("saved through the junction");
        assert_eq!(std::fs::read(real.join("notes.md")).expect("read"), b"edited\n");
        assert!(std::fs::read_link(root.join("in")).is_ok(), "still a junction");
        assert_eq!(listing(&real), ["notes.md"]);
    }

    #[test]
    fn every_failure_says_which_file_in_a_sentence() {
        let path = Path::new("docs").join("README.md");
        let why = || io::Error::new(io::ErrorKind::PermissionDenied, "Access is denied.");
        for failure in [
            SaveError::Changed,
            SaveError::ReadOnly,
            SaveError::Outside,
            SaveError::NotAFile,
            SaveError::DanglingLink,
            SaveError::NoDirectory,
            SaveError::Denied(why()),
            SaveError::Busy(why()),
            SaveError::Io(why()),
            SaveError::Stranded {
                temp: PathBuf::from(".README.md.1.abeam-save~"),
                why: why(),
            },
        ] {
            let said = failure.message(&path);
            assert!(said.starts_with("Not saved: "), "{said}");
            assert!(said.contains(&path.display().to_string()), "{said}");
            assert!(said.ends_with('.'), "{said}");
        }
        let stranded = SaveError::Stranded {
            temp: PathBuf::from(".README.md.1.abeam-save~"),
            why: why(),
        };
        assert!(stranded.message(&path).contains(".README.md.1.abeam-save~"), "says where");
        assert!(SaveError::Denied(why()).message(&path).contains("permission"));
        assert!(SaveError::Busy(why()).message(&path).contains("another program"));
    }

    /// What has a temporary file's name — a stranded save's only copy, a
    /// killed process's leftover under a reused id, anybody's file — is passed
    /// over, never emptied and never removed, on a save that works and on one
    /// that is refused.
    #[test]
    fn a_file_on_the_temporary_name_survives_a_save_byte_for_byte() {
        let dir = TempDir::new("disk-save-squatter");
        let path = dir.write("a.md", b"original\n");
        let name = path.file_name().expect("a name").to_owned();
        let squatter = path.with_file_name(temp_name(&name, 0));
        std::fs::write(&squatter, b"the only copy of something\n").expect("on the name");

        let opened = open(&path, dir.path()).expect("editable");
        let saved = save(&path, dir.path(), b"edited\n", &opened.baseline, Options::default())
            .expect("saved under another temporary name");
        assert_eq!(std::fs::read(&path).expect("read"), b"edited\n");
        assert_eq!(std::fs::read(&squatter).expect("still there"), b"the only copy of something\n");

        std::fs::write(&path, b"theirs\n").expect("somebody else's write");
        let refused = save(&path, dir.path(), b"mine\n", &saved.baseline, Options::default());
        assert!(matches!(refused, Err(SaveError::Changed)), "{refused:?}");
        assert_eq!(std::fs::read(&squatter).expect("still there"), b"the only copy of something\n");
        let mut names = listing(dir.path());
        names.retain(|n| n != "a.md");
        assert_eq!(names.len(), 1, "only the squatter beside the file: {names:?}");
    }

    /// The primitive a new file is placed with: it never replaces anything.
    #[test]
    fn placing_a_new_file_onto_one_that_appeared_fails_and_leaves_both() {
        let dir = TempDir::new("disk-save-place");
        let temp = dir.write(".new.md.1.abeam-save~", b"mine\n");
        let target = dir.write("new.md", b"theirs, made a moment ago\n");

        let why = platform::place(&temp, &target).expect_err("there is a file there");
        assert_eq!(why.kind(), io::ErrorKind::AlreadyExists, "{why}");
        assert_eq!(std::fs::read(&target).expect("read"), b"theirs, made a moment ago\n");
        assert_eq!(std::fs::read(&temp).expect("read"), b"mine\n");

        std::fs::remove_file(&target).expect("make room");
        platform::place(&temp, &target).expect("nothing there now");
        assert_eq!(std::fs::read(&target).expect("read"), b"mine\n");
        assert_eq!(listing(dir.path()), ["new.md"], "and the temporary name is gone");
    }

    /// The end of a save `ReplaceFileW` abandoned half way, driven by hand —
    /// the failure itself cannot be arranged — on both platforms, because the
    /// placement it finishes with is both platforms'.
    #[test]
    fn a_save_abandoned_half_way_is_finished_or_its_text_is_kept() {
        let dir = TempDir::new("disk-save-finish");
        let target = dir.path().join("a.md");
        let name = target.file_name().expect("a name").to_owned();
        let names = |n| target.with_file_name(temp_name(&name, n));
        let gave_up = || io::Error::other("the replacement could not be moved");

        // The name is empty, as Windows leaves it: finished, and said.
        let temp = Temp::write(names, b"saved\n", Perms::Default).expect("a temporary file");
        let notice = finish(temp, &target, gave_up(), true).expect("finished");
        assert_eq!(std::fs::read(&target).expect("read"), b"saved\n");
        assert!(notice.left_behind);
        let said = notice.message(&target);
        assert!(said.starts_with("Saved "), "{said}");
        assert!(said.contains("could not be moved") && said.contains("old version"), "{said}");
        assert!(!Notice { left_behind: false, ..notice }.message(&target).contains("old version"));

        // Somebody's file arrived in the empty name first: it is not replaced,
        // and the text stays where it is, named.
        let temp = Temp::write(names, b"second\n", Perms::Default).expect("a temporary file");
        let at = temp.path().to_path_buf();
        let Err(SaveError::Stranded { temp: kept, .. }) = finish(temp, &target, gave_up(), false)
        else {
            panic!("a placement onto a file that is there must strand, not replace");
        };
        assert_eq!(kept, at);
        assert_eq!(std::fs::read(&kept).expect("kept"), b"second\n");
        assert_eq!(std::fs::read(&target).expect("read"), b"saved\n");
    }

    /// Directories made for a new file are taken away again when the save
    /// does not happen — only those, and only while empty.
    #[test]
    fn directories_made_for_a_save_that_fails_are_removed_and_no_others() {
        let dir = TempDir::new("disk-save-made");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("had")).expect("a directory that was there");
        // A name the target can have and its temporary file cannot: every
        // platform here holds a name to 255 units, and the temporary one is
        // the name plus its scaffolding. So the directories are made and the
        // save then fails.
        let long = format!("{}.md", "x".repeat(245));
        let target = root.join("had").join("new").join("deeper").join(&long);
        let make = Options { create_dirs: true, ..Options::default() };

        let refused = save(&target, &root, b"x\n", &Baseline::Absent, make);
        assert!(matches!(refused, Err(SaveError::Io(_))), "{refused:?}");
        assert!(listing(&root.join("had")).is_empty(), "the made directories are gone");
        assert!(root.join("had").is_dir(), "and the one that was there is not");

        // A save that works keeps them.
        let fine = root.join("had").join("new").join("deeper").join("fine.md");
        let saved = save(&fine, &root, b"x\n", &Baseline::Absent, make).expect("saved");
        assert_eq!(listing(&root.join("had").join("new").join("deeper")), ["fine.md"]);
        assert_eq!(saved.requested, fine, "the target as it was asked for");
        assert_eq!(saved.notice, None);
    }

    /// The bits a file had are the bits it has after a save, and a save does
    /// not impose the pad's 0600 on somebody's project.
    #[test]
    #[cfg(unix)]
    fn a_save_keeps_the_permission_bits_the_file_had() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("disk-save-mode");
        for mode in [0o755, 0o640, 0o664] {
            let path = dir.write("script.sh", b"#!/bin/sh\n");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .expect("chmod");
            let seen = present(b"#!/bin/sh\n");
            save(&path, dir.path(), b"#!/bin/sh\nexit 0\n", &seen, Options::default())
                .expect("saved");
            let now = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o7777;
            assert_eq!(now, mode, "{mode:o} became {now:o}");
            std::fs::remove_file(&path).expect("tidy");
        }
    }

    /// A file held open with no sharing — what a scanner, the indexer or
    /// OneDrive does for a moment — fails the save cleanly once the retries
    /// run out: within the budget, with the target untouched and the
    /// temporary file gone. Both with the comparison, which is what meets the
    /// lock first, and forced, which goes straight to `ReplaceFileW`.
    #[test]
    #[cfg(windows)]
    fn a_file_held_open_by_another_program_is_busy_and_untouched() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new("disk-save-held");
        let path = dir.write("held.md", b"original\n");
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("hold it open with no sharing");

        let force = Options { force: true, ..Options::default() };
        for options in [Options::default(), force] {
            let started = std::time::Instant::now();
            let refused = save(&path, dir.path(), b"new\n", &present(b"original\n"), options);
            let took = started.elapsed();
            assert!(matches!(refused, Err(SaveError::Busy(_))), "{options:?}: {refused:?}");
            assert!(took < Duration::from_secs(1), "the retries took {took:?}");
            assert!(took >= PAUSES.iter().sum::<Duration>(), "and did retry: {took:?}");
            assert_eq!(listing(dir.path()), ["held.md"], "the temporary file is gone");
        }

        drop(held);
        assert_eq!(std::fs::read(&path).expect("read"), b"original\n", "untouched");
        save(&path, dir.path(), b"new\n", &present(b"original\n"), Options::default())
            .expect("and saves once it is let go");
    }

    /// An ACL denying write access is a read-only file, refused before
    /// anything is written — `ReplaceFileW` itself would replace it without a
    /// word, which is what this test found first — and a refusal still
    /// standing after the retries is asked again and called a permission
    /// rather than "busy". `icacls` because it needs no privilege on a file
    /// one owns.
    #[test]
    #[cfg(windows)]
    fn a_file_whose_acl_denies_writing_is_a_permission_and_not_a_moment() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new("disk-save-acl");
        let path = dir.write("guarded.md", b"original\n");
        let user = std::env::var("USERNAME").expect("a user name");
        let icacls = |args: &[&str]| {
            std::process::Command::new("icacls")
                .arg(&path)
                .args(args)
                .stdin(std::process::Stdio::null())
                .output()
                .is_ok_and(|out| out.status.success())
        };
        // Writing data and appending, and not `(W)`: that set carries
        // `SYNCHRONIZE`, which every open asks for, so it would deny reading
        // the file too and this would be a test of something else.
        assert!(icacls(&["/deny", &format!("{user}:(WD,AD)")]), "icacls needs no elevation here");

        let force = Options { force: true, ..Options::default() };
        let refused = save(&path, dir.path(), b"new\n", &present(b"original\n"), force);
        let denied = || io::Error::from_raw_os_error(5);
        let probed = platform::gave_up(&path, denied());
        let reset = icacls(&["/reset"]);
        assert!(reset, "the fixture's ACL was put back");
        assert!(matches!(refused, Err(SaveError::ReadOnly)), "{refused:?}");
        assert_eq!(std::fs::read(&path).expect("read"), b"original\n");
        assert_eq!(listing(dir.path()), ["guarded.md"]);
        assert!(matches!(probed, SaveError::Denied(_)), "{probed:?}");

        // The same last refusal, for a file that is merely writable, or merely
        // held by somebody else, is the moment it looks like.
        assert!(matches!(platform::gave_up(&path, denied()), SaveError::Busy(_)));
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("hold it open");
        assert!(matches!(platform::gave_up(&path, denied()), SaveError::Busy(_)));
        drop(held);
    }

    /// What `ReplaceFileW` is for: a hidden file stays hidden, and keeps the
    /// time it was created, which a rename would have taken from the
    /// temporary file.
    #[test]
    #[cfg(windows)]
    fn a_save_keeps_the_files_attributes_and_creation_time() {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use winapi::um::winnt::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY};

        let dir = TempDir::new("disk-save-attrs");
        let path = dir.path().join("hidden.md");
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .attributes(FILE_ATTRIBUTE_HIDDEN)
                .open(&path)
                .expect("a hidden file");
            std::io::Write::write_all(&mut file, b"before\n").expect("write it");
        }
        let before = std::fs::metadata(&path).expect("stat");
        assert_ne!(before.file_attributes() & FILE_ATTRIBUTE_HIDDEN, 0, "the fixture is hidden");
        // A gap the clock can see, so that a creation time taken from the
        // temporary file would be a different one.
        std::thread::sleep(Duration::from_millis(50));

        save(&path, dir.path(), b"after\n", &present(b"before\n"), Options::default())
            .expect("a hidden file is not a read-only one");
        let after = std::fs::metadata(&path).expect("stat");
        assert_eq!(std::fs::read(&path).expect("read"), b"after\n");
        assert_ne!(after.file_attributes() & FILE_ATTRIBUTE_HIDDEN, 0, "still hidden");
        assert_eq!(after.file_attributes() & FILE_ATTRIBUTE_READONLY, 0);
        assert_eq!(after.creation_time(), before.creation_time(), "the same file, to Windows");
        assert_eq!(listing(dir.path()), ["hidden.md"]);
    }

    /// A path past `MAX_PATH`, which a plain `W` call cannot reach without the
    /// verbatim prefix `std` adds for itself.
    #[test]
    #[cfg(windows)]
    fn a_file_deeper_than_max_path_saves() {
        let dir = TempDir::new("disk-save-long");
        let mut deep = dir.path().to_path_buf();
        for n in 0..6 {
            deep.push(format!("{n}-a-directory-name-long-enough-to-add-up-quickly-x"));
        }
        std::fs::create_dir_all(&deep).expect("a deep directory");
        let path = deep.join("file.md");
        std::fs::write(&path, b"deep\n").expect("a deep file");
        assert!(path.as_os_str().len() > 260, "the fixture is not long enough");

        let opened = open(&path, dir.path()).expect("editable");
        save(&path, dir.path(), b"deeper\n", &opened.baseline, Options::default())
            .expect("replaced through the verbatim prefix");
        assert_eq!(std::fs::read(&path).expect("read"), b"deeper\n");
        assert_eq!(listing(&deep), ["file.md"]);
    }

    #[test]
    #[cfg(windows)]
    fn a_long_path_is_made_verbatim_and_a_short_one_is_left_alone() {
        let text = |path: &str| {
            let wide = platform::wide(Path::new(path));
            assert_eq!(wide.last(), Some(&0), "NUL-terminated");
            String::from_utf16(&wide[..wide.len() - 1]).expect("UTF-16")
        };
        assert_eq!(text(r"C:\repo\a.md"), r"C:\repo\a.md");
        let long = format!(r"C:\{}\a.md", "x".repeat(260));
        assert_eq!(text(&long), format!(r"\\?\{long}"));
        let share = format!(r"\\server\share\{}\a.md", "x".repeat(260));
        assert_eq!(text(&share), format!(r"\\?\UNC\server\share\{}\a.md", "x".repeat(260)));
    }
}
