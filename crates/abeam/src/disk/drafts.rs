//! Unsaved text, kept where it survives abeam and never in the repository.
//!
//! An edit the user has not saved exists only in the process holding it, and a
//! process can be killed — a closed terminal, a crash, a machine that ran out
//! of battery. The scratch pad answers that by saving everything, every two
//! seconds, because its file is abeam's own. A file in a repository cannot be
//! answered that way: saving it is a decision the user makes, and an autosave
//! into the work would be the agent's file changing under it for a reason
//! nobody chose. So what is typed and not saved is copied *elsewhere* — a
//! recovery copy, in the profile beside the pad — and offered back the next
//! time that file is opened for editing.
//!
//! ## Where a copy lives
//!
//! `<profile>/abeam/drafts/<key>.txt`, where the profile is
//! `crate::disk::profile`'s — `%APPDATA%` on Windows, the XDG data directory on
//! Unix — and `drafts/` is the pad's `scratch/` sibling. `<key>` is
//! `crate::paths::workspace_key` of the file's absolute path: the file name for
//! a person to recognise and sixteen hex digits that are the actual key, under
//! the same rule that decides whether two spellings are one path everywhere
//! else in abeam. A caller passes `Opened::path`, the resolved path, so that a
//! file reached through a link and directly is one draft rather than two.
//!
//! **Never under the workspace**, for the pad's two reasons and a third. A
//! draft inside the repository would be an untracked file in the git pane next
//! door, and a change the watcher reports — and it would be a copy of
//! somebody's unsaved text sitting in the one directory that gets committed
//! and pushed. The profile is not inside a repository on any ordinary machine,
//! and [`Drafts::write`] asks rather than assumes, because the machines that
//! are not ordinary exist: a dotfiles repository whose root is the home
//! directory contains `~/.local/share` whole.
//!
//! ## What a copy says
//!
//! Five header lines and then the text, so that a person who finds one can
//! read it with anything and see what it is:
//!
//! ```text
//! abeam recovery copy 1
//! path: C:\Users\philm\forge\README.md
//! base: 46213:0123456789abcdef
//! saved: 1791380000
//! text: 812
//! …812 bytes of the text, exactly as the editor held it…
//! ```
//!
//! `base` is the [`Fingerprint`] of the file the text was typed against, or
//! `none` for a file being created: the files view compares it with the file
//! on disk when it offers the copy back, and starts in the conflict state if
//! they differ. `text` is a length rather than "everything after the header",
//! so a copy that lost its tail is recognised as damaged rather than offered
//! back as a shorter text. `path` is for the person and is not compared on
//! reading: the name is already that comparison, made under the one rule.
//!
//! Written the way everything else here is written — a temporary file, all of
//! it on the disk, a rename — and 0600 on Unix like the pad, because it is the
//! same kind of private text. When the copy is written, and how often, is the
//! files view's: this module only keeps it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::baseline::Fingerprint;
use super::{Perms, Temp, profile, temp_name};
use crate::paths;

/// The drafts' directory inside abeam's, beside the pad's `scratch`.
const DRAFTS: &str = "drafts";

/// Plain text with a header, which any program opens.
const EXT: &str = "txt";

/// The first line of every copy, and its version: a copy whose first line is
/// anything else was not written by this format and is not guessed at.
const MAGIC: &str = "abeam recovery copy 1";

/// Where this machine keeps recovery copies.
///
/// Asked once and kept, for the pad store's `path_for` reason: a directory
/// derived twice is a directory that can be derived differently, and a `None`
/// in the pane's hand at the start says there is nowhere before anybody types.
#[derive(Debug, Clone)]
pub struct Drafts {
    dir: PathBuf,
}

/// A recovery copy, read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// The text as the editor held it: `\n` between lines.
    pub text: String,
    /// The fingerprint of the file it was typed against, or `None` for a file
    /// that did not exist yet.
    pub base: Option<Fingerprint>,
    /// When it was written, to the second, where the clock said.
    pub saved_at: Option<SystemTime>,
}

impl Drafts {
    /// The drafts directory in this machine's profile, or `None` when the
    /// environment will not say where the profile is.
    pub fn here() -> Option<Self> {
        Some(Self::at(profile::dir()?.join(DRAFTS)))
    }

    /// Drafts kept in `dir`: what [`Drafts::here`] builds, and what a test
    /// builds instead, so that the suite never writes into the profile of
    /// whoever runs it.
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the copy of `target` is kept.
    pub fn path_of(&self, target: &Path) -> PathBuf {
        self.dir
            .join(format!("{}.{EXT}", paths::workspace_key(target)))
    }

    /// Keep `text` as the unsaved state of `target`, typed against `base`,
    /// replacing any copy already kept — unless that would put it inside
    /// `root`.
    ///
    /// The containment is asked twice: of the directory as spelled, before
    /// anything is made, and of the directory as resolved, once it exists,
    /// because a profile reached through a junction or a link is a profile
    /// somewhere else. The first is what keeps abeam from making a directory
    /// in the repository at all; the second can only refuse after one was
    /// made, through a link somebody pointed there, and refuses the copy
    /// regardless.
    ///
    /// `Err` is a sentence for the pane: a copy not kept is a promise the
    /// files view made and could not keep, and the person typing should know
    /// before the next crash rather than after it.
    pub fn write(
        &self,
        root: &Path,
        target: &Path,
        text: &str,
        base: Option<Fingerprint>,
    ) -> Result<(), String> {
        let root = paths::resolve_root(root);
        if paths::under(&root, &self.dir) {
            return Err(self.inside(target));
        }
        std::fs::create_dir_all(&self.dir).map_err(|why| self.failed(target, &why))?;
        if paths::resolve(&self.dir).is_ok_and(|dir| paths::under(&root, &dir)) {
            return Err(self.inside(target));
        }

        let path = self.path_of(target);
        let bytes = encode(target, text, base, SystemTime::now());
        // `path_of` always ends in the key, so the default is never reached.
        let key = path.file_name().map(OsString::from).unwrap_or_default();
        let temp = |n| path.with_file_name(temp_name(&key, n));
        let temp =
            Temp::write(temp, &bytes, Perms::Private).map_err(|why| self.failed(target, &why))?;
        temp.rename_over(&path).map_err(|why| self.failed(target, &why))
    }

    /// The copy kept for `target`: `Ok(None)` when there is none, and `Err`
    /// when there is something there that could not be read.
    ///
    /// **Those two must stay apart**, which is the pad store's lesson about
    /// `Loaded::unreadable` arriving here intact. A copy that is there and
    /// would not open — held for a moment by a scanner, damaged, written by a
    /// later abeam in a format this one does not know — is somebody's unsaved
    /// text, and answering `None` would let the next [`Drafts::write`] replace
    /// it with whatever was typed since. A caller that sees `Err` must not
    /// write a copy for that file this session.
    pub fn read(&self, target: &Path) -> Result<Option<Recovery>, String> {
        let path = self.path_of(target);
        match std::fs::read(&path) {
            Ok(bytes) => parse(&bytes).map(Some).ok_or_else(|| {
                self.unreadable(target, "it is not a recovery copy this abeam can read")
            }),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(why) => Err(self.unreadable(target, &why.to_string())),
        }
    }

    /// Forget the copy kept for `target`, after a save or a discard. A copy
    /// that is not there is already forgotten.
    pub fn delete(&self, target: &Path) -> Result<(), String> {
        let path = self.path_of(target);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(why) => Err(format!(
                "abeam could not remove the recovery copy of {} at {}: {why}.",
                target.display(),
                path.display()
            )),
        }
    }

    fn inside(&self, target: &Path) -> String {
        format!(
            "abeam did not keep a recovery copy of {}: the drafts directory, {}, is inside this \
             workspace, and a copy of unsaved text there would be committed with the work. Save \
             to keep what you typed.",
            target.display(),
            self.dir.display()
        )
    }

    fn failed(&self, target: &Path, why: &std::io::Error) -> String {
        format!(
            "abeam could not keep a recovery copy of {} in {}: {why}. What you typed is still \
             on screen; save to keep it.",
            target.display(),
            self.dir.display()
        )
    }

    fn unreadable(&self, target: &Path, why: &str) -> String {
        format!(
            "abeam found a recovery copy of {} at {} and could not read it: {why}. It has been \
             left where it is.",
            target.display(),
            self.path_of(target).display()
        )
    }
}

/// No profile to keep copies in, said before anybody types: the pad store's
/// `nowhere`, for the files view.
pub fn nowhere() -> String {
    format!(
        "abeam has nowhere in your profile to keep recovery copies: nothing absolute is set for \
         {}. Unsaved text will be lost if abeam is closed, so save as you go.",
        profile::PROFILE
    )
}

/// A copy's bytes. See the module docs for the shape.
fn encode(target: &Path, text: &str, base: Option<Fingerprint>, at: SystemTime) -> Vec<u8> {
    // One line however the path is spelled: a line break in a file name is
    // legal on Unix and would end the header early.
    let path = target.to_string_lossy().replace(['\r', '\n'], "\u{fffd}");
    let base = base.map_or_else(|| "none".to_string(), |base| base.to_string());
    let saved = at
        .duration_since(UNIX_EPOCH)
        .map_or_else(|_| "unknown".to_string(), |since| since.as_secs().to_string());
    let mut out = format!(
        "{MAGIC}\npath: {path}\nbase: {base}\nsaved: {saved}\ntext: {}\n",
        text.len()
    )
    .into_bytes();
    out.extend_from_slice(text.as_bytes());
    out
}

/// A copy read back, or `None` for anything that is not exactly one.
fn parse(bytes: &[u8]) -> Option<Recovery> {
    let mut rest = bytes;
    if line(&mut rest)? != MAGIC {
        return None;
    }
    line(&mut rest)?.strip_prefix("path: ")?;
    let base = match line(&mut rest)?.strip_prefix("base: ")? {
        "none" => None,
        print => Some(print.parse().ok()?),
    };
    // Checked, because a time is a number somebody's disk handed back: a
    // millisecond timestamp, or a digit flipped by damage, is past the end of
    // what a `SystemTime` holds on Windows, and `+` would panic opening the
    // file. That is a damaged copy like any other, and says so.
    let saved_at = match line(&mut rest)?.strip_prefix("saved: ")? {
        "unknown" => None,
        secs => Some(UNIX_EPOCH.checked_add(Duration::from_secs(secs.parse().ok()?))?),
    };
    let len: usize = line(&mut rest)?.strip_prefix("text: ")?.parse().ok()?;
    if rest.len() != len {
        return None;
    }
    Some(Recovery {
        text: String::from_utf8(rest.to_vec()).ok()?,
        base,
        saved_at,
    })
}

/// The next header line, without its `\n`, and `rest` moved past it.
fn line<'a>(rest: &mut &'a [u8]) -> Option<&'a str> {
    let end = rest.iter().position(|&b| b == b'\n')?;
    let found = std::str::from_utf8(&rest[..end]).ok()?;
    *rest = &rest[end + 1..];
    Some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    /// A workspace and a profile, side by side in one fixture and neither
    /// inside the other — the arrangement every ordinary machine has.
    struct Fixture {
        dir: TempDir,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = TempDir::new(tag);
            std::fs::create_dir_all(dir.path().join("repo")).expect("a repository");
            Self { dir }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("repo")
        }

        fn drafts(&self) -> Drafts {
            Drafts::at(self.dir.path().join("profile").join("abeam").join(DRAFTS))
        }
    }

    #[test]
    fn a_copy_kept_is_the_copy_that_comes_back_and_then_it_is_gone() {
        let fx = Fixture::new("drafts-roundtrip");
        let drafts = fx.drafts();
        let target = fx.root().join("README.md");
        // Header-shaped lines inside the text, no final newline, and text that
        // is not ASCII: the length is what ends the text, not a pattern.
        let text = "# Notes\ntext: 3\nbase: none\n\tcafé 日本";
        let base = Fingerprint::of(b"# Notes\n");

        assert_eq!(drafts.read(&target), Ok(None), "nothing kept yet");
        let before = SystemTime::now() - Duration::from_secs(1);
        drafts.write(&fx.root(), &target, text, Some(base)).expect("kept");
        let got = drafts.read(&target).expect("readable").expect("there");
        assert_eq!(got.text, text);
        assert_eq!(got.base, Some(base));
        assert!(got.saved_at.is_some_and(|at| at >= before), "{:?}", got.saved_at);

        // Kept again, it replaces the first, and leaves only itself behind.
        drafts.write(&fx.root(), &target, "", None).expect("kept again");
        let got = drafts.read(&target).expect("readable").expect("there");
        assert_eq!((got.text.as_str(), got.base), ("", None), "a new file's empty draft");
        let names: Vec<_> = std::fs::read_dir(drafts.dir())
            .expect("read the drafts")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert_eq!(names, [drafts.path_of(&target).file_name().expect("a name")]);

        drafts.delete(&target).expect("forgotten");
        assert_eq!(drafts.read(&target), Ok(None));
        drafts.delete(&target).expect("and forgetting twice is fine");
    }

    #[test]
    fn a_copy_names_its_file_and_its_base_in_words() {
        let fx = Fixture::new("drafts-header");
        let drafts = fx.drafts();
        let target = fx.root().join("docs").join("plan.md");
        let base = Fingerprint::of(b"the plan\n");
        drafts.write(&fx.root(), &target, "the new plan\n", Some(base)).expect("kept");

        let path = drafts.path_of(&target);
        assert!(path.extension().is_some_and(|ext| ext == EXT));
        assert!(
            path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("plan.md-")),
            "the file's own name, for somebody looking: {}",
            path.display()
        );
        let raw = String::from_utf8(std::fs::read(&path).expect("read")).expect("text");
        let mut lines = raw.lines();
        assert_eq!(lines.next(), Some(MAGIC));
        assert_eq!(lines.next(), Some(format!("path: {}", target.display()).as_str()));
        assert_eq!(lines.next(), Some(format!("base: {base}").as_str()));
        assert!(lines.next().is_some_and(|saved| saved.starts_with("saved: ")));
        assert_eq!(lines.next(), Some("text: 13"));
        assert_eq!(lines.next(), Some("the new plan"));
    }

    #[test]
    fn a_copy_is_never_kept_under_the_workspace() {
        let fx = Fixture::new("drafts-outside");
        let drafts = fx.drafts();
        let target = fx.root().join("README.md");
        drafts.write(&fx.root(), &target, "unsaved\n", None).expect("kept");
        let path = drafts.path_of(&target);
        assert!(path.is_file());
        assert!(!paths::under(&fx.root(), &path), "{} is in the repository", path.display());
        assert!(
            std::fs::read_dir(fx.root()).expect("read the repository").next().is_none(),
            "and nothing at all was written into the repository"
        );

        // This machine's own profile, as it would really be used: a sibling of
        // the pad's directory, and not inside a workspace in the temp
        // directory. Read from the environment rather than written to it.
        if let (Some(drafts), Some(abeam)) = (Drafts::here(), profile::dir()) {
            assert_eq!(drafts.dir(), abeam.join(DRAFTS));
            assert!(drafts.dir().is_absolute());
            assert!(!paths::under(&fx.root(), &drafts.path_of(&target)));
        }
    }

    #[test]
    fn a_drafts_directory_inside_the_workspace_is_refused_and_not_made() {
        // A dotfiles repository rooted at the home directory holds
        // `~/.local/share` whole, which is the machine this is for.
        let fx = Fixture::new("drafts-inside");
        let drafts = Drafts::at(fx.root().join(".local").join("share").join("abeam").join(DRAFTS));
        let target = fx.root().join("README.md");
        let why = drafts
            .write(&fx.root(), &target, "unsaved\n", None)
            .expect_err("never inside the workspace");
        assert!(why.contains("inside this workspace"), "{why}");
        assert!(why.contains("README.md"), "{why}");
        assert!(
            std::fs::read_dir(fx.root()).expect("read the repository").next().is_none(),
            "not even the directory was made"
        );
    }

    #[test]
    fn a_copy_that_is_there_and_will_not_read_is_not_mistaken_for_none() {
        let fx = Fixture::new("drafts-damaged");
        let drafts = fx.drafts();
        let target = fx.root().join("a.md");
        drafts.write(&fx.root(), &target, "twelve bytes", None).expect("kept");
        let path = drafts.path_of(&target);
        let whole = std::fs::read(&path).expect("read");

        // A tail lost, a header from somewhere else, and a later format.
        for damaged in [
            whole[..whole.len() - 1].to_vec(),
            b"not a recovery copy\n".to_vec(),
            String::from_utf8(whole.clone())
                .expect("text")
                .replace(MAGIC, "abeam recovery copy 2")
                .into_bytes(),
        ] {
            std::fs::write(&path, &damaged).expect("damage it");
            let why = drafts.read(&target).expect_err("there, and not readable");
            assert!(why.contains("left where it is"), "{why}");
            assert_eq!(std::fs::read(&path).expect("read"), damaged, "and it was");
        }
    }

    #[test]
    fn a_time_past_what_the_clock_can_hold_is_damage_and_not_a_crash() {
        // `UNIX_EPOCH + secs` panics past the end of `SystemTime`, which on
        // Windows is about 9.1e11 seconds away — a millisecond timestamp gets
        // there. This copy would have crashed abeam on the next `e`.
        let fx = Fixture::new("drafts-time");
        let drafts = fx.drafts();
        let target = fx.root().join("a.md");
        let with_time = |saved: &str| {
            format!("{MAGIC}\npath: {}\nbase: none\nsaved: {saved}\ntext: 2\nhi", target.display())
        };

        std::fs::create_dir_all(drafts.dir()).expect("the drafts directory");
        let path = drafts.path_of(&target);
        std::fs::write(&path, with_time(&u64::MAX.to_string())).expect("a far-future copy");
        let why = drafts.read(&target).expect_err("damaged, not a panic and not none");
        assert!(why.contains("left where it is"), "{why}");

        // A millisecond timestamp: past the end on Windows, a very late year
        // on Unix. Either answer is fine; a panic is not.
        std::fs::write(&path, with_time("1791380000000")).expect("a millisecond copy");
        let _ = drafts.read(&target);

        std::fs::write(&path, with_time("1791380000")).expect("an ordinary copy");
        let got = drafts.read(&target).expect("readable").expect("there");
        assert_eq!(got.saved_at, UNIX_EPOCH.checked_add(Duration::from_secs(1_791_380_000)));
        assert_eq!(got.text, "hi");
    }

    #[test]
    fn a_copy_written_beside_something_with_its_temporary_name_leaves_that_alone() {
        let fx = Fixture::new("drafts-taken");
        let drafts = fx.drafts();
        let target = fx.root().join("a.md");
        std::fs::create_dir_all(drafts.dir()).expect("the drafts directory");
        let path = drafts.path_of(&target);
        let squatter = path.with_file_name(temp_name(path.file_name().expect("a name"), 0));
        std::fs::write(&squatter, b"somebody's\n").expect("a file on the temporary name");

        drafts.write(&fx.root(), &target, "kept\n", None).expect("kept under another name");
        assert_eq!(drafts.read(&target).expect("readable").expect("there").text, "kept\n");
        assert_eq!(std::fs::read(&squatter).expect("still there"), b"somebody's\n");
    }

    #[test]
    fn one_file_spelled_two_ways_is_one_copy_where_the_platform_says_so() {
        let drafts = Drafts::at(PathBuf::from("drafts"));
        #[cfg(windows)]
        assert_eq!(
            drafts.path_of(Path::new(r"C:\Repo\README.md")),
            drafts.path_of(Path::new(r"c:/repo/readme.md"))
        );
        #[cfg(unix)]
        assert_ne!(
            drafts.path_of(Path::new("/repo/README.md")),
            drafts.path_of(Path::new("/repo/readme.md"))
        );
        // And two files are two copies everywhere, even with one name.
        #[cfg(windows)]
        let (one, other) = (r"C:\repo\a\README.md", r"C:\repo\b\README.md");
        #[cfg(unix)]
        let (one, other) = ("/repo/a/README.md", "/repo/b/README.md");
        assert_ne!(drafts.path_of(Path::new(one)), drafts.path_of(Path::new(other)));
    }

    #[test]
    fn having_nowhere_to_keep_copies_says_so_before_anybody_types() {
        let why = nowhere();
        assert!(why.contains(profile::PROFILE), "{why}");
        assert!(why.contains("save"), "{why}");
    }

    /// Private text, and a mode that says so, like the pad.
    #[test]
    #[cfg(unix)]
    fn a_copy_is_readable_only_by_the_person_who_typed_it() {
        use std::os::unix::fs::PermissionsExt;

        let fx = Fixture::new("drafts-private");
        let drafts = fx.drafts();
        let target = fx.root().join("a.md");
        drafts.write(&fx.root(), &target, "nobody else's business\n", None).expect("kept");
        let mode = std::fs::metadata(drafts.path_of(&target))
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the copy is {:o}", mode & 0o777);
    }
}
