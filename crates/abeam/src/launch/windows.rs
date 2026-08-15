//! What abeam is allowed to hand to `CreateProcessW`, and how a script shim is
//! turned into something it will take.
//!
//! *Where* to look for the file is the other half of the question, it is the
//! same hazard on every platform, and it lives in the parent module. This half
//! is Windows' alone.
//!
//! abeam hands `CreateProcessW` an `.exe` or a `.com` and nothing else —
//! [`IMAGES`] is abeam's own short list rather than Windows'. That is fine
//! until you meet an npm install, which drops three files into `%APPDATA%\npm`:
//! `claude` (a POSIX shell script, no extension), `claude.cmd` and `claude.ps1`.
//! Preferring the `.cmd` on its own does not help — `CreateProcessW` cannot run
//! that either — so a `.cmd` is started by naming `cmd.exe` in front of it. The
//! npm route is the one this module is for rather than the only route there is:
//! GitHub Copilot's CLI also arrives as a plain executable, from
//! `winget install GitHub.Copilot` or from `gh copilot`, and that copy never
//! reaches this module at all. Its npm package is what a machine without either
//! of those has, and for that machine this is the difference between "hostable"
//! and "not".
//!
//! ## Why the command line travels in an environment variable
//!
//! `cmd.exe` re-parses whatever follows `/c`, and `&  |  <  >  ^  (  )  %  !`
//! and `"` are syntax to it. Getting that right is the whole of
//! [`command_line`], which follows Rust's own `make_bat_command_line` — the fix
//! for CVE-2024-24576, "BatBadBut" — almost line for line.
//!
//! What it does *not* follow is how std delivers the result. std builds the
//! entire command line itself and hands it to `CreateProcessW` raw. abeam
//! cannot: it spawns through portable-pty, whose `CommandBuilder` applies
//! MSVCRT argv quoting to every argument it is given, and MSVCRT quoting spells
//! an embedded quote `\"`. `cmd` has no backslash escape, so
//! `cmd /d /s /c "\"C:\…\claude.cmd\" foo"` reaches the child as the literal
//! `'\"C:\…\claude.cmd\"' is not recognized as an internal or external
//! command`. That was measured, not guessed. The outer quote pair that `/s`
//! exists to strip cannot be produced either, because portable-pty only ever
//! emits quotes in balanced pairs around whole arguments.
//!
//! So the command line is put in `%ABEAM_LAUNCH%` and the wire carries one
//! token that portable-pty has nothing to do to: `%ABEAM_LAUNCH%` contains no
//! space and no quote, so it is passed through byte for byte. `cmd` expands it
//! after it has finished deciding what to strip, which is also why the `%` half
//! of std's escaping is deliberately absent below — see [`command_line`].
//!
//! Two other routes were tried against a real `cmd.exe` first and are recorded
//! because they look right:
//!
//! - **`cmd /d /s /c call "script" args`**, splitting the command line into one
//!   portable-pty argument per token so that its quoting becomes the quoting
//!   `cmd` wants. It survives spaces, but an argument containing a `"` still
//!   arrives as `\"`, which does not merely mangle the argument — it desyncs
//!   `cmd`'s quote tracking, so a `&` later on the line becomes a command
//!   separator. `call` also re-parses its own line, consuming carets a second
//!   time.
//! - **Caret-escaping the spaces** in the script's path, which genuinely works
//!   for the command name. It does nothing for the arguments: `cmd` strips the
//!   carets before `%*` is formed, so the space is a separator again by the time
//!   the shim forwards it.

use std::path::{Path, PathBuf};

use super::Launch;

// The moved suite below reaches for four things that stayed in the parent
// module when this file was split out of it, and none of them is used by the
// code above the suite. Imported here rather than at the top of `mod tests` so
// that the tests are the file they were before the split, line for line — the
// only way a reader can check a move by reading it.
#[cfg(test)]
use super::{find, resolve, walk};
#[cfg(test)]
use abeam_pty::PtyConfig;
#[cfg(test)]
use std::ffi::OsStr;

/// The extensions abeam will hand to `CreateProcessW` on their own.
///
/// Not "the extensions `CreateProcessW` will start": it starts any valid PE
/// image whatever the file is called, an extension it has never heard of or no
/// extension at all included. This is abeam's own allowlist and it is
/// deliberately conservative — a name that resolved to a `.dat` is far more
/// likely to be a lookup that went wrong than a program somebody renamed, and
/// saying so is cheaper than starting it and finding out.
const IMAGES: &[&str] = &["exe", "com"];

/// The extensions abeam knows an interpreter for. Both are `cmd.exe`'s, and
/// they behave identically once it is running; the distinction is historical
/// and Windows keeps both alive.
const SCRIPTS: &[&str] = &["cmd", "bat"];

/// `PATHEXT`'s default, for the rare environment that does not set it. The
/// whole list, not an opinion about it: what abeam can *start* is [`IMAGES`],
/// and the two questions are answered separately on purpose.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Where the command line for a routed script is left for `cmd.exe` to pick up.
/// The child inherits it, which is harmless and occasionally the fastest way to
/// see what abeam actually ran.
const LAUNCH_VAR: &str = "ABEAM_LAUNCH";

/// The longest command line `cmd.exe` will actually run when `%ABEAM_LAUNCH%`
/// expands to it.
///
/// Measured rather than taken from the documented 8191, because 8191 is not
/// where the behaviour changes and the difference is the dangerous part. Against
/// a real `cmd.exe`: up to 8124 characters it runs; from 8125 to 8191 it prints
/// `The syntax of the command is incorrect.` and exits 255; **from 8192 it runs
/// nothing at all, prints nothing, and exits 0**. That last one is the reason
/// this constant exists — a child that never started, reported to whatever is
/// scripting abeam as a success. So the promise is the longest length that was
/// seen to work, not the longest the documentation names.
const MAX_LINE: usize = 8124;

/// The file [`super::find`] settled on, turned into something `CreateProcessW`
/// will take — or into the sentence explaining why there is not one.
pub(super) fn into_launch(found: PathBuf, args: &[String]) -> Result<Launch, String> {
    if is_image(&found) {
        return Ok(Launch {
            program: found.clone(),
            target: found,
            args: args.to_vec(),
            env: Vec::new(),
        });
    }
    if has_extension(&found, SCRIPTS) {
        return through_cmd(found, args);
    }

    // A `.ps1` is refused rather than routed, and it is the one refusal here
    // that is a *choice* rather than a limit. `powershell` and `pwsh` are two
    // different programs with two different profiles and two different
    // execution policies, and picking one for you is picking which of your
    // profiles runs. Nothing is lost by declining: an npm install puts a `.cmd`
    // next to every `.ps1` it writes and [`probe`] prefers anything it can
    // start to the `.ps1` beside it, so arriving here means the sibling that
    // would have worked is missing.
    //
    // It is *not* only reached by being named outright. `.PS1` is absent from
    // the default `PATHEXT`, which is where that claim came from, but a user
    // who has added it — and people do, to make `foo.ps1` runnable as `foo` —
    // puts every `.ps1` on the machine within reach of a bare name.
    if has_extension(&found, &["ps1"]) {
        return Err(format!(
            "`{}` is a PowerShell script, and abeam will not guess which \
             PowerShell you meant: `powershell` and `pwsh` load different \
             profiles under different execution policies. Name the .cmd beside \
             it, or the shell you want with the script as its argument.",
            found.display()
        ));
    }

    // Reached by the extensionless npm shim, by a `.js`, and by anything else
    // `PATHEXT` happens to list. Naming the file rather than the name that was
    // asked for is the point: `claude` resolving to a file called `claude` with
    // no extension is the whole diagnosis.
    //
    // Two shapes arrive here and the sentence has to be true of both. Telling
    // somebody holding a `claude.js` that "a file with no extension is a POSIX
    // shell script" reads as abeam having looked at a different file, which is
    // the one impression an error message must never give.
    let because = match found.extension() {
        None => "a file with no extension is a POSIX shell script, and there is \
                 no shell on Windows that is the right one to hand it to"
            .to_string(),
        Some(ext) => format!(
            "`.{}` is none of those, and abeam will not pick an interpreter for \
             an extension it does not know",
            ext.to_string_lossy().to_ascii_lowercase()
        ),
    };
    Err(format!(
        "`{}` is not a program Windows can start. `CreateProcessW` runs .exe \
         and .com, and abeam runs .cmd and .bat through cmd.exe; {because}.",
        found.display()
    ))
}

/// A `.cmd` or a `.bat`, with `cmd.exe` named in front of it.
fn through_cmd(script: PathBuf, args: &[String]) -> Result<Launch, String> {
    let interpreter = interpreter()?;
    let line = command_line(&script, args)?;
    Ok(Launch {
        program: interpreter,
        target: script,
        // `/e:ON` because an npm shim is not a toy batch file — it uses `||`
        // and `%~dp0`, and both are command extensions. `/v:OFF` so that a `!`
        // inside an argument is a `!` rather than the start of a delayed
        // expansion. `/d` so that whatever somebody left in the AutoRun
        // registry key does not run first, inside abeam, as the user. `/c` last
        // because everything after it is the command.
        //
        // No `/s`: it forces the "strip the first and last quote" rule, and
        // that rule only fires when the first character after `/c` is a quote.
        // Here it is always `%`, so `/s` would be a no-op that a later reader
        // has to work out is a no-op.
        args: ["/e:ON", "/v:OFF", "/d", "/c", &format!("%{LAUNCH_VAR}%")]
            .iter()
            .map(|a| (*a).to_string())
            .collect(),
        env: vec![(LAUNCH_VAR.to_string(), line)],
    })
}

/// `cmd.exe`, found the same careful way as everything else here.
///
/// The copy Windows shipped first, and `%ComSpec%` only behind it. That is the
/// opposite of what a shell does and it is deliberate: abeam is not looking for
/// the command processor somebody chose, it is looking for *a* command
/// processor to expand one shim, and this is the one lookup the user did not
/// ask for. Honouring a customised `ComSpec` therefore buys nothing here while
/// widening the set of things abeam trusts — which is the same conclusion Rust
/// std reached in the code this module's quoting is taken from: its
/// `command_prompt()` goes to `GetSystemDirectoryW` and never reads `ComSpec`
/// at all. `ComSpec` is kept as a fallback rather than dropped so that a system
/// which has genuinely relocated `System32` still starts its scripts.
///
/// Both are gated identically: an absolute path, to a file, with an extension
/// [`IMAGES`] allows. A relative `ComSpec` would be the bare-name hole again
/// one level down.
fn interpreter() -> Result<PathBuf, String> {
    interpreter_from(
        std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("cmd.exe")),
        std::env::var_os("ComSpec").map(PathBuf::from),
    )
}

/// The choice itself, over the two candidates handed in rather than read.
///
/// Split out for the same reason [`super::walk`] is: the process environment
/// belongs to the whole test binary, and a test that set `ComSpec` to prove the
/// order would be setting it for the two hundred tests running beside it.
fn interpreter_from(shipped: Option<PathBuf>, comspec: Option<PathBuf>) -> Result<PathBuf, String> {
    shipped
        .into_iter()
        .chain(comspec)
        .find(|path| path.is_absolute() && is_image(path) && path.is_file())
        .ok_or_else(|| {
            "abeam has no cmd.exe to run this script with: neither \
             %SystemRoot%\\System32\\cmd.exe nor %ComSpec% names a program that \
             is there."
                .to_string()
        })
}

/// The command line `cmd.exe` is asked to run: the script, then the caller's
/// arguments, quoted so that the child sees exactly the arguments abeam was
/// given and `cmd` sees no syntax at all.
///
/// This is Rust's own `make_bat_command_line` / `append_bat_arg`, the fix for
/// CVE-2024-24576, with two deliberate differences.
///
/// The first is the outer `cmd.exe /e:ON /v:OFF /d /c "` … `"` wrapper, which
/// is not built here because it is not on the wire — see the module docs.
///
/// The second is std's `%` escaping, and it is worth being precise about why it
/// is absent, because putting it back is the obvious "fix" for a bug that does
/// not exist. std replaces every `%` with `%%cd:~,%%` — a zero-length substring
/// of `%cd%`, which expands to nothing and distracts `cmd` from expanding
/// `%VAR%`. It needs that because its command line reaches `cmd` as literal
/// text, and `cmd` expands `%` in phase one. Here the command line *arrives by*
/// that expansion, and `cmd`'s expansion is a single left-to-right pass which
/// does not rescan what it has just substituted, so `%VAR%` in an argument is
/// already literal. Adding the escape would send the child the seven characters
/// `%%cd:~,` it never asked for. Both halves of that were measured against a
/// real `cmd.exe`.
///
/// Quoting is deliberately eager: anything that is not alphanumeric and not one
/// of a short list known to be harmless gets the argument quoted, rather than
/// abeam trying to enumerate every character `cmd` treats as syntax and being
/// one short.
fn command_line(script: &Path, args: &[String]) -> Result<String, String> {
    let name = script.to_string_lossy();
    // Windows file names cannot contain either, so a path that does is not a
    // path to anything — but it *would* close the quote below and turn the rest
    // of the line into syntax, so it is refused rather than escaped.
    if name.contains('"') || name.ends_with('\\') {
        return Err(format!(
            "`{name}` cannot be started through cmd.exe: a path it can quote may \
             not contain a `\"` or end with a `\\`."
        ));
    }

    let mut line = format!("\"{name}\"");
    for arg in args {
        // Nothing escapes these for `cmd`. A newline ends the command outright
        // and a carriage return truncates it, so an argument carrying one has
        // to be turned away with a sentence — silently dropping the tail is how
        // an argument injection reads from the outside.
        if let Some(bad) = arg.chars().find(|c| matches!(c, '\0' | '\r' | '\n')) {
            return Err(format!(
                "an argument contains {}, which cannot be passed through cmd.exe \
                 to a .cmd or .bat program at all.",
                match bad {
                    '\0' => "a NUL",
                    '\r' => "a carriage return",
                    _ => "a newline",
                }
            ));
        }
        line.push(' ');
        append_arg(&mut line, arg);
    }

    // Counted once the whole line is built, because what has to fit is what
    // `cmd` expands rather than what the caller typed: the quoting above adds
    // characters, and the script's own path is on the line too.
    //
    // Refused with a sentence, exactly as a NUL or a newline is, and for a
    // stronger version of the same reason. Those mangle the command; this one
    // does not even fail — past 8191 characters `cmd` runs nothing, says
    // nothing and exits 0, so an over-long argument drew an empty pane and
    // reported success. It is data-dependent and invisible, which is the worst
    // shape a limit can have.
    if line.len() > MAX_LINE {
        return Err(format!(
            "the command line for `{name}` comes to {} characters, and cmd.exe \
             will not run one longer than {MAX_LINE}: beyond that it either \
             refuses the line or — worse — starts nothing at all and reports \
             success. That is cmd.exe's limit and not abeam's, and it applies \
             because this program is a .cmd or .bat shim that has to be started \
             through it. The same agent installed natively is an .exe abeam \
             starts directly, and takes roughly four times as much. Shorten the \
             arguments, or hand the long one over in a file.",
            line.len()
        ));
    }
    Ok(line)
}

/// One argument, quoted for `cmd` and for the argv parser behind it.
///
/// The two have to be satisfied at once. `cmd` hands the shim its arguments as
/// text — an npm shim ends `"%_prog%" "%dp0%\…\cli.js" %*` — so what node
/// eventually parses is this same string, by the MSVCRT rules. `""` is the one
/// spelling of an embedded quote both understand: `cmd` reads the pair as a
/// literal quote inside a quoted region, and every CRT since Visual C++ 2008
/// does the same.
fn append_arg(line: &mut String, arg: &str) {
    /// Not "the characters `cmd` treats as syntax" — the characters known not
    /// to be. An unquoted `\` is fine so long as the argument is not otherwise
    /// quoted, which is why it is here rather than in the list that forces it.
    const UNQUOTED: &str = r"#$*+-./:?@\_";

    // An empty argument would vanish entirely without a pair of quotes to be.
    // A trailing `\` needs them for the opposite reason: a shim that writes
    // `"%~2"` around it would otherwise have the backslash escape its own
    // closing quote.
    let quote = arg.is_empty()
        || arg.ends_with('\\')
        || arg.chars().any(|c| {
            c.is_control() || (c.is_ascii() && !(c.is_ascii_alphanumeric() || UNQUOTED.contains(c)))
        });

    if quote {
        line.push('"');
    }
    // A backslash is only an escape when a quote follows it, so runs of them
    // are counted and doubled at exactly the two places one can.
    let mut backslashes = 0usize;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                // 2n before the quote, then a second quote to escape it.
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push('"');
            }
            backslashes = 0;
        }
        line.push(c);
    }
    if quote {
        // 2n before the closing quote, so the run stays a run rather than
        // escaping the quote that ends the argument.
        line.extend(std::iter::repeat_n('\\', backslashes));
        line.push('"');
    }
}

// --- what a name means in a directory -------------------------------------

/// `name` inside `dir`, with whatever extension makes it startable.
pub(super) fn probe(dir: &Path, name: &str) -> Option<PathBuf> {
    probe_with(dir, name, &pathext())
}

/// The search inside one directory, over a `PATHEXT` handed in rather than
/// read — split out for the same reason [`super::walk`] is, so that a test can
/// pin what a customised one does without writing to an environment the whole
/// test binary shares.
///
/// `PATHEXT` matches come before the bare file, deliberately: an npm install
/// leaves `claude`, `claude.cmd` and `claude.ps1` side by side, and the
/// extensionless one is a POSIX shell script. Taking it first — which is what
/// portable-pty's own search does — is a program that cannot start. That stayed
/// load-bearing after `.cmd` became launchable, because the two are still in
/// the same directory and only one of them is the answer.
///
/// Within the `PATHEXT` matches abeam then departs from Windows' own rule,
/// which is to take the first match and stop. abeam takes the first match it
/// can actually **start**, which is not always the first match. The departure
/// is invisible on the default `PATHEXT` and is the whole difference between
/// hosting an npm agent and not on a customised one: with the real npm trio on
/// `PATH`, `.PS1;.COM;.EXE;.BAT;.CMD` made `abeam +claude` refuse a PowerShell
/// script and `.COM;.EXE;.JS;.CMD` made it refuse a `.js`, with the launchable
/// `claude.cmd` sitting in the same directory both times. Windows' order
/// answers "what does this name mean on this machine", which is a question
/// about association; the user asked abeam to run their agent, and in that
/// directory there is only ever one file that can be run.
///
/// When none of them can be started the answer is the one worth a sentence
/// rather than the first one found. A `.ps1` is preferred there because its
/// refusal is the only one that names the sibling which would have worked; a
/// `.js` or a bare POSIX shim can only be described.
fn probe_with(dir: &Path, name: &str, pathext: &[String]) -> Option<PathBuf> {
    let present: Vec<PathBuf> = pathext
        .iter()
        .map(|ext| dir.join(format!("{name}{ext}")))
        .chain(std::iter::once(dir.join(name)))
        .filter(|file| file.is_file())
        .collect();

    present
        .iter()
        .find(|file| startable(file))
        .or_else(|| present.iter().find(|file| has_extension(file, &["ps1"])))
        .or_else(|| present.first())
        .cloned()
}

/// A file this module has a way to run: `CreateProcessW` takes an [`IMAGES`]
/// one as it stands, and a [`SCRIPTS`] one goes through `cmd.exe`.
pub(super) fn startable(path: &Path) -> bool {
    is_image(path) || has_extension(path, SCRIPTS)
}

/// `PATHEXT` spelled in lower case.
///
/// Windows sets it in capitals and matches file names without regard to case,
/// so the only thing the choice affects is the path abeam then *shows* — in the
/// message about a file it will not start, and in the pty diagnostics.
/// `claude.cmd` is what a reader has on disk; `claude.CMD` is a second thing to
/// wonder about.
fn pathext() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| DEFAULT_PATHEXT.to_string())
        .split(';')
        .filter(|ext| ext.starts_with('.'))
        .map(str::to_ascii_lowercase)
        .collect()
}

fn has_extension(path: &Path, list: &[&str]) -> bool {
    path.extension()
        .is_some_and(|ext| list.iter().any(|want| ext.eq_ignore_ascii_case(want)))
}

fn is_image(path: &Path) -> bool {
    has_extension(path, IMAGES)
}

/// Windows-only like the rest of the suite, the quoting tests included — what
/// they are about is a `PATH` walk, a `PATHEXT`, and a parser that only ships
/// on this platform.
///
/// Be clear about what those quoting tests are, though, because it is easy to
/// read more into them than is there: each one asserts the exact string abeam
/// builds, and none of them runs `cmd.exe`. They are the *claim*. The proof is
/// the two spawning tests at the end, which put a real shim in a real pty and
/// read back what arrived — so an argument shape that matters is not properly
/// covered here until it appears down there as well.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    /// The command line for `claude.cmd` with these arguments, as one string.
    fn line(list: &[&str]) -> String {
        command_line(Path::new(r"C:\npm\claude.cmd"), &args(list)).expect("a quotable argument")
    }

    /// A `PATH` out of these directories, joined the way this platform joins
    /// one. Written out rather than built with `std::env::join_paths`, which
    /// returns a `Result` for a case (a directory with a `;` in its name) that
    /// no test here has and that would only add an `unwrap` to read past.
    fn path_of(dirs: &[&Path]) -> String {
        dirs.iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(";")
    }

    /// A relative `PATH` entry that really does hold a file, and the name of
    /// that file — so that a walk which had stopped refusing relative entries
    /// would answer `Some` rather than `None`.
    ///
    /// Discovered rather than written down, the same way
    /// `a_hint_that_is_not_absolute_is_trusted_no_further_than_a_path_entry_is`
    /// finds its own. `cargo test` runs with the crate directory as the current
    /// one, so `("src", "main.rs")` would do — and a test that pins the layout
    /// of the crate it lives in fails the day somebody moves a file, for a
    /// reason that has nothing to do with what it is about.
    fn a_relative_entry_holding_a_file() -> (PathBuf, String) {
        let here = std::env::current_dir().expect("a current directory");
        std::fs::read_dir(&here)
            .expect("read the current directory")
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .find_map(|entry| {
                let inside = std::fs::read_dir(entry.path())
                    .ok()?
                    .flatten()
                    .find(|inner| inner.path().is_file())?;
                Some((
                    PathBuf::from(entry.file_name()),
                    inside.file_name().to_string_lossy().into_owned(),
                ))
            })
            .expect("the current directory has a subdirectory with a file in it")
    }

    // --- what may be started ---------------------------------------------

    #[test]
    fn a_program_is_never_taken_from_the_directory_abeam_is_looking_at() {
        // The bug this whole module exists for. portable-pty hands a bare name
        // it could not find on PATH to `CreateProcessW` unchanged, and Windows
        // resolves that against the *calling process's* current directory — so
        // a repository containing `claude.exe` used to be what abeam started.
        //
        // Demonstrated against a `PATH` handed in rather than by standing in
        // the planted directory: the current directory belongs to the whole
        // test binary, and two hundred other tests are running beside this one.
        // What has to be true is that no entry which could name the current
        // directory is ever looked in, and these are all of them.
        let dir = TempDir::new("launch-planted");
        dir.write("abeam-planted.exe", b"MZ not really a program");
        let planted = "abeam-planted.exe";

        for hostile in [";;", ";", ".", ".;", r".\tools", "..", ""] {
            assert_eq!(
                walk(OsStr::new(hostile), planted),
                None,
                "PATH {hostile:?} was searched, and it names the current directory"
            );
        }

        // The loop above is necessary and it is not sufficient, and this test
        // claimed the opposite of that for as long as it existed. Nothing
        // called `abeam-planted.exe` exists relative to the test binary's
        // current directory either — the planted copy is in a temp directory —
        // so every one of those entries answers `None` whether or not `walk`
        // refuses a relative one. Delete `.filter(|dir| dir.is_absolute())` and
        // the loop stays green, which is the one thing a test of a filter must
        // not do.
        //
        // What catches it is a relative entry that genuinely holds a file: with
        // the filter it is `None`, and without it the walk returns the file.
        let (relative, name) = a_relative_entry_holding_a_file();
        assert!(
            relative.join(&name).is_file(),
            "the entry does name a file, relatively"
        );
        assert_eq!(
            walk(relative.as_os_str(), &name),
            None,
            "a relative PATH entry was searched, and what it names is the \
             current directory"
        );

        // ...and an absolute entry still is, so the refusals above are the
        // filter choosing rather than the walk having stopped working.
        assert_eq!(
            walk(dir.path().as_os_str(), planted),
            Some(dir.path().join(planted)),
            "an absolute PATH entry is still searched"
        );

        // The end of the same story: nothing on PATH, so nothing is started —
        // where portable-pty would have passed the bare name through.
        assert!(resolve(planted, &[]).is_err());
    }

    #[test]
    fn a_file_windows_cannot_start_does_not_shadow_the_program_further_along_path() {
        // [`super::walk`]'s second pass, pinned on this platform. It was added
        // for Unix — where a directory holds one candidate and [`probe_with`]
        // has nothing to choose between, so the preference had to be lifted to
        // the walk — and because `walk` is shared it changed Windows too: an
        // earlier `PATH` entry holding only something abeam cannot start no
        // longer hides a later entry holding the program. That was previously
        // an error, and it is a behaviour change worth a test of its own rather
        // than one inherited from a suite that does not run here. Without this,
        // the whole two-pass could be deleted and every Windows test would
        // still pass.
        //
        // The extensionless npm shim is the file to plant, because `probe_with`
        // finds it whatever `PATHEXT` says — it chains the bare name after the
        // extensions — and `startable` refuses it however it was found. A
        // `.ps1` would depend on a `PATHEXT` this machine may not have.
        let earlier = TempDir::new("launch-shadow-earlier");
        let later = TempDir::new("launch-shadow-later");
        let unusable = earlier.write("abeam-agent", b"#!/bin/sh\nexec node cli.js\n");
        let usable = later.write("abeam-agent.cmd", b"@echo off\r\n");

        assert_eq!(
            walk(
                OsStr::new(&path_of(&[earlier.path(), later.path()])),
                "abeam-agent"
            ),
            Some(usable),
            "a POSIX shim earlier on PATH hid the .cmd behind it"
        );

        // ...and with nothing startable anywhere on it, the file that is there
        // is still the one named, because the sentence it produces is the one
        // the user can act on: the whole difference between "`abeam-agent` was
        // not found on PATH" and being told what the file in `%APPDATA%\npm`
        // actually is.
        assert_eq!(
            walk(OsStr::new(&path_of(&[earlier.path()])), "abeam-agent"),
            Some(unusable.clone()),
            "the only copy on PATH was dropped, leaving nothing to name"
        );
        let refused = into_launch(unusable, &[]).expect_err("it still cannot be started");
        assert!(refused.contains("abeam-agent"), "got: {refused}");
        assert!(refused.contains("POSIX"), "got: {refused}");
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_resolved() {
        // `.\tools\sh.exe` would be resolved against the repository on screen,
        // which is the one directory in this question somebody else gets to
        // write to. Refusing says so; resolving would not.
        let refused =
            resolve(r".\tools\sh.exe", &[]).expect_err("a relative path is not a program");
        assert!(refused.contains("relative"), "got: {refused}");
        assert!(refused.contains("absolute"), "the way out has to be named");
    }

    #[test]
    fn an_extensionless_script_never_wins_over_an_executable_beside_it() {
        // An npm install drops `claude`, `claude.cmd` and `claude.ps1` in one
        // directory, and the extensionless one is a POSIX shell script
        // `CreateProcessW` cannot run. portable-pty's own search checks the
        // exact name first and takes it.
        let dir = TempDir::new("launch-pathext");
        dir.write("abeam-probe", b"#!/bin/sh\n");
        dir.write("abeam-probe.exe", b"MZ");
        assert_eq!(
            probe(dir.path(), "abeam-probe"),
            Some(dir.path().join("abeam-probe.exe"))
        );

        // The npm layout exactly: the `.cmd` is the one that can be started,
        // and it still has to beat the shell script sitting next to it.
        let npm = TempDir::new("launch-npm");
        npm.write("abeam-probe", b"#!/bin/sh\n");
        npm.write("abeam-probe.cmd", b"@echo off\r\n");
        assert_eq!(
            probe(npm.path(), "abeam-probe"),
            Some(npm.path().join("abeam-probe.cmd"))
        );

        // With nothing else beside it, the script is still what is there —
        // `resolve` is what turns that into a sentence rather than a spawn.
        let only = TempDir::new("launch-pathext-only");
        only.write("abeam-probe", b"#!/bin/sh\n");
        assert_eq!(
            probe(only.path(), "abeam-probe"),
            Some(only.path().join("abeam-probe"))
        );
    }

    #[test]
    fn a_customised_pathext_still_reaches_the_one_file_of_the_npm_trio_that_can_start() {
        // Both of the last two are real `PATHEXT` values off real machines, and
        // both used to stop `abeam +claude` dead with the launchable `.cmd`
        // sitting in the same directory: the first took the `.ps1` and refused
        // to guess a PowerShell, the second took the `.js` and said it was not
        // a program Windows can start. Windows' own rule is "first match wins";
        // abeam's is "first match that can be started wins", and this is what
        // that buys.
        let dir = TempDir::new("launch-pathext-custom");
        dir.write("abeam-agent", b"#!/bin/sh\n");
        let shim = dir.write("abeam-agent.cmd", b"@echo off\r\n");
        dir.write("abeam-agent.ps1", b"# nothing\r\n");
        dir.write("abeam-agent.js", b"// nothing\n");

        for list in [
            ".com;.exe;.bat;.cmd",
            ".ps1;.com;.exe;.bat;.cmd",
            ".com;.exe;.js;.cmd",
        ] {
            let exts: Vec<String> = list.split(';').map(str::to_string).collect();
            assert_eq!(
                probe_with(dir.path(), "abeam-agent", &exts),
                Some(shim.clone()),
                "PATHEXT {list} did not reach the .cmd"
            );
        }
    }

    #[test]
    fn when_nothing_in_the_directory_can_be_started_the_one_named_is_the_one_worth_a_sentence() {
        // Windows' order would take the `.js`, and all abeam can say about a
        // `.js` is that it is not a program. The `.ps1` refusal names the
        // sibling that would have worked and the way out, so that is the file
        // to be holding when the message is written.
        let dir = TempDir::new("launch-pathext-none");
        dir.write("abeam-agent.js", b"// nothing\n");
        dir.write("abeam-agent.ps1", b"# nothing\r\n");
        let exts: Vec<String> = [".js", ".ps1"].iter().map(|e| e.to_string()).collect();

        let found = probe_with(dir.path(), "abeam-agent", &exts).expect("both are there");
        assert!(
            found.ends_with("abeam-agent.ps1"),
            "got: {}",
            found.display()
        );

        // ...and with no `.ps1` in the directory it is the first match, which
        // is the only thing left to name.
        let js = TempDir::new("launch-pathext-js");
        let only = js.write("abeam-agent.js", b"// nothing\n");
        assert_eq!(probe_with(js.path(), "abeam-agent", &exts), Some(only));
    }

    #[test]
    fn a_hint_that_is_not_absolute_is_trusted_no_further_than_a_path_entry_is() {
        // `panes::shell`'s `known_home` joins onto `%SystemRoot%` and
        // `%ProgramFiles%`, so a relative or empty one of those hands this
        // module a relative path — and `is_file()` on one asks about the
        // current directory, which under abeam is the repository on screen.
        // Demonstrated without standing anywhere in particular: a file that
        // exists relative to wherever the test binary happens to be is exactly
        // the shape of answer that must not be taken.
        let here = std::env::current_dir().expect("a current directory");
        let relative = std::fs::read_dir(&here)
            .expect("read the current directory")
            .flatten()
            .find(|entry| entry.path().is_file())
            .map(|entry| PathBuf::from(entry.file_name()))
            .expect("the current directory has a file in it");
        assert!(
            relative.is_file(),
            "the hint does name something, relatively"
        );

        assert!(
            find("abeam-no-such-hinted-program", Some(relative)).is_err(),
            "a relative hint was taken, and what it names is the current directory"
        );

        // The hint still works when it is what it is supposed to be, so the
        // refusal above is the check rather than the hint being ignored.
        let dir = TempDir::new("launch-hint");
        let exe = dir.write("abeam-hinted.exe", b"MZ");
        assert_eq!(
            find("abeam-hinted", Some(exe.clone())).expect("an absolute hint"),
            exe
        );
    }

    #[test]
    fn the_command_processor_is_the_one_windows_shipped_and_comspec_is_only_the_fallback() {
        // abeam wants *a* command processor to expand a shim, not the
        // interactive shell somebody chose, and this is the one lookup the user
        // did not ask for — so a customised `%ComSpec%` buys nothing here and
        // widens what abeam trusts. Rust std's `command_prompt()`, the code
        // this module's quoting comes from, reaches the same conclusion by
        // reading `GetSystemDirectoryW` and never consulting `ComSpec` at all.
        let dir = TempDir::new("launch-comspec");
        let shipped = dir.write("abeam-shipped.exe", b"MZ");
        let comspec = dir.write("abeam-comspec.exe", b"MZ");

        assert_eq!(
            interpreter_from(Some(shipped.clone()), Some(comspec.clone())).unwrap(),
            shipped,
            "%ComSpec% won, and it is the fallback"
        );

        // A fallback rather than nothing, so a system that has genuinely
        // relocated System32 still starts its scripts.
        assert_eq!(
            interpreter_from(
                Some(dir.path().join("abeam-absent.exe")),
                Some(comspec.clone())
            )
            .unwrap(),
            comspec
        );

        // Gated identically, both of them. A relative one is the bare-name hole
        // a level down, and something that is not an image is not an
        // interpreter however absolute it is.
        assert_eq!(
            interpreter_from(
                Some(PathBuf::from(r"Windows\System32\cmd.exe")),
                Some(comspec)
            )
            .unwrap(),
            dir.path().join("abeam-comspec.exe")
        );
        let text = dir.write("abeam-shipped.txt", b"MZ");
        assert!(interpreter_from(Some(text), None).is_err());
        assert!(interpreter_from(None, None).is_err());
    }

    #[test]
    fn an_exe_is_started_directly_and_nothing_is_added_to_it() {
        // The unchanged half. A routed program pays for an interpreter and an
        // environment variable; this one must pay for neither.
        let dir = TempDir::new("launch-exe");
        let exe = dir.write("abeam-direct.exe", b"MZ");
        let launch = resolve(&exe.to_string_lossy(), &args(&["--flag", "a b"])).unwrap();

        assert_eq!(launch.program, exe);
        assert_eq!(launch.target, exe, "an .exe is its own target");
        assert_eq!(launch.args, args(&["--flag", "a b"]));
        assert!(launch.env.is_empty(), "no command line to carry");
    }

    #[test]
    fn a_cmd_is_started_by_naming_cmd_exe_in_front_of_it() {
        let dir = TempDir::new("launch-cmd");
        let script = dir.write("abeam-shim.cmd", b"@echo off\r\n");
        let launch = resolve(&script.to_string_lossy(), &args(&["one", "a&b"])).unwrap();

        assert!(is_image(&launch.program), "cmd.exe is what Windows starts");
        assert_eq!(
            launch
                .program
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase(),
            "cmd.exe"
        );
        assert!(
            launch.program.is_absolute(),
            "never a bare name, not even this one"
        );
        // The border has one job, and "cmd" is a true answer to a question
        // nobody asked.
        assert_eq!(launch.target, script);

        assert_eq!(
            launch.args,
            args(&["/e:ON", "/v:OFF", "/d", "/c", "%ABEAM_LAUNCH%"]),
            "one token on the wire, and nothing portable-pty can quote"
        );
        assert_eq!(
            launch.env,
            vec![(
                "ABEAM_LAUNCH".to_string(),
                format!("\"{}\" one \"a&b\"", script.display())
            )]
        );
    }

    #[test]
    fn the_whole_npm_layout_resolves_to_the_one_file_of_the_three_that_can_run() {
        // The bug `docs/status.md` records, end to end. `npm i -g` writes
        // exactly these three
        // files, and only the middle one is startable: the extensionless shim
        // is a POSIX shell script, and the `.ps1` loses twice over — `.PS1` is
        // absent from the *default* `PATHEXT`, and where somebody has added it
        // `probe` still prefers the file it can start.
        let dir = TempDir::new("launch-npm-trio");
        dir.write("abeam-agent", b"#!/bin/sh\nexec node cli.js\n");
        let shim = dir.write("abeam-agent.cmd", b"@echo off\r\n");
        dir.write("abeam-agent.ps1", b"# nothing\r\n");

        // The way `abeam +abeam-agent` would reach it, with the process's own
        // `PATH` left alone — it is shared with every test running beside this.
        assert_eq!(
            walk(dir.path().as_os_str(), "abeam-agent"),
            Some(shim.clone())
        );

        // ...and the name without an extension still lands on the `.cmd`, which
        // is what an absolute `%APPDATA%\npm\claude` does too.
        let launch = resolve(&dir.path().join("abeam-agent").to_string_lossy(), &[]).unwrap();
        assert_eq!(launch.target, shim);
        assert!(is_image(&launch.program));
    }

    #[test]
    fn a_powershell_script_is_refused_because_which_powershell_is_a_real_question() {
        let dir = TempDir::new("launch-ps1");
        let script = dir.write("abeam-shim.ps1", b"# nothing\r\n");
        let refused = resolve(&script.to_string_lossy(), &[]).expect_err(".ps1 is not launchable");
        assert!(refused.contains("abeam-shim.ps1"), "got: {refused}");
        assert!(
            refused.contains("pwsh"),
            "which one it will not guess between"
        );
        assert!(
            refused.contains(".cmd"),
            "the sibling that would have worked"
        );
    }

    #[test]
    fn an_extensionless_shim_is_refused_with_the_reason_rather_than_a_win32_error() {
        // `CreateProcessW` says "%1 is not a valid Win32 application", which
        // names neither the file nor the problem.
        let dir = TempDir::new("launch-posix");
        let script = dir.write("abeam-shim", b"#!/bin/sh\n");
        let refused =
            resolve(&script.to_string_lossy(), &[]).expect_err("a sh script is not a program");
        assert!(refused.contains("abeam-shim"), "got: {refused}");
        assert!(refused.contains("POSIX"), "got: {refused}");
    }

    #[test]
    fn a_file_that_has_an_extension_is_not_described_as_one_that_has_none() {
        // The same refusal, the other shape. A `.js` reaches it whenever
        // `PATHEXT` lists one or somebody names the file outright, and being
        // told that "a file with no extension is a POSIX shell script" about a
        // file called `abeam-shim.js` reads as abeam having looked somewhere
        // else entirely.
        let dir = TempDir::new("launch-js");
        let script = dir.write("abeam-shim.js", b"// nothing\n");
        let refused = resolve(&script.to_string_lossy(), &[]).expect_err("a .js is not a program");

        assert!(refused.contains("abeam-shim.js"), "got: {refused}");
        assert!(refused.contains("`.js`"), "the extension it has: {refused}");
        assert!(
            !refused.contains("POSIX") && !refused.contains("no extension"),
            "a .js was described as an extensionless shell script: {refused}"
        );
    }

    // --- quoting -----------------------------------------------------------
    //
    // Every expectation below is the exact string `cmd.exe` is handed. They are
    // written out in full rather than assembled, because a helper that built
    // them would be the same code under test.

    #[test]
    fn an_ordinary_argument_is_passed_through_untouched() {
        assert_eq!(line(&[]), r#""C:\npm\claude.cmd""#);
        assert_eq!(line(&["plain"]), r#""C:\npm\claude.cmd" plain"#);
        // Quoting is eager, so `--flag` staying bare is a promise about the
        // safe list rather than an accident.
        assert_eq!(line(&["--resume"]), r#""C:\npm\claude.cmd" --resume"#);
        assert_eq!(
            line(&[r"C:\some\path.txt"]),
            r#""C:\npm\claude.cmd" C:\some\path.txt"#
        );
    }

    #[test]
    fn a_space_makes_one_argument_out_of_two_words() {
        assert_eq!(line(&["a b"]), r#""C:\npm\claude.cmd" "a b""#);
        // An argument that is nothing at all still has to arrive as an
        // argument; without the pair it is dropped on the floor.
        assert_eq!(line(&[""]), r#""C:\npm\claude.cmd" """#);
    }

    #[test]
    fn every_character_cmd_reads_as_syntax_is_shut_inside_quotes() {
        // The reason `docs/status.md` gives for the obvious fix being wrong.
        // Unquoted, each of these
        // ends abeam's command and starts somebody else's.
        assert_eq!(line(&["a&b"]), r#""C:\npm\claude.cmd" "a&b""#);
        assert_eq!(line(&["a|b"]), r#""C:\npm\claude.cmd" "a|b""#);
        assert_eq!(line(&["a^b"]), r#""C:\npm\claude.cmd" "a^b""#);
        assert_eq!(line(&["a>b"]), r#""C:\npm\claude.cmd" "a>b""#);
        assert_eq!(line(&["a<b"]), r#""C:\npm\claude.cmd" "a<b""#);
        assert_eq!(line(&["(a)"]), r#""C:\npm\claude.cmd" "(a)""#);
        assert_eq!(line(&["a;b"]), r#""C:\npm\claude.cmd" "a;b""#);
    }

    #[test]
    fn a_variable_reference_is_quoted_and_left_alone() {
        // Quoted, and *not* escaped: the command line arrives at `cmd` by an
        // expansion, and `cmd` does not rescan what it has just substituted.
        // std escapes `%` because its command line arrives as literal text;
        // doing the same here would send the child `%%cd:~,%VAR%`.
        assert_eq!(line(&["%VAR%"]), r#""C:\npm\claude.cmd" "%VAR%""#);
        assert_eq!(
            line(&["-p", "cd %CD%"]),
            r#""C:\npm\claude.cmd" -p "cd %CD%""#
        );
        // The `!` is quoted here and that is all this string says: nothing in
        // it is what keeps a `!` a `!` rather than the start of a delayed
        // expansion. That is `/v:OFF`, which `through_cmd` puts on the other
        // side of the wire — and the spawning test at the end of this module is
        // where the two are proved to work together.
        assert_eq!(line(&["!DELAYED!"]), r#""C:\npm\claude.cmd" "!DELAYED!""#);
    }

    #[test]
    fn an_embedded_quote_is_doubled_which_is_the_one_spelling_both_parsers_read() {
        // `\"` is what MSVCRT quoting would produce and what `cmd` cannot read;
        // `""` is understood by `cmd` inside a quoted region and by every CRT
        // since Visual C++ 2008, which is what the shim hands the argument to.
        assert_eq!(
            line(&[r#"say "hi""#]),
            r#""C:\npm\claude.cmd" "say ""hi""""#
        );
        assert_eq!(line(&[r#"a"b"#]), r#""C:\npm\claude.cmd" "a""b""#);
    }

    #[test]
    fn a_backslash_run_is_doubled_only_where_a_quote_would_eat_it() {
        // A trailing backslash is quoted for a shim that writes `"%~2"` around
        // the argument, and doubled so it cannot escape its own closing quote.
        assert_eq!(line(&[r"C:\dir\"]), r#""C:\npm\claude.cmd" "C:\dir\\""#);
        assert_eq!(line(&[r"C:\a b\"]), r#""C:\npm\claude.cmd" "C:\a b\\""#);
        // Two backslashes before a quote become four, so the pair survives as a
        // pair and the quote is still escaped by the one that follows.
        assert_eq!(line(&[r#"a\"b"#]), r#""C:\npm\claude.cmd" "a\\""b""#);
        assert_eq!(line(&[r#"a\\"b"#]), r#""C:\npm\claude.cmd" "a\\\\""b""#);
        // ...and a run in the middle, with no quote after it, is left alone.
        assert_eq!(
            line(&[r"C:\a\\b"]),
            r#""C:\npm\claude.cmd" C:\a\\b"#,
            "an unquoted backslash is only an escape next to a quote"
        );
    }

    #[test]
    fn what_cannot_be_escaped_for_cmd_is_refused_rather_than_mangled() {
        // A newline ends the command outright and a carriage return truncates
        // it. Dropping the tail silently is indistinguishable, from outside,
        // from an argument injection that worked.
        for (bad, why) in [
            ("a\0b", "NUL"),
            ("a\rb", "carriage return"),
            ("a\nb", "newline"),
        ] {
            let refused = command_line(Path::new(r"C:\npm\claude.cmd"), &args(&[bad]))
                .expect_err("this cannot be escaped");
            assert!(refused.contains(why), "got: {refused}");
        }
    }

    #[test]
    fn a_line_longer_than_cmd_will_run_is_refused_rather_than_quietly_doing_nothing() {
        // The worst failure this module had, because it had no symptom. Past
        // 8191 characters `cmd` starts nothing, prints nothing and exits 0, so
        // `abeam -p "<10 KB prompt>"` drew an empty pane, printed
        // `exited: ExitStatus { code: 0 }` and left abeam exiting 0 as well.
        // Ten kilobytes is an ordinary prompt, and the cliff is invisible: the
        // same command works against a natively installed `claude.exe` and
        // no-ops against the npm one.
        let prefix = r#""C:\npm\claude.cmd" "#.len();
        let fits = "a".repeat(MAX_LINE - prefix);
        assert_eq!(
            line(&[&fits]).len(),
            MAX_LINE,
            "the longest line cmd.exe was seen to run is still built"
        );

        let over = format!("{fits}a");
        let refused = command_line(Path::new(r"C:\npm\claude.cmd"), &args(&[&over]))
            .expect_err("one character past what cmd.exe will run");
        assert!(
            refused.contains(&MAX_LINE.to_string()),
            "the limit has to be a number somebody can aim under: {refused}"
        );
        assert!(
            refused.contains("cmd.exe's limit and not abeam's"),
            "whose limit it is: {refused}"
        );
        assert!(
            refused.contains("natively"),
            "the install that does not have it: {refused}"
        );

        // And it is the whole line that is measured, not the argument: a long
        // one made of characters that have to be quoted crosses sooner.
        let quoted = "a b".repeat((MAX_LINE - prefix) / 3);
        assert!(
            command_line(Path::new(r"C:\npm\claude.cmd"), &args(&[&quoted])).is_err(),
            "the quotes the line pays for were not counted"
        );
    }

    #[test]
    fn a_path_that_would_close_its_own_quote_is_refused() {
        // Neither is a legal Windows file name, so neither names anything —
        // but both would end the quoted region and turn the rest of the line
        // into syntax, which is worth a sentence rather than a bad spawn.
        for bad in [r#"C:\np"m\claude.cmd"#, r"C:\npm\claude.cmd\"] {
            assert!(
                command_line(Path::new(bad), &[]).is_err(),
                "{bad} should not be quotable"
            );
        }
    }

    // --- and it actually starts -------------------------------------------
    //
    // Everything above this line is a claim about what `cmd.exe` would do with
    // a string. These two are the only place anything asks it.

    /// A `.cmd` that prints its whole argument list back, in a directory with a
    /// space in its name — one of the two hazards `docs/status.md` names, and one
    /// that has to be paid for by every one of these tests rather than by the
    /// one that is about it.
    fn shim(dir: &TempDir) -> PathBuf {
        let home = dir.path().join("with space");
        std::fs::create_dir_all(&home).expect("a directory with a space in it");
        let script = home.join("abeam-shim.cmd");
        std::fs::write(&script, b"@echo off\r\necho ABEAM-SHIM-OK [%*]\r\n").expect("write a shim");
        script
    }

    /// Run one and give back the screen it printed on.
    ///
    /// Shared rather than written out twice: what differs between these tests
    /// is the argument sent and the line that comes back, and a twenty-second
    /// deadline copied is a twenty-second deadline that drifts.
    fn shim_screen(cfg: PtyConfig) -> String {
        use crate::pane::Pane;
        use crate::panes::TerminalPane;
        use std::time::{Duration, Instant};

        let mut pane = TerminalPane::spawn_with(cfg).expect("spawn the shim through cmd.exe");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            pane.tick();
            let screen = pane.last_screen().join("\n");
            if screen.contains("ABEAM-SHIM-OK") {
                return screen;
            }
            assert!(
                Instant::now() < deadline,
                "the shim never printed anything:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_cmd_shim_really_runs_under_conpty_with_its_arguments_intact() {
        // The quoting tests above compare strings. This one is the proof that
        // the string is the one `cmd.exe` wanted: a real shim, in a real pty,
        // printing back what it was given.
        //
        // One argument carries an `&`, which is why "go through cmd.exe /c" is
        // called the wrong fix in `docs/status.md`. Unquoted it would end abeam's
        // command and run `b]` as another one, which is exactly what is on
        // screen when this regresses.
        let dir = TempDir::new("launch-spawn");
        let script = shim(&dir);
        let home = script.parent().expect("the shim has a directory");

        // Found the way a `PATH` lookup would find it, so the resolution under
        // test is the one an `abeam +claude` performs...
        assert_eq!(
            walk(home.as_os_str(), "abeam-shim"),
            Some(script.clone()),
            "the shim is reachable by bare name"
        );
        // ...and then resolved by path, because putting a directory on this
        // process's `PATH` would put it on the `PATH` of every other test
        // running beside this one.
        let launch = resolve(&script.to_string_lossy(), &args(&["plain", "a&b"])).unwrap();

        // Wide enough that nothing on the line wraps: the assertion is about
        // the text, and a rejoin would hide a space that moved.
        let screen = shim_screen(launch.config().cwd(dir.path()).size(10, 200));
        assert!(
            screen.contains(r#"ABEAM-SHIM-OK [plain "a&b"]"#),
            "the shim was started but its arguments did not survive:\n{screen}"
        );
    }

    #[test]
    fn a_percent_and_a_bang_reach_the_child_as_themselves_with_the_variables_set() {
        // The module docs spend most of their words on these two — why std's
        // `%` escaping is deliberately absent, and why `/v:OFF` is on the
        // command line — and until this existed nothing regression-tested
        // either. A string assertion cannot: what is claimed is a fact about
        // how `cmd` expands, so `cmd` has to be the one asked.
        //
        // Both variables are *set on the child*, which is what makes a pass
        // mean anything. An expansion anywhere along the way would replace the
        // argument with `ABEAM-EXPANDED-...` and this would fail, where against
        // an unset variable `%ABEAM_PCT%` survives by accident and `!…!`
        // vanishes silently.
        let dir = TempDir::new("launch-spawn-expansion");
        let script = shim(&dir);
        let launch = resolve(
            &script.to_string_lossy(),
            &args(&["%ABEAM_PCT%", "!ABEAM_BANG!"]),
        )
        .unwrap();

        let screen = shim_screen(
            launch
                .config()
                .env("ABEAM_PCT", "ABEAM-EXPANDED-PERCENT")
                .env("ABEAM_BANG", "ABEAM-EXPANDED-BANG")
                .cwd(dir.path())
                .size(10, 200),
        );

        assert!(
            screen.contains(r#"ABEAM-SHIM-OK ["%ABEAM_PCT%" "!ABEAM_BANG!"]"#),
            "an argument was expanded on the way to the child:\n{screen}"
        );
        // Said twice on purpose. The line above could be satisfied by a screen
        // that also contained an expanded copy; this one cannot.
        assert!(
            !screen.contains("ABEAM-EXPANDED"),
            "a variable was expanded somewhere on this screen:\n{screen}"
        );
    }
}
