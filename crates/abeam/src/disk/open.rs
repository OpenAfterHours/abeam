//! Reading a file to edit it, which is a stricter question than reading it to
//! show it.
//!
//! `crate::panes::viewer::load` is generous on purpose. Every path it is handed
//! was chosen by an agent, so it decodes whatever it finds — a Latin-1 byte
//! becomes a replacement character, a `\r\n` becomes a `\n`, a lone `\r` too —
//! because a page somebody is reading is better shown approximately than not
//! at all. Every one of those generosities is a lie the moment the text is
//! written back: the replacement character goes to disk in place of the byte,
//! and the whole file changes its line endings in a diff that is about one
//! typo. A save would make the approximate reading true.
//!
//! So [`open`] asks the stricter question — can this be given back exactly?
//! — and answers no with a [`Refusal`] the files view puts in its border,
//! still showing the file through `load`'s reading. What it says yes to, it
//! takes apart into what an [`Editor`](crate::editor::Editor) holds and what
//! it does not:
//!
//! - the text, with `\n` between lines, which is all the editor ever holds;
//! - the [`Format`]: which line ending the file used, and whether it began with
//!   a UTF-8 byte order mark;
//! - the [`Baseline`]: every byte as read, for the save to compare against.
//!
//! The final newline needs nothing of its own, and that is worth being exact
//! about because it is the one most editors get wrong. The editor's text is
//! its lines joined by `\n`, so a file that ends with a line ending has an
//! empty last line and gives the ending back when it is joined, and a file
//! that does not, does not. `"a\n"` is two lines, `["a", ""]`, and `"a"` is
//! one; nothing has to remember which it was.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::baseline::{Baseline, Fingerprint};
use crate::editor::{Policy, TabKey, Tabs};
use crate::panes::viewer::load::{self, MAX_BYTES, SNIFF_BYTES};

/// The UTF-8 byte order mark. Notepad writes one and says nothing, so a file
/// that has one is ordinary on Windows, and a save that dropped it would be a
/// one-line diff in every file somebody touched.
pub const BOM: &[u8] = b"\xEF\xBB\xBF";

/// The line ending a file uses throughout.
///
/// Two, because a file with any other mix is refused rather than recorded —
/// see [`Refusal::MixedEndings`] — and an old Mac's lone `\r` is one of those.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Eol {
    /// `\n`. Also what a file with no line ending at all is given, so that the
    /// first `Enter` typed into it has an answer.
    ///
    /// The platform's own ending would be the other candidate there, and it is
    /// refused for the reason one rule beats two: a one-line file opened on
    /// Windows and on Linux would otherwise grow different endings from the
    /// same keystroke, in one repository. LF is also what the pad writes on
    /// every platform, and what git normalises to wherever anybody has asked
    /// it to normalise; a repository that wants CRLF has files that already
    /// say so, and those keep it.
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
}

impl Eol {
    /// How many bytes one of these is on disk, which is what the editor's cap
    /// needs to count honestly. See [`Policy::line_ending_bytes`].
    pub fn bytes(self) -> usize {
        match self {
            Eol::Lf => 1,
            Eol::CrLf => 2,
        }
    }
}

/// Everything about a file's bytes that the editor's text does not carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Format {
    pub eol: Eol,
    /// The file began with [`BOM`].
    pub bom: bool,
}

impl Format {
    /// The editor's text as the bytes of a file in this format: the mark if
    /// there was one, and this line ending between every two lines.
    ///
    /// The inverse of what [`open`] took apart, exactly. `text` is what
    /// `Editor::text` returns, which never holds a `\r`; one that did would be
    /// written out as it stands and make the file mixed, which is the reason
    /// for the assertion rather than a reason to clean it here.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        debug_assert!(!text.contains('\r'), "an editor's text never holds a carriage return");
        let breaks = text.bytes().filter(|&b| b == b'\n').count();
        let mut out =
            Vec::with_capacity(BOM.len() + text.len() + breaks * (self.eol.bytes() - 1));
        if self.bom {
            out.extend_from_slice(BOM);
        }
        match self.eol {
            Eol::Lf => out.extend_from_slice(text.as_bytes()),
            Eol::CrLf => {
                for (i, line) in text.split('\n').enumerate() {
                    if i > 0 {
                        out.extend_from_slice(b"\r\n");
                    }
                    out.extend_from_slice(line.as_bytes());
                }
            }
        }
        out
    }

    /// What an editor of a file in this format is held to: the reader's
    /// `load::MAX_BYTES` counted in the bytes the save will write — this line
    /// ending, this mark — and tabs kept as they are, with the `Tab` key
    /// typing `key`.
    pub fn policy(&self, key: TabKey) -> Policy {
        Policy {
            max_bytes: MAX_BYTES as usize,
            tabs: Tabs::Keep { key },
            line_ending_bytes: self.eol.bytes(),
            bom_bytes: if self.bom { BOM.len() } else { 0 },
        }
    }
}

/// A file that can be edited and given back exactly.
#[derive(Debug, Clone)]
pub struct Opened {
    /// What the path resolved to — links followed, spelled the way
    /// `crate::paths::resolve` spells it. The file a save will replace, and
    /// the path a recovery copy is kept under, so that two spellings of one
    /// file are one draft.
    pub path: PathBuf,
    /// The path as the caller spelled it, under the root — for a link, the
    /// link. What a sentence about the file should name, and one of the two
    /// spellings the watcher may report a change under: see
    /// `crate::disk::Saved::requested`.
    pub requested: PathBuf,
    /// The text with `\n` between lines and without the mark: what
    /// `Editor::from_text` takes.
    pub text: String,
    pub format: Format,
    /// Every byte as read.
    pub baseline: Baseline,
}

impl Opened {
    /// What a recovery copy of this file records as the version it was typed
    /// against.
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(match &self.baseline {
            Baseline::Present(bytes) => bytes,
            Baseline::Absent => &[],
        })
    }

    /// The editor policy for this file: its format's, with the `Tab` key
    /// typing a tab if any line of the file is already indented with one and
    /// two spaces if none is.
    ///
    /// Decided once, from the file as it was opened, because the alternative —
    /// asking again as the user types — would have `Tab` change what it types
    /// in the middle of a session, the first time somebody indents a line by
    /// hand. A file with no indentation at all gets the spaces, which is what
    /// most of what anybody opens here — markdown, YAML, Python — wants, and
    /// a file whose own convention is tabs says so in its first indented line.
    pub fn policy(&self) -> Policy {
        let key = if self.text.split('\n').any(|line| line.starts_with('\t')) {
            TabKey::Tab
        } else {
            TabKey::Spaces(2)
        };
        self.format.policy(key)
    }
}

/// Why a file is shown but not editable, each with the sentence that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Missing,
    Denied,
    /// A directory, a device, a pipe: something that is not one regular file.
    NotAFile,
    /// The path resolves — links and junctions followed — to somewhere outside
    /// the workspace root, under `crate::paths::under`'s rule.
    ///
    /// Judged on the resolved path because that is the file a save would
    /// write: a link inside the repository to a file outside it is a way of
    /// editing something that is not the work, from a pane that says it is
    /// showing the work.
    Outside,
    /// Larger than `load::MAX_BYTES` on disk. The reader shows the head of it;
    /// a save of the head would delete the rest.
    TooLarge { bytes: u64 },
    /// A NUL in the first `load::SNIFF_BYTES`: `load`'s test, and git's.
    Binary,
    /// Not valid UTF-8, first going wrong on this line. Editing would mean
    /// decoding lossily, and the first save would write the replacement
    /// characters back over the bytes they stand for.
    NotUtf8 { line: usize },
    /// Some lines end in `\r\n` and some in `\n`, or a `\r` stands on its own,
    /// first on this line. The editor holds one ending, so a save would give
    /// back one ending, and the diff would be every line of the minority kind.
    MixedEndings { line: usize },
    /// Another program had the file open without sharing it for longer than
    /// the retries waited — Defender, the indexer, OneDrive. Windows only, and
    /// a moment rather than a property of the file, so it says to try again.
    Busy,
    /// Anything else the platform said, in its words.
    Io(String),
}

impl Refusal {
    /// One sentence for the border, saying why the file is read-only here.
    pub fn message(&self) -> String {
        match self {
            Refusal::Missing => "No such file: it may have been renamed or deleted.".into(),
            Refusal::Denied => "Permission denied.".into(),
            Refusal::NotAFile => "Not a regular file.".into(),
            Refusal::Outside => "Outside this workspace once its links are followed.".into(),
            Refusal::TooLarge { bytes } => format!(
                "Larger than the {} abeam will edit ({}).",
                load::human(MAX_BYTES),
                load::human(*bytes)
            ),
            Refusal::Binary => "A binary file: there is no text to edit.".into(),
            Refusal::NotUtf8 { line } => format!(
                "Not valid UTF-8 (line {line}), so a save could not give back the bytes it read."
            ),
            Refusal::MixedEndings { line } => format!(
                "Mixed line endings (line {line}), which a save would make all one kind."
            ),
            Refusal::Busy => {
                "Another program is holding it open; try again in a moment.".into()
            }
            Refusal::Io(why) => format!("Could not be read: {why}."),
        }
    }

    fn from_io(why: std::io::Error) -> Self {
        if super::held(&why) {
            return Refusal::Busy;
        }
        match why.kind() {
            std::io::ErrorKind::NotFound => Refusal::Missing,
            std::io::ErrorKind::PermissionDenied => Refusal::Denied,
            std::io::ErrorKind::IsADirectory => Refusal::NotAFile,
            _ => Refusal::Io(why.to_string()),
        }
    }
}

/// The file at `path`, whole and exactly, if it can be edited inside `root`.
///
/// **Resolved before anything else is asked of it**, and the containment is
/// asked of the resolved path: see [`Refusal::Outside`]. `root` is resolved
/// too, which for a root that came from `main` is a second canonicalisation of
/// an answer that is already canonical — one call, for a guarantee that does
/// not depend on where the root came from.
///
/// **One byte past the cap rather than a `metadata` call** for the size, which
/// is `load`'s trick: the read itself says whether there was more, so a size
/// that went stale between two calls cannot decide it.
///
/// **A file another program has open without sharing it is waited for**, the
/// save's short way (`crate::disk::PAUSES`), because on Windows that is what
/// Defender does to the file the agent has just written — exactly the file
/// somebody presses `e` on — and a refusal for that moment would mark a file
/// read-only that is fine.
pub fn open(path: &Path, root: &Path) -> Result<Opened, Refusal> {
    let resolved = crate::paths::resolve(path).map_err(Refusal::from_io)?;
    if !crate::paths::under(&crate::paths::resolve_root(root), &resolved) {
        return Err(Refusal::Outside);
    }
    // Checked before opening, for `load`'s reason: on Windows opening a
    // directory fails with a bare access-denied.
    let meta = std::fs::metadata(&resolved).map_err(Refusal::from_io)?;
    if !meta.is_file() {
        return Err(Refusal::NotAFile);
    }

    let bytes = super::patiently(super::held, || {
        let mut bytes = Vec::new();
        std::fs::File::open(&resolved)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    })
    .map_err(Refusal::from_io)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Refusal::TooLarge {
            bytes: meta.len().max(bytes.len() as u64),
        });
    }

    let (text, format) = decode(&bytes)?;
    Ok(Opened {
        path: resolved,
        requested: path.to_path_buf(),
        text,
        format,
        baseline: Baseline::Present(bytes.into()),
    })
}

/// The bytes of a file taken apart into text and [`Format`], or the reason
/// they cannot be.
///
/// The order of the questions is the order of the sentences somebody would
/// want: a binary file is also not UTF-8, almost always, and "binary" is the
/// more useful thing to be told.
pub fn decode(bytes: &[u8]) -> Result<(String, Format), Refusal> {
    if bytes.iter().take(SNIFF_BYTES).any(|&b| b == 0) {
        return Err(Refusal::Binary);
    }
    let (bom, body) = match bytes.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let text = std::str::from_utf8(body).map_err(|why| Refusal::NotUtf8 {
        line: 1 + body[..why.valid_up_to()].iter().filter(|&&b| b == b'\n').count(),
    })?;
    let eol = endings(body)?;
    let text = match eol {
        // Nothing to take out: with no `\r\n` in the file and no lone `\r`
        // either, there is no `\r` at all.
        Eol::Lf => text.to_owned(),
        Eol::CrLf => text.replace("\r\n", "\n"),
    };
    Ok((text, Format { eol, bom }))
}

/// The one line ending `body` uses, or the line on which it first uses a
/// second.
///
/// Counted on the bytes, which is safe in UTF-8 for the reason the encoding
/// was designed around: `\r` and `\n` can never appear inside a multi-byte
/// character.
fn endings(body: &[u8]) -> Result<Eol, Refusal> {
    let mut seen = None;
    let mut line = 1;
    let mut at = 0;
    while let Some(off) = body[at..].iter().position(|&b| b == b'\n' || b == b'\r') {
        let i = at + off;
        let this = if body[i] == b'\n' {
            at = i + 1;
            Eol::Lf
        } else if body.get(i + 1) == Some(&b'\n') {
            at = i + 2;
            Eol::CrLf
        } else {
            // A `\r` with no `\n` after it is a third ending, and the editor
            // has nowhere to keep it.
            return Err(Refusal::MixedEndings { line });
        };
        match seen {
            None => seen = Some(this),
            Some(first) if first != this => return Err(Refusal::MixedEndings { line }),
            Some(_) => {}
        }
        line += 1;
    }
    Ok(seen.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::testutil::{TempDir, symlink_file};

    /// A file's bytes, through `decode`, through an editor and back, must be
    /// the bytes it started as — the property this whole module exists for.
    fn round_trip(bytes: &[u8]) -> Format {
        let (text, format) = decode(bytes).expect("an editable file");
        let editor = Editor::from_text(format.policy(TabKey::Tab), &text);
        assert!(!editor.truncated(), "{bytes:?} fits the editor whole");
        assert_eq!(editor.text(), text, "the editor holds the text it was given");
        assert_eq!(format.encode(&editor.text()), bytes, "{bytes:?} came back changed");
        format
    }

    #[test]
    fn every_shape_of_file_comes_back_byte_for_byte() {
        let lf = Format { eol: Eol::Lf, bom: false };
        let crlf = Format { eol: Eol::CrLf, bom: false };
        for (bytes, format) in [
            (&b"one\ntwo\n"[..], lf),
            (b"one\r\ntwo\r\n", crlf),
            (b"\xEF\xBB\xBFone\r\ntwo\r\n", Format { eol: Eol::CrLf, bom: true }),
            (b"\xEF\xBB\xBFone\ntwo", Format { eol: Eol::Lf, bom: true }),
            // No final newline, either ending.
            (b"one\ntwo", lf),
            (b"one\r\ntwo", crlf),
            // Nothing at all, and nothing but one ending.
            (b"", lf),
            (b"\n", lf),
            (b"\r\n", crlf),
            (b"\r\n\r\n", crlf),
            (b"\xEF\xBB\xBF", Format { eol: Eol::Lf, bom: true }),
            // No line ending anywhere: the default, which writes nothing back.
            (b"just one line", lf),
            // Tabs kept, and text that is not ASCII.
            ("\tfn main() {\r\n\t\tlet café = \"日本\";\r\n\t}\r\n".as_bytes(), crlf),
            ("- 二つ\n\t- trailing spaces   \n".as_bytes(), lf),
        ] {
            assert_eq!(round_trip(bytes), format, "{bytes:?}");
        }
    }

    #[test]
    fn the_final_newline_is_the_editors_empty_last_line() {
        // Nothing records it, because nothing has to: the editor's text is its
        // lines joined by `\n`, so the ending is an empty line, and it comes
        // back on the join.
        let (text, _) = decode(b"a\r\n").expect("editable");
        let editor = Editor::from_text(Format::default().policy(TabKey::Tab), &text);
        assert_eq!(editor.line_count(), 2);
        assert_eq!(editor.text(), "a\n");
        let (text, _) = decode(b"a").expect("editable");
        assert_eq!(Editor::from_text(Format::default().policy(TabKey::Tab), &text).line_count(), 1);
    }

    #[test]
    fn what_could_not_be_given_back_is_refused_with_the_line_it_goes_wrong_on() {
        assert_eq!(decode(b"one\r\ntwo\nthree\r\n"), Err(Refusal::MixedEndings { line: 2 }));
        assert_eq!(decode(b"one\ntwo\r\n"), Err(Refusal::MixedEndings { line: 2 }));
        assert_eq!(decode(b"one\rtwo\r"), Err(Refusal::MixedEndings { line: 1 }));
        assert_eq!(decode(b"one\n\ntwo\r"), Err(Refusal::MixedEndings { line: 3 }));
        assert_eq!(decode(b"a lone CR at the end\r"), Err(Refusal::MixedEndings { line: 1 }));
        assert_eq!(decode(b"one\ncaf\xe9\n"), Err(Refusal::NotUtf8 { line: 2 }));
        // A binary is binary even when it is also not UTF-8, because that is
        // the more useful sentence; and a NUL past the sniff is text, as it is
        // to the reader.
        assert_eq!(decode(b"\x7fELF\x00\x01\xff"), Err(Refusal::Binary));
        let mut late = vec![b'x'; SNIFF_BYTES];
        late.push(0);
        assert!(decode(&late).is_ok());
        // A mark followed by bytes that are not UTF-8 is still not UTF-8.
        assert_eq!(decode(b"\xEF\xBB\xBF\xff"), Err(Refusal::NotUtf8 { line: 1 }));
        // UTF-16 announces itself with NULs, and is binary to this as to git.
        assert_eq!(decode(b"\xFF\xFEa\x00b\x00"), Err(Refusal::Binary));
    }

    #[test]
    fn every_refusal_says_something() {
        for refusal in [
            Refusal::Missing,
            Refusal::Denied,
            Refusal::NotAFile,
            Refusal::Outside,
            Refusal::TooLarge { bytes: 600 * 1024 },
            Refusal::Binary,
            Refusal::NotUtf8 { line: 3 },
            Refusal::MixedEndings { line: 4 },
            Refusal::Busy,
            Refusal::Io("the disk is on fire".into()),
        ] {
            let said = refusal.message();
            assert!(said.ends_with('.'), "{refusal:?}: {said}");
            assert!(said.chars().next().is_some_and(char::is_uppercase), "{said}");
        }
        assert!(Refusal::TooLarge { bytes: 600 * 1024 }.message().contains("512.0 KB"));
        assert!(Refusal::MixedEndings { line: 4 }.message().contains("line 4"));
    }

    #[test]
    fn a_file_is_opened_whole_with_its_format_and_its_bytes() {
        let dir = TempDir::new("disk-open");
        let bytes = b"\xEF\xBB\xBF# Notes\r\n\r\n\tindented\r\n";
        let path = dir.write("notes.md", bytes);
        let opened = open(&path, dir.path()).expect("editable");
        assert_eq!(opened.text, "# Notes\n\n\tindented\n");
        assert_eq!(opened.format, Format { eol: Eol::CrLf, bom: true });
        assert_eq!(opened.baseline, Baseline::Present(bytes.to_vec().into()));
        assert_eq!(opened.fingerprint(), Fingerprint::of(bytes));
        assert!(crate::paths::same_dir(&opened.path, &path));

        // The policy counts the disk's bytes, and the tab-indented line makes
        // the `Tab` key a tab.
        let policy = opened.policy();
        assert_eq!(policy.line_ending_bytes, 2);
        assert_eq!(policy.bom_bytes, 3);
        assert_eq!(policy.max_bytes, MAX_BYTES as usize);
        assert_eq!(policy.tabs, Tabs::Keep { key: TabKey::Tab });
        let plain = open(&dir.write("plain.md", b"  two spaces\n"), dir.path()).expect("editable");
        assert_eq!(plain.policy().tabs, Tabs::Keep { key: TabKey::Spaces(2) });
    }

    #[test]
    fn a_file_at_the_cap_is_editable_whole_and_one_byte_more_is_not() {
        // At the cap with a mark and CRLF, so that the editor's own count of
        // the disk's bytes is what is under test: a cap counted in LF would
        // take this whole and then refuse the save's own size, or cut it.
        let dir = TempDir::new("disk-open-cap");
        let line = b"0123456789abcdef0123456789abcd\r\n";
        let mut bytes = BOM.to_vec();
        while bytes.len() + line.len() <= MAX_BYTES as usize {
            bytes.extend_from_slice(line);
        }
        bytes.resize(MAX_BYTES as usize, b'x');
        let path = dir.write("cap.txt", &bytes);
        let opened = open(&path, dir.path()).expect("exactly at the cap");
        let editor = Editor::from_text(opened.policy(), &opened.text);
        assert!(!editor.truncated(), "the editor took all of it");
        assert!(editor.is_full(), "and has no room left");
        assert_eq!(opened.format.encode(&editor.text()), bytes);

        bytes.push(b'x');
        std::fs::write(&path, &bytes).expect("one byte more");
        assert_eq!(
            open(&path, dir.path()).map(|_| ()),
            Err(Refusal::TooLarge { bytes: MAX_BYTES + 1 })
        );
    }

    #[test]
    fn what_is_not_one_file_inside_the_workspace_is_refused() {
        let dir = TempDir::new("disk-open-where");
        let root = dir.path().join("repo");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(root.join("sub")).expect("a repository");
        std::fs::create_dir_all(&elsewhere).expect("somewhere else");
        let outside = elsewhere.join("secret.md");
        std::fs::write(&outside, b"not the work\n").expect("a file outside");

        assert_eq!(open(&root.join("nope.md"), &root).map(|_| ()), Err(Refusal::Missing));
        assert_eq!(open(&root.join("sub"), &root).map(|_| ()), Err(Refusal::NotAFile));
        assert_eq!(open(&outside, &root).map(|_| ()), Err(Refusal::Outside));
        // `..` is resolved, not compared: this spelling starts with the root.
        let climbed = root.join("sub").join("..").join("..").join("elsewhere").join("secret.md");
        assert_eq!(open(&climbed, &root).map(|_| ()), Err(Refusal::Outside));
    }

    /// A link inside the repository to a file outside it, and one to a file
    /// inside it. Skipped, saying so, where links cannot be made — Windows
    /// without Developer Mode or elevation.
    #[test]
    fn a_link_is_judged_by_what_it_points_at() {
        let dir = TempDir::new("disk-open-link");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("a repository");
        let inside = root.join("real.md");
        std::fs::write(&inside, b"the work\n").expect("a file inside");
        let outside = dir.path().join("secret.md");
        std::fs::write(&outside, b"not the work\n").expect("a file outside");

        if !symlink_file(&outside, &root.join("out.md"))
            || !symlink_file(&inside, &root.join("in.md"))
        {
            eprintln!("skipped: this machine will not make a symlink");
            return;
        }
        assert_eq!(open(&root.join("out.md"), &root).map(|_| ()), Err(Refusal::Outside));
        let opened = open(&root.join("in.md"), &root).expect("a link to the work");
        assert!(crate::paths::same_dir(&opened.path, &inside), "resolved to the file itself");
        assert_eq!(opened.requested, root.join("in.md"), "and the link as it was asked for");
    }

    /// Defender's moment, built: a handle with no sharing, let go of shortly
    /// after the open starts, is waited out; one that is never let go of is
    /// "busy", not "permission denied", and is reported inside a second.
    #[test]
    #[cfg(windows)]
    fn a_file_held_open_for_a_moment_is_waited_for_and_one_held_for_good_is_busy() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = TempDir::new("disk-open-held");
        let path = dir.write("held.md", b"# held\n");
        let hold = || {
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .expect("hold it open with no sharing")
        };

        let held = hold();
        let letting_go = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            drop(held);
        });
        let opened = open(&path, dir.path()).expect("waited for, then read");
        assert_eq!(opened.text, "# held\n");
        letting_go.join().expect("the holder let go");

        let held = hold();
        let started = std::time::Instant::now();
        assert_eq!(open(&path, dir.path()).map(|_| ()), Err(Refusal::Busy));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        drop(held);
    }
}
