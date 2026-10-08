//! What the disk held when an editor was opened on it, for telling "nobody has
//! touched it" from "somebody has".
//!
//! **The bytes themselves, not a stamp of them**, which is a deliberate step
//! past the pad store's `Stamp`. The pad compares a length and a modification
//! time, and it is right to: its file is abeam's own, in the profile, and the
//! only other writer it has to notice is a second abeam window. A file in a
//! repository has an agent writing to it, and an agent can rewrite a file to
//! the same length inside one tick of a coarse clock — FAT's is two seconds,
//! and a network share's is whatever the server says — which a stamp would
//! call untouched and a save would then overwrite. Holding the bytes costs at
//! most `load::MAX_BYTES`, because nothing larger can be opened for editing,
//! and it makes the comparison exact: there is no "probably the same" for a
//! reviewer to reason about.
//!
//! [`Fingerprint`] is the same fact made small enough to write down, for the
//! recovery copy, which has to say which version of a file it was typed
//! against in a session that no longer has the bytes. FNV-1a and the length,
//! not `DefaultHasher`, for `crate::paths::workspace_key`'s reason: this number
//! is compared across processes and across toolchains, and the standard
//! library's hasher promises to be neither.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::str::FromStr;

/// The length of a file and the FNV-1a of its bytes: enough to say, later and
/// in another process, whether a file is still the one somebody was editing.
///
/// Not a security property and not offered as one — nothing here defends
/// against somebody *making* a file collide — and the length is in it so that
/// the commonest change of all, a file that grew or shrank, can never be a
/// collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub len: u64,
    pub hash: u64,
}

impl Fingerprint {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            len: bytes.len() as u64,
            hash: crate::paths::fnv1a_bytes(bytes),
        }
    }
}

/// `46213:0123456789abcdef` — the length in decimal, because that is the half
/// a person can check against a directory listing, and the hash in fixed-width
/// hex.
impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:016x}", self.len, self.hash)
    }
}

impl FromStr for Fingerprint {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let (len, hash) = s.split_once(':').ok_or(())?;
        if hash.len() != 16 {
            return Err(());
        }
        Ok(Self {
            len: len.parse().map_err(|_| ())?,
            hash: u64::from_str_radix(hash, 16).map_err(|_| ())?,
        })
    }
}

/// What a save expects to find at its target: nothing, or exactly these bytes.
///
/// [`open`](fn@crate::disk::open) hands one back with the text, and every
/// successful [`save`](fn@crate::disk::save) hands back the next — the bytes
/// it wrote — so a run of saves compares each against the last rather than
/// against the file as it was opened.
#[derive(Clone, PartialEq, Eq)]
pub enum Baseline {
    /// There was no file: one being created, or one deleted since it was read.
    Absent,
    /// There was a file and these were all of its bytes.
    Present(Box<[u8]>),
}

impl Baseline {
    /// The fingerprint of what was there, or `None` for a file that was not.
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        match self {
            Baseline::Absent => None,
            Baseline::Present(bytes) => Some(Fingerprint::of(bytes)),
        }
    }

    /// Whether `path` holds exactly this now: no file for [`Baseline::Absent`],
    /// these bytes and no others for [`Baseline::Present`].
    ///
    /// The comparison [`save`](fn@crate::disk::save) makes immediately before
    /// it replaces anything, and the one the files view makes when the watcher
    /// reports a path it has just saved, to tell its own write's echo from
    /// somebody else's. Reads at most one byte more than the baseline holds,
    /// which is enough to know a longer file is a different one.
    ///
    /// `Err` is "could not tell" — the file is there and would not open — and
    /// is never an answer either way. The caller decides what not knowing
    /// means; for a save it means not yet.
    pub fn is_on_disk(&self, path: &Path) -> io::Result<bool> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(why) if why.kind() == io::ErrorKind::NotFound => {
                return Ok(matches!(self, Baseline::Absent));
            }
            Err(why) => return Err(why),
        };
        let Baseline::Present(want) = self else {
            return Ok(false);
        };
        let mut got = Vec::with_capacity(want.len() + 1);
        file.take(want.len() as u64 + 1).read_to_end(&mut got)?;
        Ok(got[..] == want[..])
    }
}

/// The fingerprint rather than the bytes: a failed assertion on a 500 KB file
/// should print a line, not the file.
impl fmt::Debug for Baseline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Baseline::Absent => f.write_str("Baseline::Absent"),
            Baseline::Present(bytes) => {
                write!(f, "Baseline::Present({})", Fingerprint::of(bytes))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn a_fingerprint_is_written_down_and_read_back_as_itself() {
        let print = Fingerprint::of(b"# Notes\r\n");
        assert_eq!(print.len, 9);
        assert_eq!(print.to_string().parse::<Fingerprint>(), Ok(print));
        assert_eq!(
            Fingerprint::of(b"foobar").to_string(),
            "6:85944171f73967e8",
            "the published FNV-1a vector, so anybody can recompute one"
        );
        for bad in [
            "",
            "6",
            "6:",
            "x:85944171f73967e8",
            "6:85944171f73967e",
            "6-85944171f73967e8",
        ] {
            assert_eq!(bad.parse::<Fingerprint>(), Err(()), "{bad:?}");
        }
    }

    #[test]
    fn the_disk_is_compared_byte_for_byte_and_a_missing_file_is_an_answer() {
        let dir = TempDir::new("disk-baseline");
        let path = dir.write("a.txt", b"one\ntwo\n");
        let seen = Baseline::Present(b"one\ntwo\n".to_vec().into());
        assert!(seen.is_on_disk(&path).expect("readable"));
        assert!(!Baseline::Absent.is_on_disk(&path).expect("readable"));

        // The same length, a different byte: what a stamp would call
        // untouched inside one tick of a coarse clock.
        std::fs::write(&path, b"one\ntwO\n").expect("rewrite it");
        assert!(!seen.is_on_disk(&path).expect("readable"));
        // Longer by one, and shorter by one.
        std::fs::write(&path, b"one\ntwo\n\n").expect("rewrite it");
        assert!(!seen.is_on_disk(&path).expect("readable"));
        std::fs::write(&path, b"one\ntwo").expect("rewrite it");
        assert!(!seen.is_on_disk(&path).expect("readable"));

        std::fs::remove_file(&path).expect("delete it");
        assert!(!seen.is_on_disk(&path).expect("absence is an answer"));
        assert!(Baseline::Absent.is_on_disk(&path).expect("absence is an answer"));
        assert_eq!(Baseline::Absent.fingerprint(), None);
        assert_eq!(seen.fingerprint(), Some(Fingerprint::of(b"one\ntwo\n")));
    }
}
