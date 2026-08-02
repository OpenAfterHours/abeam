//! Getting a file's bytes, or a reason why not.
//!
//! Every path into this module is a path an agent chose, not one the user
//! typed, so the failure cases are the normal cases: the watcher fires on a
//! temp file that is gone by the time we read it, on a lock file, on a
//! 200-megabyte build artefact. None of that may take the pane down, so
//! `load` has no failure it reports by panicking and no path it reports by
//! returning something the caller has to interpret.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// Read at most this much. A viewer that faithfully loads a 200 MB file is a
/// viewer that hangs the agent's pane, and nobody reads the two-millionth line
/// of anything.
///
/// This is the only bound on how long the frame that opens a document can take,
/// so it is set from the *layout* cost rather than the read: reading is tens of
/// milliseconds a megabyte, but laying one out — parse, highlight, wrap, once
/// per width — was measured at 920 ms for 2 MiB of markdown in a release build,
/// and it repeats on every resize. 512 KiB is about 210 ms, and a document that
/// large is a generated report rather than something someone wrote. Going over
/// is not silent: the pane says "stopped at 512 KB of …" at the end of what it
/// did read.
pub const MAX_BYTES: u64 = 512 * 1024;

/// A NUL in the first block means binary — the same test `git` uses, and it is
/// right about everything except UTF-16, which is rare enough in a repo to
/// live with being called binary.
const SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub text: String,
    /// The file was longer than `MAX_BYTES` and this is the head of it. The
    /// pane says so on screen; silently showing a prefix is how you get a bug
    /// report about a document that "ends in the middle of a sentence".
    pub truncated: bool,
    /// Size on disk, which is not `text.len()` once truncation or lossy UTF-8
    /// decoding has happened.
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    Denied,
    NotAFile,
    Binary { bytes: u64 },
    Other(String),
}

impl LoadError {
    /// One line, shown in the pane. Phrased as what happened rather than as an
    /// error code, because the reader did not ask for this file — an agent did.
    pub fn message(&self) -> String {
        match self {
            LoadError::Missing => "no such file — it may have been renamed or deleted".into(),
            LoadError::Denied => "permission denied".into(),
            LoadError::NotAFile => "not a regular file".into(),
            LoadError::Binary { bytes } => {
                format!("binary file, {} — nothing to read here", human(*bytes))
            }
            LoadError::Other(msg) => msg.clone(),
        }
    }
}

pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    // Checked before opening, because on Windows opening a directory fails
    // with a bare access-denied and "permission denied" is a bad thing to tell
    // someone who pointed at a folder.
    let meta = std::fs::metadata(path).map_err(from_io)?;
    if !meta.is_file() {
        return Err(LoadError::NotAFile);
    }
    let bytes = meta.len();

    let file = File::open(path).map_err(from_io)?;
    let mut buf = Vec::new();
    // One byte past the cap, so the read itself tells us whether there was
    // more rather than trusting a size that may be stale.
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(from_io)?;

    if buf.iter().take(SNIFF_BYTES).any(|&b| b == 0) {
        return Err(LoadError::Binary { bytes });
    }

    let truncated = buf.len() as u64 > MAX_BYTES;
    if truncated {
        buf.truncate(MAX_BYTES as usize);
        // Back off to the last complete line: a cut mid-line reads as
        // corruption, and a cut mid-codepoint reads as a replacement glyph.
        if let Some(nl) = buf.iter().rposition(|&b| b == b'\n') {
            buf.truncate(nl + 1);
        }
    }

    Ok(Loaded {
        text: normalise(&String::from_utf8_lossy(&buf)),
        truncated,
        bytes,
    })
}

/// The whole error mapping, in one place and separately testable. Producing a
/// real ACL-denied file inside a unit test is a platform-specific fight; the
/// classification is the part that can actually be wrong.
pub fn from_io(err: io::Error) -> LoadError {
    match err.kind() {
        io::ErrorKind::NotFound => LoadError::Missing,
        io::ErrorKind::PermissionDenied => LoadError::Denied,
        io::ErrorKind::IsADirectory => LoadError::NotAFile,
        _ => LoadError::Other(err.to_string()),
    }
}

/// CRLF is the default on this platform and a stray `\r` renders as a hole in
/// the middle of the pane, so it goes before anything else sees the text.
fn normalise(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn reads_a_normal_file() {
        let dir = TempDir::new("load-normal");
        let path = dir.write("hello.md", b"# hi\n");
        let got = load(&path).expect("readable");
        assert_eq!(got.text, "# hi\n");
        assert!(!got.truncated);
        assert_eq!(got.bytes, 5);
    }

    #[test]
    fn a_missing_file_is_a_message_not_a_panic() {
        let dir = TempDir::new("load-missing");
        assert_eq!(
            load(&dir.path().join("nope.md")),
            Err(LoadError::Missing),
            "the watcher fires on files that are already gone"
        );
    }

    #[test]
    fn a_directory_is_not_a_file() {
        let dir = TempDir::new("load-dir");
        assert_eq!(load(dir.path()), Err(LoadError::NotAFile));
    }

    #[test]
    fn a_binary_file_is_refused_rather_than_rendered() {
        let dir = TempDir::new("load-binary");
        let path = dir.write("a.bin", &[0x7f, b'E', b'L', b'F', 0x00, 0x01, 0x02]);
        assert_eq!(load(&path), Err(LoadError::Binary { bytes: 7 }));
    }

    #[test]
    fn a_file_over_the_cap_is_truncated_and_says_so() {
        let dir = TempDir::new("load-big");
        let line = b"0123456789abcdef0123456789abcdef\n";
        let mut body = Vec::new();
        while (body.len() as u64) < MAX_BYTES + 4096 {
            body.extend_from_slice(line);
        }
        let path = dir.write("big.txt", &body);

        let got = load(&path).expect("readable");
        assert!(got.truncated);
        assert!(got.text.len() as u64 <= MAX_BYTES);
        assert_eq!(got.bytes, body.len() as u64);
        // Cut on a line boundary, not mid-line.
        assert!(got.text.ends_with('\n'));
    }

    #[test]
    fn invalid_utf8_is_shown_lossily_rather_than_refused() {
        // Not binary — no NUL — just not valid UTF-8. A latin-1 README should
        // still be readable, mojibake and all.
        let dir = TempDir::new("load-latin1");
        let path = dir.write("readme.txt", b"caf\xe9 au lait\n");
        let got = load(&path).expect("readable");
        assert!(got.text.starts_with("caf"));
        assert!(got.text.ends_with("au lait\n"));
    }

    #[test]
    fn crlf_is_normalised_before_anything_measures_a_column() {
        let dir = TempDir::new("load-crlf");
        let path = dir.write("dos.md", b"one\r\ntwo\r\n");
        assert_eq!(load(&path).expect("readable").text, "one\ntwo\n");
    }

    #[test]
    fn io_errors_map_to_the_right_notice() {
        use io::ErrorKind::*;
        assert_eq!(from_io(io::Error::from(NotFound)), LoadError::Missing);
        assert_eq!(from_io(io::Error::from(PermissionDenied)), LoadError::Denied);
        assert_eq!(from_io(io::Error::from(IsADirectory)), LoadError::NotAFile);
        // Anything unclassified still produces a message rather than nothing.
        assert!(!from_io(io::Error::from(UnexpectedEof)).message().is_empty());
    }

    #[test]
    fn sizes_read_as_sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2048), "2.0 KB");
        assert_eq!(human(3 * 1024 * 1024), "3.0 MB");
    }
}
