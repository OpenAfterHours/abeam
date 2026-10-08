//! Naming a file that is not there yet: what `a` in the file list checks
//! before it opens an empty editor.
//!
//! Every other path the files view touches was handed to it — by the watcher,
//! the git view, a row of the list — and exists. This is the one a person
//! types, and so the one that can be anything: a name with `..` in it, a path
//! into `.git`, a drive letter, `CON`. [`check`] is the whole of what stands
//! between that and a file appearing somewhere nobody meant, and it answers
//! before anything is made: no directory, no file, not even the editor. What
//! it refuses it says in a sentence the box shows, and the box stays open.
//!
//! It runs on every keystroke — the box says what `Enter` would do as the name
//! is typed — so nothing in it may fail on a name half typed, and nothing in it
//! slices the name by bytes: `Zoë.md` is four bytes before its dot, and the
//! third of them is half of a letter.
//!
//! ## One rule on every platform
//!
//! **The Windows rules are applied everywhere.** `<>:"|?*`, the device names,
//! a name ending in a dot or a space: on Linux each of those is a legal file
//! name, and each is a file the repository's next checkout on Windows cannot
//! write. A repository is shared between machines more often than not, and a
//! check that answered differently depending on where abeam happened to be
//! running would let a file be made on one that breaks the clone on the other.
//! For the same reason `/` and `\` both separate parts of a name on every
//! platform, and `.git` is matched in any case.
//!
//! Two more rules come from the same argument. A name is matched against the
//! names already in its directory **without regard to case**, on Linux too:
//! `README.md` typed beside a `readme.md` is that file, because on Windows and
//! macOS it is, and a repository holding both is one those checkouts cannot
//! write ([`same_name`] says how the case is folded). And a part of a name is
//! held to the shortest length any of them allows — [`LONGEST`] UTF-16 units,
//! Windows' measure, and as many bytes of UTF-8, the measure of Linux and
//! macOS — so that a name too long for somebody's disk is refused in the box
//! rather than by the first save, or by the next clone.
//!
//! ## What a name may be
//!
//! Relative to the directory the list is showing, and below it: no leading
//! slash, no drive or share, no `.` or `..` part, no empty part. Subdirectories
//! are allowed — `notes/today.md` — and the ones that are not there yet are
//! named on the box (`creates notes/`) and made by the first save, not before,
//! so a name typed and abandoned leaves nothing behind.
//!
//! **What a name is judged by is where it leads, not how it is spelled.** Every
//! part of it the disk already has is followed — a link, a junction, an NTFS
//! short name — and the path that comes out must be inside the workspace and
//! must not pass through a `.git` on the way down from its root. The spelling
//! rules above cannot be the whole guard, because the spelling is the one thing
//! a person controls: on a volume that makes short names `GIT~1` *is* `.git`, a
//! junction called `g` can lead into it, and either would let the first save
//! write `.git/hooks/post-checkout`, which is a program git runs. The outside
//! half is `crate::disk::save`'s own rule, asked early enough to be said on the
//! box rather than at the first `Ctrl+S`; the `.git` half is this module's
//! alone, because a save has no reason to know which directory is git's.
//!
//! A name that is already a file is not a refusal: it opens that file to edit,
//! as `e` would, because that is what somebody typing an existing name wants,
//! and it is opened under the spelling the disk has. It is judged by the same
//! rule first — a link to a file outside the workspace is refused here, before
//! the page has shown a line of it. A name that is already a directory is
//! refused, and so is a link to nothing.
//!
//! ## Which line ending a new file gets
//!
//! [`line_ending`]: the one the files beside it use. An empty file has none of
//! its own, and `crate::disk::open` gives a file without one `\n` on the
//! argument that a repository which wants `\r\n` has files that already say so.
//! A new file is the case where those files are right next to it, so it asks
//! them — the first file in the directory it is going into that has a line
//! ending, then the workspace root's — and takes `\n` when nothing answers. No
//! platform default: the same keystroke in the same repository makes the same
//! file on every machine. Never a byte order mark, which is a habit of the file
//! that has one and not of the directory.
//!
//! A file that is not on this machine is not asked. A OneDrive placeholder, or
//! anything else Windows marks as offline or as recalled when it is opened, is
//! downloaded by being read, and a line ending is not worth a download: see
//! [`remote`].

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::disk::open::Eol;
use crate::paths;

/// What a name typed into the box turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Named {
    /// Already a file, under the spelling the disk has: `a` opens it to edit,
    /// as `e` would.
    Existing(PathBuf),
    /// Not there yet: an empty editor on `path`, and the directories its first
    /// save will make, as the box says them — `notes/` — when there are any.
    New { path: PathBuf, makes: Option<String> },
}

/// The characters Windows will not have in a file name, which is reason enough
/// to refuse them in any repository: see the module doc.
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// The device names Windows reserves, with or without an extension and in any
/// case: `con.md` is the console there, not a file.
const DEVICES: &[&str] = &["CON", "PRN", "AUX", "NUL"];

/// The longest a part of a name may be, counted both ways the module doc
/// gives: in UTF-16 units, which is how NTFS counts its 255, and in bytes of
/// UTF-8, which is how ext4 and APFS count theirs. A UTF-16 count is never the
/// larger of the two, so it is the byte count that refuses a name of letters
/// outside ASCII that Windows alone would take.
const LONGEST: usize = 255;

/// What `typed` names inside `dir`, a directory of the workspace at `root` —
/// or the sentence that says why it may not be made.
pub(super) fn check(root: &Path, dir: &Path, typed: &str) -> Result<Named, String> {
    if typed.is_empty() {
        return Err("A new file needs a name.".into());
    }
    if typed.starts_with(['/', '\\']) {
        return Err(
            "A name from the directory shown, not from the top of a disk: no leading slash."
                .into(),
        );
    }
    let parts: Vec<&str> = typed.split(['/', '\\']).collect();
    if parts.last().is_some_and(|last| last.is_empty()) {
        return Err("That names a directory: put the file's name after the slash.".into());
    }
    for part in &parts {
        refuse_part(part)?;
    }

    // Down the parts as far as the disk has them, each under the disk's own
    // spelling when the typed one is not it: the deepest is where every link on
    // the way has been followed to, and what is left over is what the first
    // save will make.
    let mut at = dir.to_path_buf();
    let mut missing: Vec<&str> = Vec::new();
    for part in &parts {
        match missing.is_empty().then(|| spelled(&at, part)).flatten() {
            Some(name) => at.push(name),
            None => missing.push(part),
        }
    }
    let root = paths::resolve_root(root);
    let Some((_, dirs)) = missing.split_last() else {
        return existing(&root, &at, typed);
    };

    match std::fs::metadata(&at) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "{} is a file, so nothing can go inside it.",
                at.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ));
        }
        Err(why) => return Err(format!("abeam could not look at {}: {why}.", at.display())),
    }
    let resolved = paths::resolve(&at).map_err(|why| {
        format!("abeam could not follow {} to where it leads: {why}.", at.display())
    })?;
    if !paths::under(&root, &resolved) {
        return Err(format!(
            "{} leads outside this workspace through a link, and nothing is made there.",
            at.display()
        ));
    }
    if in_git(&root, &resolved) {
        return Err(into_git(typed));
    }
    let makes = (!dirs.is_empty()).then(|| format!("{}/", dirs.join("/")));
    let path = missing.iter().fold(at, |path, part| path.join(part));
    Ok(Named::New { path, makes })
}

/// Every part of the name is on disk already, at `at`: the file to open rather
/// than make, resolved — or why it may not be opened.
///
/// Judged here and not left to the strict read `e` makes, which would refuse a
/// file outside the workspace too, but only after the page had been handed it
/// and drawn it: a name typed into this box must not be a way to put a file
/// from somewhere else on the screen.
fn existing(root: &Path, at: &Path, typed: &str) -> Result<Named, String> {
    match std::fs::metadata(at) {
        Ok(meta) if meta.is_dir() => return Err(format!("{typed} is a directory, not a file.")),
        Ok(_) => {}
        // `spelled` found something, and following it found nothing.
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "{typed} is a link to something that is not there, and nothing is made \
                 through one."
            ));
        }
        Err(why) => return Err(format!("abeam could not look at {typed}: {why}.")),
    }
    let resolved = paths::resolve(at)
        .map_err(|why| format!("abeam could not follow {typed} to where it leads: {why}."))?;
    if !paths::under(root, &resolved) {
        return Err(format!(
            "{typed} leads outside this workspace through a link, and nothing there is opened \
             to edit."
        ));
    }
    if in_git(root, &resolved) {
        return Err(into_git(typed));
    }
    Ok(Named::Existing(resolved))
}

/// The refusal of a name that is not spelled `.git` and leads there anyway.
fn into_git(typed: &str) -> String {
    format!(
        "{typed} leads into .git once its links and short names are followed, and nothing goes \
         inside .git: that is git's own directory."
    )
}

/// Whether `resolved` — a path inside the workspace at `root`, every link
/// followed — passes through a `.git` on the way down from the root, in any
/// case. Both are spelled by `crate::paths`, so the root's parts are the
/// path's first ones and the rest is the way down; a `.git` *above* the root is
/// not this workspace's to guard.
fn in_git(root: &Path, resolved: &Path) -> bool {
    resolved.components().skip(root.components().count()).any(|part| {
        matches!(part, Component::Normal(name)
            if name.to_str().is_some_and(|name| name.eq_ignore_ascii_case(".git")))
    })
}

/// The name `part` has inside `dir` on disk, when it has one there: `part`
/// itself when the filesystem finds it under that spelling — exactly, or as
/// Windows and macOS find a name, in any case — and otherwise the first entry,
/// by name, that [`same_name`] says is the same name, which is the one Linux
/// will not find for itself. `None` when `dir` has nothing of that name, or is
/// not a directory.
///
/// The directory is read only when the plain lookup fails, and only for the
/// first part that is not there — after that nothing below can be — so a
/// keystroke costs at most one listing.
fn spelled(dir: &Path, part: &str) -> Option<OsString> {
    if std::fs::symlink_metadata(dir.join(part)).is_ok() {
        return Some(part.into());
    }
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name.to_str().is_some_and(|name| same_name(name, part)))
        .min()
}

/// Whether two names are one file on a disk that does not mind case — NTFS,
/// and APFS and HFS+ as macOS formats them — compared a character at a time
/// through [`fold`].
///
/// [`fold`] is Unicode's *simple* case mapping, one character to one, and
/// upper rather than lower case, because that is the shape of the table NTFS
/// itself compares names through: `ς` and `σ` are both `Σ` there, and `ß`
/// stays `ß` rather than becoming the `SS` a full mapping would make of it.
/// Not normalisation: an `é` written as one character and as `e` with an
/// accent after it are two names here, as they are to NTFS, though APFS would
/// call them one.
fn same_name(a: &str, b: &str) -> bool {
    a.chars().map(fold).eq(b.chars().map(fold))
}

/// `c` in upper case where that is one character, and `c` itself where it is
/// more than one — `ß`, a ligature — which is the simple mapping for every
/// character a file name is likely to hold, out of what `std` has to offer.
fn fold(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

/// One part of a name, between two separators, or the reason it may not be.
fn refuse_part(part: &str) -> Result<(), String> {
    if part.is_empty() {
        return Err("Two slashes in a row leave a part of the name empty.".into());
    }
    if part == "." || part == ".." {
        return Err(
            "`.` and `..` are not allowed: name the file from the directory shown, downwards."
                .into(),
        );
    }
    if let Some(c) = part.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(format!("`{c}` cannot be in a file name on Windows."));
    }
    if part.chars().any(char::is_control) {
        return Err("A name cannot hold a control character.".into());
    }
    let units = part.encode_utf16().count();
    if units > LONGEST {
        return Err(format!(
            "A part of a name can be at most {LONGEST} characters long on Windows, and this one \
             is {units}."
        ));
    }
    if part.len() > LONGEST {
        return Err(format!(
            "A part of a name can be at most {LONGEST} bytes long on Linux and macOS, and this \
             one is {} in UTF-8.",
            part.len()
        ));
    }
    if part.ends_with(['.', ' ']) {
        return Err(
            "A name cannot end in a dot or a space: Windows drops them, and the file would be \
             somebody else's."
                .into(),
        );
    }
    if part.eq_ignore_ascii_case(".git") {
        return Err("Nothing goes inside .git: that is git's own directory.".into());
    }
    if is_device(part) {
        return Err(format!(
            "{part} is a device on Windows, with any extension, and cannot be a file."
        ));
    }
    Ok(())
}

/// Whether `part`, before its first dot, is one of the names Windows keeps for
/// devices: [`DEVICES`], and `COM1`–`COM9` and `LPT1`–`LPT9`.
///
/// The four-letter ones are compared as bytes, not as a `str`: a stem four
/// bytes long is not always four characters — `Zoë`, `a日`, `ßß` — and a
/// `str` cut at its third byte there is cut through a letter, which panics.
/// A byte of a character outside ASCII never equals one inside it, so nothing
/// but `COM1`–`LPT9` can match.
fn is_device(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end();
    if DEVICES.iter().any(|device| stem.eq_ignore_ascii_case(device)) {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

/// The line ending a new file at `path` should be written with: see the module
/// doc. `root` is the workspace's.
pub(super) fn line_ending(path: &Path, root: &Path) -> Eol {
    let mut near = path.parent();
    while let Some(dir) = near {
        if dir.is_dir() {
            break;
        }
        near = dir.parent();
    }
    near.and_then(said_by)
        .or_else(|| said_by(root))
        .unwrap_or_default()
}

/// How many of a directory's files are asked, and how much of each is read:
/// enough to find a line ending in any text file anybody keeps beside another,
/// little enough to be nothing on a keystroke.
const ASKED: usize = 32;
const READ: u64 = 8 * 1024;

/// The line ending the first file in `dir` that has one uses — first by name,
/// so the answer does not depend on the order the directory lists in — or
/// `None` when no file there has one in its first [`READ`] bytes. A binary is
/// passed over: a `\n` in one says nothing about anybody's text. So is a file
/// that is not on this machine ([`remote`]), before it is opened.
fn said_by(dir: &Path) -> Option<Eol> {
    use std::io::Read;

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| !elsewhere(entry))
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files.iter().take(ASKED).find_map(|file| {
        let mut head = Vec::new();
        std::fs::File::open(file)
            .ok()?
            .take(READ)
            .read_to_end(&mut head)
            .ok()?;
        if head.contains(&0) {
            return None;
        }
        let at = head.iter().position(|&b| b == b'\n')?;
        Some(if at > 0 && head[at - 1] == b'\r' {
            Eol::CrLf
        } else {
            Eol::Lf
        })
    })
}

/// Whether the file `entry` names has its bytes somewhere other than this
/// machine, asked of what the directory listing already holds — on Windows a
/// `DirEntry`'s metadata comes with the listing, and asking for it opens
/// nothing. A file that will not say is passed over too: not knowing is not a
/// reason to risk the download.
#[cfg(windows)]
fn elsewhere(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    entry.metadata().map_or(true, |meta| remote(meta.file_attributes()))
}

/// Nothing Unix lists is fetched by being opened in a way a directory listing
/// can see.
#[cfg(not(windows))]
fn elsewhere(_: &std::fs::DirEntry) -> bool {
    false
}

/// Whether a file with these Windows attributes is downloaded, or recalled from
/// slower storage, by being opened or read: offline (`0x1000`), recalled on
/// open (`0x40000`), or recalled on data access (`0x400000`) — the last is what
/// a OneDrive file that is "available when online" carries. Any of them, and
/// reading eight kilobytes to learn a line ending would fetch the whole file.
#[cfg(windows)]
fn remote(attributes: u32) -> bool {
    use winapi::um::winnt::{
        FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    };
    attributes
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// A directory link at `link` to `target`: a junction on Windows, which
    /// needs no privilege — the precedent `crate::disk::save`'s junction test
    /// sets — and a symbolic link on Unix.
    fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        let made = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link).is_ok();
        made
    }

    fn refused(dir: &TempDir, typed: &str) -> String {
        check(dir.path(), dir.path(), typed).expect_err(typed)
    }

    #[test]
    fn every_kind_of_name_that_would_go_somewhere_unmeant_is_refused_with_a_reason() {
        let dir = TempDir::new("name-refused");
        let long = "a".repeat(LONGEST + 1);
        let deep = format!("{long}/x.md");
        let wide = format!("{}.md", "日".repeat(LONGEST / 3));
        for (typed, says) in [
            ("", "needs a name"),
            ("/etc/passwd", "no leading slash"),
            ("\\\\server\\share\\x.md", "no leading slash"),
            ("\\x.md", "no leading slash"),
            ("C:\\x.md", "`:`"),
            ("c:x.md", "`:`"),
            ("../x.md", "`..`"),
            ("notes/../../x.md", "`..`"),
            ("./x.md", "`.`"),
            ("notes/", "names a directory"),
            ("notes//x.md", "Two slashes"),
            ("a<b.md", "`<`"),
            ("a>b.md", "`>`"),
            ("a\"b.md", "`\"`"),
            ("a|b.md", "`|`"),
            ("a?b.md", "`?`"),
            ("a*b.md", "`*`"),
            ("a\tb.md", "control character"),
            ("a\u{7f}b.md", "control character"),
            ("notes.", "dot or a space"),
            ("notes ", "dot or a space"),
            ("dir./x.md", "dot or a space"),
            (".git/config", ".git"),
            (".GIT/hooks/x", ".git"),
            ("sub/.git/x", ".git"),
            ("CON", "device"),
            ("con.md", "device"),
            ("Nul.tar.gz", "device"),
            ("aux", "device"),
            ("PRN.txt", "device"),
            ("COM1", "device"),
            ("com9.log", "device"),
            ("LPT3.md", "device"),
            ("notes/con/x.md", "device"),
            // One part too long for NTFS, in a directory or at the end; and one
            // short enough for NTFS that is too long for ext4 and APFS.
            (long.as_str(), "255 characters long on Windows, and this one is 256"),
            (deep.as_str(), "255 characters"),
            (wide.as_str(), "255 bytes long on Linux and macOS, and this one is 258"),
        ] {
            let why = refused(&dir, typed);
            assert!(why.contains(says), "{typed:?}: {why}");
            assert!(why.ends_with('.'), "{typed:?}: a sentence, {why}");
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none(), "and nothing was made");
    }

    #[test]
    fn a_name_at_the_longest_a_part_may_be_is_a_name() {
        let dir = TempDir::new("name-longest");
        let ascii = "a".repeat(LONGEST);
        // 85 three-byte letters: 255 bytes, 85 UTF-16 units.
        let wide = "日".repeat(LONGEST / 3);
        for typed in [ascii, wide] {
            assert!(check(dir.path(), dir.path(), &typed).is_ok(), "{typed}");
        }
    }

    #[test]
    fn names_that_only_look_like_devices_are_names() {
        let dir = TempDir::new("name-devices");
        for typed in ["CONSOLE.md", "com10", "lpt0.md", "COM.md", "auxiliary.rs", "nul_check"] {
            assert!(check(dir.path(), dir.path(), typed).is_ok(), "{typed}");
        }
    }

    /// The box asks this on every keystroke, so a name it cannot read is a
    /// crash on the letter that made it so. Every one of these is four bytes
    /// before its first dot, which is the length the device check looks at
    /// twice — and each is four bytes that are not four characters.
    #[test]
    fn a_name_with_letters_outside_ascii_is_a_name_and_never_a_crash() {
        let dir = TempDir::new("name-unicode");
        for typed in [
            "Zoë.md", "ßß.md", "a日.md", "x字", "aaé", "Zoë", "çom1", "日本語.md", "notes/Zoë/x.md",
        ] {
            assert!(
                matches!(check(dir.path(), dir.path(), typed), Ok(Named::New { .. })),
                "{typed}"
            );
            // And every prefix of it, as the box sees it while it is typed.
            for (at, _) in typed.char_indices() {
                let _ = check(dir.path(), dir.path(), &typed[..at]);
            }
            assert!(!is_device(typed), "{typed}");
        }
    }

    #[test]
    fn a_nested_name_says_which_directories_its_first_save_will_make() {
        let dir = TempDir::new("name-nested");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        assert_eq!(
            check(dir.path(), dir.path(), "notes/today.md"),
            Ok(Named::New {
                path: dir.path().join("notes").join("today.md"),
                makes: Some("notes/".into())
            })
        );
        assert_eq!(
            check(dir.path(), dir.path(), "notes\\2025/today.md"),
            Ok(Named::New {
                path: dir.path().join("notes").join("2025").join("today.md"),
                makes: Some("notes/2025/".into())
            }),
            "either slash, on every platform"
        );
        assert_eq!(
            check(dir.path(), dir.path(), "src/new.rs"),
            Ok(Named::New {
                path: dir.path().join("src").join("new.rs"),
                makes: None
            })
        );
        // From the directory the list is in, not from the root.
        let src = dir.path().join("src");
        assert_eq!(
            check(dir.path(), &src, "lib.rs"),
            Ok(Named::New {
                path: src.join("lib.rs"),
                makes: None
            })
        );
        assert!(!dir.path().join("notes").exists(), "nothing is made by naming");
    }

    #[test]
    fn a_name_that_is_already_a_file_opens_it_and_a_directory_is_refused() {
        let dir = TempDir::new("name-existing");
        dir.write("README.md", b"# hi\n");
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        let readme = paths::resolve(&dir.path().join("README.md")).unwrap();
        assert_eq!(check(dir.path(), dir.path(), "README.md"), Ok(Named::Existing(readme)));
        assert!(refused(&dir, "docs").contains("is a directory"));
        assert!(refused(&dir, "README.md/x.md").contains("is a file"));
    }

    /// On Linux too: two names that differ only in case are two files there
    /// and one on Windows and macOS, and a repository holding both cannot be
    /// checked out on either. So the name already there is the one opened, and
    /// a directory already there is the one a new file goes into.
    #[test]
    fn a_name_that_differs_only_in_case_is_the_one_already_there_on_every_platform() {
        let dir = TempDir::new("name-case");
        dir.write("README.md", b"# hi\n");
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        let readme = paths::resolve(&dir.path().join("README.md")).unwrap();
        assert_eq!(check(dir.path(), dir.path(), "readme.MD"), Ok(Named::Existing(readme)));
        assert!(refused(&dir, "DOCS").contains("is a directory"));

        let Ok(Named::New { path, makes }) = check(dir.path(), dir.path(), "Docs/new.md") else {
            panic!("a new file in a directory that is there");
        };
        assert_eq!(makes, None, "no second `docs` beside the first");
        assert!(paths::same_dir(path.parent().unwrap(), &dir.path().join("docs")), "{path:?}");
        assert_eq!(path.file_name().unwrap(), "new.md");

        // The fold: one character to one, upper case, as NTFS compares.
        assert!(same_name("Zoë.md", "ZOË.MD"));
        assert!(same_name("όροσ", "ΌΡΟΣ") && same_name("όρος", "ΌΡΟΣ"), "both sigmas");
        assert!(!same_name("straße", "STRASSE"), "ß is not two letters");
        assert!(!same_name("a.md", "a.mdx"));
    }

    #[test]
    fn a_directory_that_leads_outside_the_workspace_is_refused_before_anything_is_made() {
        let dir = TempDir::new("name-link");
        let root = dir.path().join("repo");
        let outside = dir.path().join("elsewhere");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        assert!(link_dir(&outside, &root.join("out")), "a junction needs no elevation");
        std::fs::create_dir_all(root.join("inside")).unwrap();
        assert!(link_dir(&root.join("inside"), &root.join("in")));
        assert!(
            check(&root, &root, "in/x.md").is_ok(),
            "a link that stays inside is followed and allowed"
        );
        let why = check(&root, &root, "out/x.md").expect_err("outside");
        assert!(why.contains("outside this workspace"), "{why}");
        let why = check(&root, &root, "out/deeper/x.md").expect_err("outside, through a new dir");
        assert!(why.contains("outside this workspace"), "{why}");
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());

        // A file that is there, through the same link: refused too, before the
        // page has been handed a line of it.
        std::fs::write(outside.join("secret.md"), b"not the work\n").unwrap();
        let why = check(&root, &root, "out/secret.md").expect_err("an outside file");
        assert!(why.contains("outside this workspace"), "{why}");
        assert!(why.ends_with('.'), "{why}");
    }

    /// The spelling rules refuse `.git` typed; this is the same refusal for a
    /// `.git` reached any other way. A junction (a symbolic link on Unix) into
    /// it, and — on a Windows volume that makes them — its 8.3 short name, which
    /// is the spelling a person can type without typing `.git`.
    #[test]
    fn a_link_or_a_short_name_into_dot_git_is_refused_however_it_is_spelled() {
        let dir = TempDir::new("name-dotgit");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join(".git").join("hooks")).unwrap();
        std::fs::write(root.join(".git").join("config"), b"[core]\n").unwrap();
        assert!(link_dir(&root.join(".git"), &root.join("g")), "a junction needs no elevation");

        #[cfg(windows)]
        let short = short_name(&root.join(".git"));
        #[cfg(not(windows))]
        let short: Option<String> = None;
        if short.is_none() {
            eprintln!("skipped the short name: this volume makes none");
        }
        let spellings: Vec<String> = std::iter::once("g".to_string()).chain(short).collect();
        for spelling in &spellings {
            for typed in [
                format!("{spelling}/hooks/post-checkout"),
                format!("{spelling}/config"),
                format!("{spelling}/new/x.md"),
            ] {
                let why = check(&root, &root, &typed).expect_err(&typed);
                assert!(why.contains("nothing goes inside .git"), "{typed}: {why}");
            }
        }
        // From a directory under the root, too: the way down is from the root.
        std::fs::create_dir_all(root.join("sub")).unwrap();
        assert!(link_dir(&root.join(".git").join("hooks"), &root.join("sub").join("h")));
        let why = check(&root, &root.join("sub"), "h/pre-commit").expect_err("hooks");
        assert!(why.contains(".git"), "{why}");
        assert!(!root.join(".git").join("hooks").join("post-checkout").exists());
        assert!(!root.join(".git").join("new").exists());
    }

    /// The 8.3 short name of `path`'s last part, or `None` where the volume
    /// made none. Declared by hand, as `crate::disk::save` declares its two
    /// calls, rather than by turning on another winapi feature for a test.
    #[cfg(windows)]
    fn short_name(path: &Path) -> Option<String> {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use winapi::shared::minwindef::DWORD;
        use winapi::um::winnt::{LPCWSTR, LPWSTR};

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetShortPathNameW(long: LPCWSTR, short: LPWSTR, len: DWORD) -> DWORD;
        }

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut out = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated, and `out` is as long as it is said
        // to be; the call writes at most that many units and returns how many.
        let len = unsafe {
            GetShortPathNameW(wide.as_ptr(), out.as_mut_ptr(), out.len() as DWORD)
        } as usize;
        if len == 0 || len >= out.len() {
            return None;
        }
        let short = PathBuf::from(OsString::from_wide(&out[..len]));
        let name = short.file_name()?.to_str()?.to_string();
        (!name.eq_ignore_ascii_case(".git")).then_some(name)
    }

    #[test]
    fn a_new_file_takes_the_line_ending_its_neighbours_use() {
        let dir = TempDir::new("name-eol");
        let root = dir.path();
        std::fs::create_dir_all(root.join("crlf")).unwrap();
        std::fs::create_dir_all(root.join("lf")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("crlf").join("a.md"), b"one\r\ntwo\r\n").unwrap();
        std::fs::write(root.join("lf").join("a.md"), b"one\ntwo\n").unwrap();
        // A binary first by name is passed over, and so is a file with no
        // line ending at all.
        std::fs::write(root.join("lf").join("0.bin"), b"\x00\r\n").unwrap();
        std::fs::write(root.join("lf").join("00.txt"), b"no ending").unwrap();

        assert_eq!(line_ending(&root.join("crlf").join("new.md"), root), Eol::CrLf);
        assert_eq!(line_ending(&root.join("lf").join("new.md"), root), Eol::Lf);
        // A directory with nothing to say asks the root; a directory not made
        // yet asks the nearest one that is.
        assert_eq!(line_ending(&root.join("empty").join("new.md"), root), Eol::Lf);
        std::fs::write(root.join("README.md"), b"# r\r\n").unwrap();
        assert_eq!(line_ending(&root.join("empty").join("new.md"), root), Eol::CrLf);
        assert_eq!(
            line_ending(&root.join("crlf").join("deep").join("new.md"), root),
            Eol::CrLf
        );
        // And nothing anywhere is `\n`.
        let bare = TempDir::new("name-eol-bare");
        assert_eq!(line_ending(&bare.path().join("x.md"), bare.path()), Eol::Lf);
    }

    #[cfg(windows)]
    #[test]
    fn a_file_that_is_not_on_this_machine_is_not_opened_to_ask_it() {
        use std::io::Write;
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use winapi::um::winnt::{
            FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE,
            FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
        };

        // The predicate, for every bit that means "somewhere else".
        for bit in [
            FILE_ATTRIBUTE_OFFLINE,
            FILE_ATTRIBUTE_RECALL_ON_OPEN,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        ] {
            assert!(remote(bit), "{bit:#x}");
            assert!(remote(bit | FILE_ATTRIBUTE_READONLY), "{bit:#x}");
        }
        for here in [0, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY] {
            assert!(!remote(here), "{here:#x}");
        }

        // And a real one: the offline bit is one a program may set, which the
        // two recall bits are not. First by name, and CRLF, so asking it would
        // change the answer.
        let dir = TempDir::new("name-eol-offline");
        let away = dir.path().join("a.md");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .attributes(FILE_ATTRIBUTE_OFFLINE)
            .open(&away)
            .unwrap();
        file.write_all(b"one\r\ntwo\r\n").unwrap();
        drop(file);
        std::fs::write(dir.path().join("b.md"), b"one\ntwo\n").unwrap();
        let attributes = std::fs::metadata(&away).unwrap().file_attributes();
        if attributes & FILE_ATTRIBUTE_OFFLINE == 0 {
            eprintln!("skipped: this volume would not keep the offline attribute");
            return;
        }
        assert_eq!(said_by(dir.path()), Some(Eol::Lf), "the offline file was asked");
    }
}
