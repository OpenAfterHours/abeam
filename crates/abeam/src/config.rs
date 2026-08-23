//! The one file abeam reads on its own account: the names a `+` can take, and
//! the state a session opens in.
//!
//! abeam's configuration has been two environment variables for its whole life,
//! and `docs/design.md` lists what that costs in the same breath as it lists
//! what it buys. This file is the smallest thing that pays the two debts worth paying.
//! `[preset.<name>]` puts a command line behind a word — `abeam +fleet` instead
//! of `abeam +claude agent` with a view to switch to afterwards — and
//! `[defaults]` is somewhere for the reader's light/dark choice to live, which
//! until now started dark every session because there was nowhere to write it
//! down.
//!
//! ## Where it lives, and why that is a security decision
//!
//! **The user's profile, and never the repository.** On Windows
//! `%APPDATA%\abeam\abeam.toml`; on Unix `$XDG_CONFIG_HOME/abeam/abeam.toml`,
//! or `$HOME/.config/abeam/abeam.toml` when that variable is unset.
//!
//! There is no repository-local config, that is a decision rather than a gap,
//! and it is the same decision `crate::launch` exists to enforce one level
//! down. The repository on screen is the one directory in this whole program
//! that somebody else gets to write to — it is a clone, it is somebody's pull
//! request, it is whatever `git checkout` just put there. `launch::resolve`
//! spends four hundred lines making sure a `claude.exe` sitting in it can never
//! be what starts, and `main` walks the process out of it entirely for the rest
//! of the session. A `.abeam.toml` read out of that directory would hand every
//! one of those files a `[preset.claude] host = "./tools/claude"` and undo all
//! of it in six lines of TOML — with abeam's own border obligingly printing the
//! word `claude` over the top.
//!
//! The usual answer to that is a trust prompt: read the file, notice it is new,
//! ask. abeam does not have one and this file is not the place to invent one.
//! The prompt would arrive before `term::setup`, on a plain console, at the one
//! moment the user is trying to start an agent — and a dialog that appears
//! whenever a repository is fresh is a dialog that gets answered yes. So the
//! rule is the one that needs no prompt: this file comes from the profile, a
//! repository cannot contribute to it, and a preset is something the person at
//! the keyboard wrote about their own machine.
//!
//! ## No file is not an error; a file abeam cannot read is
//!
//! Nothing here is required and most machines will have none of it, so a
//! missing file is silence and a [`Config::default`]. A file that *is* there
//! and does not parse is fatal: it is refused by name, with the parser's own
//! line and column, and `main` exits 2 before `term::setup` — the same place
//! and for the same reason as `crate::agent`'s refusals, which is that a
//! message printed after raw mode is on lands on a screen that is about to be
//! thrown away.
//!
//! Fatal rather than ignored, because of what this file *is*. It names programs
//! to start. Reading half of it, or skipping a section that did not parse,
//! means abeam starts something other than what the file asks for while the
//! user believes it is doing what they wrote — and a preset that silently is
//! not there is a `+fleet` that becomes a `PATH` lookup for a program called
//! `fleet`, which fails with a message naming neither abeam's config file nor
//! their mistake. Refusing costs one edit; ignoring costs somebody an hour.
//!
//! The same answer covers a file that exists and cannot be *read* — a
//! permission bit, a directory where a file should be. Only
//! [`std::io::ErrorKind::NotFound`] is silence, because only "there is no such
//! file" is the ordinary state this feature is optional in.
//!
//! ## The two rules a preset is held to
//!
//! Both are checked here, at load time, which is the only moment before
//! somebody is waiting for a program to start.
//!
//! **A `host` resolves against the built-in table and `PATH`, never against
//! another preset.** Structurally, not by a check: [`row`] asks
//! `crate::agent::find`, which reads [`AGENTS`] alone, and the preset rows do
//! not exist yet when it does. So a `[preset.claude] host = "claude"` cannot
//! recurse, because there is no edge from a preset to a preset for it to
//! recurse along. There is no preset chaining, and the thing it would buy —
//! naming one preset from another — is one saved line in a config file, bought
//! with a cycle check on the path that decides which program starts.
//!
//! **A preset may not take a name abeam already answers.** A built-in's name,
//! or one of the two words behind the sigil. Either would be a name with two
//! meanings, one of them unreachable, with nothing anywhere on screen saying
//! which of the two ran — so it is refused, and the message names the conflict.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agent::{AGENTS, Agent, RESERVED};
use crate::pane::Focus;
use crate::panes::RightView;

/// The file's name, wherever this platform decides it lives.
const FILE: &str = "abeam.toml";

/// The directory it sits in under the profile root. abeam's own, because these
/// roots are shared with every other program on the machine.
const DIR: &str = "abeam";

// ---------------------------------------------------------------------------
// where the file is
// ---------------------------------------------------------------------------

/// The config file's path, or `None` when this machine will not say where the
/// user's profile is.
///
/// `%APPDATA%` and nothing behind it. It is the variable Windows sets for
/// exactly this — per-user application data that roams with the profile — and
/// the fallbacks that suggest themselves are all worse than having none:
/// `USERPROFILE` would put a bare `abeam` directory in somebody's home, and
/// `HOME` on Windows is git-bash's, which frequently names a POSIX-shaped path
/// no Windows program has ever written to. `crate::agentstate::home` records
/// that last hazard at length; the difference here is that abeam is choosing
/// where to *look for its own file* rather than trying to find somebody else's,
/// so one right answer with no second guess is the whole requirement.
#[cfg(windows)]
pub fn path() -> Option<PathBuf> {
    from_appdata(std::env::var_os("APPDATA").map(PathBuf::from))
}

/// The Unix twin, over the two variables the XDG base directory specification
/// names in the order it names them.
#[cfg(unix)]
pub fn path() -> Option<PathBuf> {
    from_xdg(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// Windows' answer, over the variable handed in rather than read.
///
/// Split out for `crate::agentstate::sessions_path_from`'s reason, which is
/// that the process environment belongs to the whole test binary: a test that
/// set `APPDATA` to prove this rule would be setting it for the three hundred
/// and fifty tests running beside it, several of which spawn children that
/// inherit it.
///
/// **A relative variable is refused rather than followed**, and that is the
/// same rule, for the same reason, as `sessions_path_from`'s and
/// `launch::find`'s. Joining onto a relative path leaves a relative path, so
/// the `read_to_string` below stops being a question about the user's profile
/// and becomes one about wherever this process happens to be standing — which
/// `main` deliberately moves to `%SystemRoot%` or `/`, and which before that
/// line is the repository on screen. An `APPDATA=.` typed into a shell for some
/// other program's benefit would then make `./abeam/abeam.toml` inside a cloned
/// repository into abeam's config file, which is precisely the file the module
/// docs above spend four paragraphs refusing to read. Absoluteness rather than
/// mere blankness, because blank is only the loudest way of being relative —
/// and PowerShell leaves `$env:APPDATA = ""` behind when somebody clears it.
///
/// Compiled on both platforms and gated only at its caller, which is one step
/// further than `agentstate` goes and is worth the `dead_code` waiver it costs.
/// This program is developed on Windows and runs on Linux, so the Unix rule
/// below is the one most likely to be broken by somebody who cannot run it —
/// and both of these are string arithmetic with no filesystem in them, so there
/// is nothing about either that a machine of the other kind cannot prove.
#[cfg_attr(
    unix,
    allow(dead_code, reason = "the other platform's rule, tested on both")
)]
fn from_appdata(appdata: Option<PathBuf>) -> Option<PathBuf> {
    Some(
        appdata
            .filter(|dir| dir.is_absolute())?
            .join(DIR)
            .join(FILE),
    )
}

/// Unix's answer, over the two variables handed in rather than read.
///
/// `XDG_CONFIG_HOME` when it is set to something absolute, and `~/.config`
/// otherwise, which is what the specification says the fallback is rather than
/// abeam's own invention. Both are held to the absoluteness rule above, and the
/// second one is the reason it is applied twice rather than once: a container or
/// a service unit can export an empty `HOME`, and `.config/abeam/abeam.toml`
/// resolved against `/` is a file belonging to nobody that root can write.
///
/// A **relative** `XDG_CONFIG_HOME` falls through to `HOME` rather than ending
/// the search, which is the one place this differs from simply refusing and is
/// worth being explicit about. The variable is discarded either way — nothing
/// relative is ever joined onto, which is the whole of the hazard — so the only
/// question left is whether the mistake costs the user their config file, and
/// two answers agree that it should not: the XDG specification says in as many
/// words that an implementation meeting a relative path in one of these
/// variables should consider it invalid and ignore it, and
/// `crate::agentstate::sessions_path_from` already falls through from a
/// relative `CLAUDE_CONFIG_DIR` to the home directory for the same reason. A
/// bad variable set for some other program's benefit should not empty abeam's
/// profile.
#[cfg_attr(
    windows,
    allow(dead_code, reason = "the other platform's rule, tested on both")
)]
fn from_xdg(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = match xdg.filter(|dir| dir.is_absolute()) {
        Some(xdg) => xdg,
        None => home.filter(|dir| dir.is_absolute())?.join(".config"),
    };
    Some(base.join(DIR).join(FILE))
}

// ---------------------------------------------------------------------------
// reading it
// ---------------------------------------------------------------------------

/// Whatever the user's profile has to say, or nothing at all.
///
/// `Err` is a message for standard error and an exit code 2 — see the module
/// docs for why a file that will not parse is fatal and a file that is not
/// there is not.
pub fn load() -> Result<Config, String> {
    match path() {
        Some(path) => at(&path),
        None => Ok(Config::default()),
    }
}

/// [`load`], over a path handed in rather than derived.
///
/// The seam every test that wants a real file goes through, so that none of
/// them has to have an opinion about where this machine keeps its profile.
fn at(path: &Path) -> Result<Config, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // The one silence. Every other failure is a file that is there and that
        // abeam cannot read, which is not the same thing as not having one.
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(why) => return Err(refused(path, &unparsed(&why.to_string()))),
    };

    let mut config = read(&text).map_err(|why| refused(path, &why))?;
    // Learned here rather than parsed, because the messages that name the file
    // are built later — a preset whose host is not an agent abeam knows has
    // nothing to say except "look at the thing you wrote, here".
    config.path = Some(path.to_path_buf());
    Ok(config)
}

/// The whole of the parsing and all of the refusals, over a string.
///
/// Pure, and the tests drive it with TOML written inline rather than with
/// files: what is being asserted is what a *document* means, and a temporary
/// directory between the assertion and the thing asserted is a second thing
/// that can go wrong.
fn read(text: &str) -> Result<Config, String> {
    let file: File = toml::from_str(text).map_err(|why| unparsed(&why.to_string()))?;

    let mut presets: Vec<Preset> = Vec::new();
    for (name, wire) in file.preset {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(nameless());
        }
        if crate::agent::find(&name).is_some() {
            return Err(shadowed(&name));
        }
        if RESERVED.iter().any(|word| word.eq_ignore_ascii_case(&name)) {
            return Err(sigil(&name));
        }
        // Two spellings of one name. TOML keeps them apart and abeam does not —
        // every name behind a `+` is matched without regard to case — so the
        // second would be a preset that can never be selected and never says
        // so. The same objection as a name that shadows a built-in, arriving
        // between two rows the user wrote themselves.
        if let Some(first) = presets
            .iter()
            .find(|seen| seen.name.eq_ignore_ascii_case(&name))
        {
            return Err(twice(&first.name, &name));
        }
        if wire.host.trim().is_empty() {
            return Err(hostless(&name));
        }

        presets.push(Preset {
            host: wire.host.trim().to_string(),
            args: wire.args,
            open: Wanted {
                view: wire.view,
                focus: wire.focus,
                zoom: wire.zoom,
                theme: wire.theme,
            },
            name,
        });
    }

    Ok(Config {
        path: None,
        defaults: file.defaults,
        presets,
    })
}

/// What the user's profile said, once.
#[derive(Debug, Default)]
pub struct Config {
    /// Where it was read from, for the one message that has to name it.
    /// `None` for a config that never came from a file, which is every config
    /// in this file's own tests and the empty one a machine without the file
    /// gets.
    path: Option<PathBuf>,
    defaults: Wanted,
    presets: Vec<Preset>,
}

impl Config {
    /// Every name a `+` may take: abeam's own agents, then the user's presets.
    ///
    /// **Called once, at startup.** The table is `&'static` because it outlives
    /// every question anybody asks of it — `crate::agent::Agent` says why that
    /// is a claim about lifetime rather than about literals — and the strings
    /// in it are leaked to say so. Calling this twice would leak twice, which
    /// is a bounded and uninteresting amount of memory and still not something
    /// to do in a loop.
    ///
    /// A machine with no presets gets [`AGENTS`] itself and allocates nothing,
    /// which is the ordinary case and worth keeping free.
    pub fn table(&self) -> &'static [Agent] {
        if self.presets.is_empty() {
            return AGENTS;
        }
        let mut table = AGENTS.to_vec();
        table.extend(self.presets.iter().map(|preset| self.row(preset)));
        Box::leak(table.into_boxed_slice())
    }

    /// One preset as a row of the table.
    ///
    /// The whole of the "no chaining" rule lives in the first line: the lookup
    /// is `crate::agent::find`, which reads the built-in table and nothing
    /// else, and it runs before any preset row exists. A preset that names a
    /// built-in *becomes* that built-in's candidates and inherits its install
    /// sentence, because the thing that would be missing really is Claude or
    /// Copilot. A preset that names anything else is one candidate — the word
    /// as written — for `crate::launch` to look up on `PATH`, exactly as
    /// `abeam +pwsh` would.
    fn row(&self, preset: &Preset) -> Agent {
        let (candidates, install, hosts) = match crate::agent::find(&preset.host) {
            Some(built) => (built.candidates, built.install, built.hosts),
            None => (
                every(std::slice::from_ref(&preset.host)),
                forever(elsewhere(&preset.name, &preset.host, &self.whence())),
                forever(preset.host.clone()),
            ),
        };

        Agent {
            name: forever(preset.name.clone()),
            candidates,
            install,
            args: every(&preset.args),
            hosts,
        }
    }

    /// The state a session opens in: `[defaults]`, with the chosen preset's own
    /// fields over the top of it.
    ///
    /// `chosen` is the name the command line settled on, and it is looked up
    /// rather than handed in as a preset because the parser deals in
    /// `crate::agent::Agent` — a type that deliberately knows nothing about
    /// views or themes. A built-in's name finds nothing here and gets the
    /// defaults, which is right: `abeam +claude` is a request to host Claude
    /// and says nothing about which pane should be showing.
    pub fn opening(&self, chosen: Option<&str>) -> Opening {
        let wanted = match chosen.and_then(|name| self.preset(name)) {
            Some(preset) => self.defaults.overlaid(preset.open),
            None => self.defaults,
        };
        wanted.opening()
    }

    fn preset(&self, name: &str) -> Option<&Preset> {
        self.presets
            .iter()
            .find(|preset| preset.name.eq_ignore_ascii_case(name))
    }

    /// The file, for a message that has to point at it. A config with no path
    /// behind it is one nobody can be sent to look at, so it is named by the
    /// only thing that is still true of it.
    fn whence(&self) -> String {
        match &self.path {
            Some(path) => path.display().to_string(),
            None => FILE.to_string(),
        }
    }
}

/// One `[preset.<name>]`, after the refusals.
#[derive(Debug)]
struct Preset {
    name: String,
    host: String,
    args: Vec<String>,
    open: Wanted,
}

// ---------------------------------------------------------------------------
// the opening state
// ---------------------------------------------------------------------------

/// What `App::new` is told to open with.
///
/// Four fields that were four literals inside `App::new` until there was
/// somewhere to write an answer down. They are one struct rather than four
/// arguments because they are one idea — *how this session starts* — and
/// because four `bool`s and two enums in a row is a call nobody can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    pub view: RightView,
    pub focus: Focus,
    pub zoom: bool,
    pub theme: Theme,
}

/// abeam's behaviour before anybody configured anything, which is what every
/// session did until this module existed and what every session without a
/// config file still does.
impl Default for Opening {
    fn default() -> Self {
        Self {
            view: RightView::Git,
            focus: Focus::Left,
            zoom: false,
            theme: Theme::Dark,
        }
    }
}

/// The reader's page, in the vocabulary of the file rather than of the pane.
///
/// `crate::panes::viewer` has its own two-valued type for this and it is
/// private to that module — deliberately, since it carries the palettes as
/// well as the choice. This enum is what crosses the boundary: it is what the
/// user typed, it is what `ViewerPane::set_theme` takes, and keeping the
/// palettes on the far side of that door is the reason the viewer does not
/// have to know that a config file exists.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// The reader's own default, and the one it has always started on.
    #[default]
    Dark,
    Light,
}

// ---------------------------------------------------------------------------
// the file, as it is on disk
// ---------------------------------------------------------------------------

/// The document, in TOML's shape rather than abeam's.
///
/// `deny_unknown_fields`, everywhere, which is the opposite of what
/// `crate::agentstate` does with Claude's records and is the same reasoning
/// pointed the other way. That format belongs to somebody else and grows
/// fields abeam has never heard of; this one is abeam's own and is *typed by
/// hand*. A `[presets.fleet]` with the plural spelling, or a `them = "dark"`,
/// is a line somebody wrote and expected to work — and quietly dropping it
/// produces a session that behaves as though the file were not there, with no
/// way at all to tell that from a file abeam never found.
///
/// What it costs is forward compatibility: a config written for a later abeam
/// is refused by an earlier one rather than partly honoured. That is the right
/// way round for a file that names programs to start, and the message says
/// which key it could not read.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    defaults: Wanted,
    /// `[preset.<name>]`, singular, because that is how the table header reads
    /// at the point somebody types it.
    ///
    /// A `BTreeMap` rather than the file's own order: TOML refuses a duplicate
    /// key itself, so what this buys is a deterministic order for `+help` and
    /// for the "`x` is installed" hints, which are the two places the table's
    /// order is visible.
    #[serde(default)]
    preset: BTreeMap<String, Wire>,
}

/// The four opening fields, each of them absent until somebody says otherwise.
///
/// One type for `[defaults]` and for the same four keys inside a preset, which
/// is what makes "a preset overrides the defaults field by field" a three-line
/// function rather than a rule to remember. Absence is what carries the
/// override: a preset that sets `view` and nothing else changes the view and
/// leaves the theme where `[defaults]` put it.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wanted {
    view: Option<View>,
    focus: Option<Side>,
    zoom: Option<bool>,
    theme: Option<Theme>,
}

impl Wanted {
    /// `over`'s answers, and this one's wherever `over` had none.
    fn overlaid(self, over: Wanted) -> Wanted {
        Wanted {
            view: over.view.or(self.view),
            focus: over.focus.or(self.focus),
            zoom: over.zoom.or(self.zoom),
            theme: over.theme.or(self.theme),
        }
    }

    /// ...and abeam's own answers wherever neither had one.
    fn opening(self) -> Opening {
        let fallback = Opening::default();
        Opening {
            view: self.view.map_or(fallback.view, View::view),
            focus: self.focus.map_or(fallback.focus, Side::focus),
            zoom: self.zoom.unwrap_or(fallback.zoom),
            theme: self.theme.unwrap_or(fallback.theme),
        }
    }
}

/// One `[preset.<name>]` as it is written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    host: String,
    #[serde(default)]
    args: Vec<String>,
    view: Option<View>,
    focus: Option<Side>,
    zoom: Option<bool>,
    theme: Option<Theme>,
}

/// Which right-hand view opens.
///
/// Five words, and `crate::panes::RightView` has six variants: `Diag` is the
/// pty instrument behind `F2`, which is somewhere you go to answer a question
/// and then come back from. A session that opened there would be a session
/// whose config file had accidentally been left in a debugging state, so it is
/// not in this vocabulary at all — the enum is the whole of the answer and
/// there is nothing to check afterwards.
///
/// `ask` is in, and it is the variant that had to be argued rather than
/// assumed, because it is displaceable in exactly the way `Diag` is: it is
/// reached by `?` from another view and `Esc` puts that view back. What settles
/// it is that opening *there* is not a debugging state — it is a session that
/// starts by asking a question, which is a thing somebody may reasonably want
/// every morning — and the view `Esc` returns to is simply whichever one was
/// showing when the ask displaced it, which on the first frame is abeam's own
/// default. Nothing about the config file is left in an odd state by choosing
/// it, which is the whole of the test `Diag` fails.
///
/// That last claim is upheld in `crate::app::App::new` and nowhere else, and it
/// is worth naming the line because the obvious spelling of it is wrong:
/// remembering the *opening* view as the one to put back makes `Esc` out of an
/// ask that was opened from a config file call `set_right_view(Ask)`, which is
/// the key that could never leave. `App::set_right_view` cannot cover it,
/// because the first view of all never goes through a switch.
///
/// `files` rather than `viewer`, because that is what the pane is called
/// everywhere a user meets it: the README's tour, the `F1` key list, the border.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum View {
    Git,
    Files,
    Shell,
    Queue,
    Ask,
}

impl View {
    fn view(self) -> RightView {
        match self {
            View::Git => RightView::Git,
            View::Files => RightView::Viewer,
            View::Shell => RightView::Shell,
            View::Queue => RightView::Queue,
            View::Ask => RightView::Ask,
        }
    }
}

/// Which pane has the keyboard.
///
/// `left` and `right` rather than `agent` and `view`, because that is what the
/// keys are called — `F4` and `F5` move between two halves of a window — and
/// because which pane is on the right is the thing this file's other key
/// chooses.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Side {
    Left,
    Right,
}

impl Side {
    fn focus(self) -> Focus {
        match self {
            Side::Left => Focus::Left,
            Side::Right => Focus::Right,
        }
    }
}

// ---------------------------------------------------------------------------
// what a table costs
// ---------------------------------------------------------------------------

/// A string that lives as long as the process, because the table does.
///
/// The alternative is a lifetime parameter on `crate::agent::Agent` and
/// therefore on `Cli`, `Choice`, both parse functions and every test helper
/// that builds a table — to describe memory that is allocated once, at startup,
/// out of a file that has already been read, and freed by nothing short of
/// `exit`. It was written that way first and the borrow parameter reached six
/// signatures before it was clear that none of them wanted it.
///
/// Bounded by the config file: a preset contributes its name, its host and its
/// arguments, once, before the first frame. Nothing here runs in a loop.
fn forever(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// The same for a list of them, which is what `candidates` and `args` are.
fn every(list: &[String]) -> &'static [&'static str] {
    Box::leak(
        list.iter()
            .cloned()
            .map(forever)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

// ---------------------------------------------------------------------------
// the refusals
// ---------------------------------------------------------------------------

/// What abeam says about a config file it will not use, whatever the reason.
///
/// The path first, because the reader has to open something and abeam's idea of
/// where that file lives is exactly the thing they may be wrong about — a
/// `%APPDATA%` they have never looked at, an `XDG_CONFIG_HOME` set by a dotfile
/// three years ago. Then the reason, whole and unedited.
///
/// One preamble and no more, because the two kinds of reason go to two
/// different readers and neither is improved by a sentence written for the
/// other. A parse failure is a typo in TOML and arrives with `toml`'s own line
/// and column, which is better than anything this file could write about
/// somebody else's punctuation; a refusal below is a line that parsed perfectly
/// and asks for something abeam cannot honour, and it names what to write
/// instead.
fn refused(path: &Path, why: &str) -> String {
    format!(
        "abeam refused its config file:\n  {}\n\n{why}",
        path.display()
    )
}

/// A file abeam could get no document out of at all: a syntax error, or an
/// operating system that would not hand over the bytes.
///
/// The one refusal that has to argue for itself, and the reason is that it is
/// the one a reader could reasonably have expected abeam to shrug off. Every
/// other refusal here names a line somebody wrote and says what to put there
/// instead, which is its own argument.
fn unparsed(why: &str) -> String {
    format!(
        "{why}\n\n\
         A config file that is there and cannot be read is refused rather than \
         ignored: it names the programs `+` can start, so carrying on would \
         mean starting something other than what it asks for. Fix it or move \
         it aside — having no config file at all is an ordinary state."
    )
}

/// A preset that has taken one of abeam's own agents' names.
fn shadowed(name: &str) -> String {
    format!(
        "`[preset.{name}]` takes the name of an agent abeam already knows, so \
         `+{name}` would mean two things and one of them would be unreachable \
         with nothing on screen saying why. Rename the preset — \
         `[preset.my-{name}]` — and say `host = \"{name}\"` inside it, which \
         is how a preset names the agent it starts."
    )
}

/// ...or one of the two words the sigil answers itself.
fn sigil(name: &str) -> String {
    format!(
        "`[preset.{name}]` takes one of abeam's own two words, so `+{name}` \
         would never reach it: `+help` and `+version` are answered before the \
         table is consulted at all. Rename the preset."
    )
}

/// Two presets abeam cannot tell apart.
fn twice(first: &str, second: &str) -> String {
    format!(
        "`[preset.{first}]` and `[preset.{second}]` are two presets abeam \
         cannot tell apart: a name behind a `+` is matched without regard to \
         case, exactly as `abeam +Claude` and `abeam +claude` are one request. \
         Rename one of them."
    )
}

/// A `[preset.""]`, or one whose name is a space.
fn nameless() -> String {
    "a preset with a blank name is one nothing can select: a `+` with nothing \
     behind it is refused, so there would be no way to type it. Give it a name \
     or delete it."
        .to_string()
}

/// A preset that names nothing to start.
fn hostless(name: &str) -> String {
    let known: Vec<String> = AGENTS.iter().map(|a| format!("`{}`", a.name)).collect();
    format!(
        "`[preset.{name}]` has a blank `host`, which names nothing to start. \
         `host` is one of the agents abeam knows — {} — or any program on \
         PATH, spelled as you would type it after a `+`.",
        known.join(" or ")
    )
}

/// The install sentence for a preset whose host is not an agent abeam knows.
///
/// The one entry in the table with nothing to say about installing itself, so
/// it says the only useful thing left: which file this came out of, and what it
/// asked for. `crate::agent::missing` prints this to somebody who has just been
/// told a program could not be found, and "here is the file where you named it"
/// is the sentence that ends that search — the mistake is very often a typo
/// three characters long.
fn elsewhere(name: &str, host: &str, file: &str) -> String {
    format!(
        "`{name}` is a preset in {file}, and its `host` is `{host}` — a \
         program abeam looks up on PATH rather than an agent it knows how to \
         install. Check the spelling there, or install `{host}` however that \
         program is installed."
    )
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Nothing here reads or writes the process environment, and almost nothing
/// here touches the filesystem. Both rules come from the same place: this suite
/// is one binary with three hundred and fifty other tests in it, several of
/// which spawn children that inherit whatever it has been doing to `APPDATA`.
///
/// So the path decision is tested through [`from_appdata`] and [`from_xdg`],
/// which take their variables as arguments, and the document is tested through
/// [`read`], which takes a string. The two tests that do open a file are about
/// the two answers that are *about* a file — one that is not there, and one
/// that is there and will not parse — and they use a temporary directory of
/// their own.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Choice, Cli, parse_with, resolve_within};
    use crate::testutil::TempDir;

    /// A config file with one preset in it, in the spelling the README gives.
    const FLEET: &str = r#"
        [defaults]
        view  = "git"
        theme = "light"

        [preset.fleet]
        host  = "claude"
        args  = ["agent"]
        view  = "queue"
        focus = "left"
        zoom  = false
        theme = "dark"
    "#;

    fn config(text: &str) -> Config {
        read(text).expect("a config file")
    }

    /// The names in a table, in the order a reader would meet them.
    fn names(table: &[Agent]) -> Vec<&str> {
        table.iter().map(|agent| agent.name).collect()
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    // --- where the file is ------------------------------------------------

    /// Two directories this platform calls absolute.
    ///
    /// Spelled for the machine running the test rather than for the rule under
    /// test, because [`Path::is_absolute`] is the platform's own question and
    /// not a portable one: `/home/philm` has no drive on it, so Windows does
    /// not call it absolute, and a Unix rule tested on Windows with a Unix path
    /// would be a test of the wrong half. What each rule *does* with a variable
    /// it accepts is the same arithmetic everywhere, and that is what these
    /// prove. `crate::agentstate::spelling` records the same split for the same
    /// reason: a path is text until it is asked about a filesystem.
    #[cfg(windows)]
    const ABS: &str = r"C:\Users\philm\AppData\Roaming";
    #[cfg(windows)]
    const ELSEWHERE: &str = r"D:\config";
    #[cfg(unix)]
    const ABS: &str = "/home/philm";
    #[cfg(unix)]
    const ELSEWHERE: &str = "/etc/xdg-for-me";

    /// What a rule put on the end of the directory it was given, as words —
    /// so that neither the separator nor the drive letter is in the assertion.
    fn under(path: Option<PathBuf>, base: &str) -> Vec<String> {
        path.expect("an absolute variable is followed")
            .strip_prefix(base)
            .expect("the answer is under the directory it was given")
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_file_is_looked_for_under_the_profile_on_both_platforms() {
        // Windows: one variable, and abeam's own directory inside it. Nothing
        // is behind `%APPDATA%` — see [`path`] for why the obvious fallbacks
        // are each worse than having none.
        assert_eq!(
            under(from_appdata(Some(PathBuf::from(ABS))), ABS),
            ["abeam", FILE]
        );

        // Unix: the XDG variable when it is set, used as the config home it
        // says it is...
        assert_eq!(
            under(from_xdg(Some(PathBuf::from(ABS)), None), ABS),
            ["abeam", FILE]
        );
        // ...and `~/.config` when it is not, which is the specification's own
        // fallback rather than abeam's invention. This is the one component
        // that differs between the two rules, so it is the one worth pinning.
        assert_eq!(
            under(from_xdg(None, Some(PathBuf::from(ABS))), ABS),
            [".config", "abeam", FILE]
        );
        // The variable wins over the home directory when both are there, which
        // is what "config home" means.
        assert_eq!(
            under(
                from_xdg(Some(PathBuf::from(ELSEWHERE)), Some(PathBuf::from(ABS))),
                ELSEWHERE
            ),
            ["abeam", FILE]
        );

        // Nothing said, nothing found — which is silence and not an error.
        assert_eq!(from_appdata(None), None);
        assert_eq!(from_xdg(None, None), None);
    }

    #[test]
    fn a_relative_variable_is_refused_rather_than_resolved_against_wherever_we_stand() {
        // The rule this shares with `agentstate::sessions_path_from` and
        // `launch::find`. Joining onto a relative path leaves a relative path,
        // so the read below stops being a question about the user's profile and
        // becomes one about the process's directory — which for most of abeam's
        // life is `%SystemRoot%` or `/`, and before that line is the repository
        // on screen. `APPDATA=.` would make a cloned repository's own
        // `abeam/abeam.toml` into abeam's config file, and that file names
        // programs to start.
        for relative in [".", "abeam", "..", "", "  "] {
            assert_eq!(
                from_appdata(Some(PathBuf::from(relative))),
                None,
                "`{relative}` is not somewhere abeam may read a config file from"
            );
            assert_eq!(from_xdg(Some(PathBuf::from(relative)), None), None);
            // And on the home side too, which is where an empty variable
            // actually turns up: a container or a service unit can export
            // `HOME=`.
            assert_eq!(from_xdg(None, Some(PathBuf::from(relative))), None);
        }

        // ...and a relative `XDG_CONFIG_HOME` is *ignored* rather than fatal:
        // it is discarded, which is the whole of the hazard, and then the home
        // directory answers as though the variable had never been set. Both the
        // XDG specification and `agentstate::sessions_path_from` say the same
        // thing, and the alternative would cost somebody their presets over a
        // variable set for another program.
        assert_eq!(
            under(
                from_xdg(Some(PathBuf::from(".")), Some(PathBuf::from(ABS))),
                ABS
            ),
            [".config", "abeam", FILE],
            "a relative XDG_CONFIG_HOME is ignored, not followed and not fatal"
        );
    }

    // --- no file, and a file that will not parse ---------------------------

    #[test]
    fn no_file_is_no_config_and_not_a_failure() {
        let dir = TempDir::new("config-absent");
        let missing = dir.path().join("nothing-here.toml");

        let config = at(&missing).expect("a machine with no config file is an ordinary machine");
        assert!(config.presets.is_empty());
        // ...and it opens exactly where abeam has always opened.
        assert_eq!(config.opening(None), Opening::default());
        // The table is the built-in one, untouched and unallocated.
        assert_eq!(names(config.table()), names(AGENTS));
    }

    #[test]
    fn a_file_that_is_there_and_does_not_parse_is_refused_by_name() {
        let dir = TempDir::new("config-broken");
        dir.write("abeam.toml", b"[preset.fleet\nhost = \"claude\"\n");
        let path = dir.path().join("abeam.toml");

        let refused = at(&path).expect_err("a broken config file is not an empty one");

        // The path, because abeam's idea of where this file lives is the thing
        // the reader is most likely not to know.
        assert!(
            refused.contains(&path.display().to_string()),
            "got: {refused}"
        );
        // The parser's own complaint, line and column included — better than
        // anything abeam could write about somebody else's typo.
        assert!(refused.contains("line 1"), "got: {refused}");
        // And why it is fatal rather than ignored.
        assert!(refused.contains("names the programs"), "got: {refused}");

        // An empty file is a perfectly good config file, which is the other
        // half of the same rule: what is refused is a document abeam cannot
        // read, not a document that says nothing.
        dir.write("empty.toml", b"\n\n# nothing here yet\n");
        assert!(at(&dir.path().join("empty.toml")).is_ok());
    }

    #[test]
    fn a_key_abeam_does_not_know_is_a_typo_and_not_a_future_version() {
        // `deny_unknown_fields`, and the argument for it: every one of these is
        // a line somebody wrote and expected to work, and a config file that
        // silently ignored them would behave exactly like a config file abeam
        // never found.
        for typo in [
            "[presets.fleet]\nhost = \"claude\"\n",
            "[defaults]\nthem = \"dark\"\n",
            "[preset.fleet]\nhost = \"claude\"\nthemes = \"dark\"\n",
        ] {
            assert!(read(typo).is_err(), "silently ignored: {typo}");
        }

        // ...and so is a value outside the vocabulary, which serde names for us
        // along with the words it would have taken.
        let refused = read("[defaults]\ntheme = \"drak\"\n").expect_err("`drak` is not a theme");
        assert!(refused.contains("drak"), "got: {refused}");
        assert!(refused.contains("dark"), "the alternatives: {refused}");

        // `diag` is deliberately not one of the four views: a session that
        // opened on the pty instrument would be one whose config file had been
        // left in a debugging state.
        assert!(read("[defaults]\nview = \"diag\"\n").is_err());
    }

    // --- presets in the table ---------------------------------------------

    #[test]
    fn a_preset_becomes_a_row_of_the_table_behind_the_built_in_it_hosts() {
        let table = config(FLEET).table();

        // Appended rather than merged: abeam's own agents keep their places, so
        // nothing a user writes can change what `+claude` means.
        assert_eq!(names(table), vec!["claude", "copilot", "codex", "fleet"]);

        let fleet = crate::agent::find_within("fleet", table).expect("the preset is in the table");
        // The host's candidates, because the thing that would be missing really
        // is Claude — and the host's own install sentence with them.
        assert_eq!(
            fleet.candidates,
            crate::agent::find("claude").unwrap().candidates
        );
        assert_eq!(fleet.install, crate::agent::find("claude").unwrap().install);
        // Two names: what to call it, and what it is.
        assert_eq!(fleet.name, "fleet");
        assert_eq!(fleet.hosts, "claude");
        assert_eq!(fleet.args, ["agent"]);

        // And a preset is not in the *built-in* table, which is the whole of
        // why a preset cannot name another preset: `find` is what a `host` is
        // looked up with, and it reads `AGENTS` alone.
        assert!(crate::agent::find("fleet").is_none());
    }

    #[test]
    fn a_preset_whose_host_is_a_program_is_a_path_lookup_and_says_where_it_came_from() {
        let mut config = config("[preset.nu]\nhost = \"nu\"\n");
        config.path = Some(PathBuf::from("/home/philm/.config/abeam/abeam.toml"));
        let table = config.table();

        let nu = crate::agent::find_within("nu", table).expect("the preset is in the table");
        // One candidate: the word as written, for `crate::launch` to find on
        // `PATH` exactly as `abeam +nu` would.
        assert_eq!(nu.candidates, ["nu"]);
        assert_eq!(nu.hosts, "nu");
        // There is nothing abeam knows about installing somebody's own program,
        // so the sentence on the failure path points at the file instead — the
        // mistake behind this message is usually three characters long.
        assert!(nu.install.contains("abeam.toml"), "got: {}", nu.install);
        assert!(nu.install.contains("`nu`"), "got: {}", nu.install);
    }

    #[test]
    fn a_preset_may_not_take_a_name_abeam_already_answers() {
        // A built-in, which is the case that matters: `[preset.claude]` would
        // make the real Claude unreachable and nothing on screen would say so.
        for name in ["claude", "Claude", "COPILOT", "codex", "CODEX"] {
            let refused = read(&format!("[preset.{name}]\nhost = \"claude\"\n"))
                .expect_err("a built-in's name is not a preset's to take");
            assert!(refused.contains(name), "the conflict is named: {refused}");
            assert!(refused.contains("already knows"), "got: {refused}");
            // With the way out, which is a rename plus the `host` line that
            // says what they were actually asking for.
            assert!(refused.contains("host = "), "got: {refused}");
        }

        // abeam's own two words, which are answered before the table is
        // consulted at all — so a preset called `help` would simply never run.
        for name in ["help", "version", "HELP"] {
            let refused = read(&format!("[preset.{name}]\nhost = \"claude\"\n"))
                .expect_err("a reserved word is not a preset's to take either");
            assert!(refused.contains("never reach it"), "got: {refused}");
        }

        // Two presets abeam cannot tell apart, which is the same objection
        // arriving between two rows the user wrote themselves.
        let refused =
            read("[preset.fleet]\nhost = \"claude\"\n[preset.FLEET]\nhost = \"claude\"\n")
                .expect_err("two spellings of one name");
        assert!(refused.contains("cannot tell apart"), "got: {refused}");

        // A name that names nothing, and a host that starts nothing.
        assert!(read("[preset.\"\"]\nhost = \"claude\"\n").is_err());
        assert!(read("[preset.\"  \"]\nhost = \"claude\"\n").is_err());
        let refused =
            read("[preset.fleet]\nhost = \"\"\n").expect_err("a blank host names nothing");
        assert!(refused.contains("names nothing to start"), "got: {refused}");
    }

    // --- what a preset does to the command line ----------------------------

    #[test]
    fn a_preset_is_selected_and_refused_exactly_as_a_built_in_is() {
        let table = config(FLEET).table();

        // Behind the sigil, folded, with the rest of the line left for the
        // child — none of which needed a line of code for presets.
        let (chosen, rest) = match parse_with(&args(&["+fleet", "--resume"]), None, table) {
            Ok(Cli::Host {
                choice: Choice::Known(agent),
                args,
            }) => (agent.name, args),
            other => panic!("expected the preset, got {other:?}"),
        };
        assert_eq!(chosen, "fleet");
        assert_eq!(rest, args(&["--resume"]));

        // Through `ABEAM_AGENT`, which the module docs promise costs no code.
        assert!(matches!(
            parse_with(&[], Some("FLEET".into()), table),
            Ok(Cli::Host {
                choice: Choice::Known(_),
                ..
            })
        ));

        // ...and refused in front of the sigil, which is the refusal in
        // `parse_with` growing with the table. `abeam fleet` is the same
        // mistake as `abeam claude`, made by the one person on the machine most
        // likely to believe the word means their preset.
        let refused =
            parse_with(&args(&["fleet"]), None, table).expect_err("a preset name is a selection");
        assert!(refused.contains("used to host"), "got: {refused}");
        assert!(refused.contains("`abeam +fleet`"), "got: {refused}");
        assert!(refused.contains("`abeam -- fleet`"), "got: {refused}");

        // Against the built-in table the same word is an argument, which is
        // what keeps this refusal a fixed lookup rather than a `PATH` probe by
        // the back door.
        assert!(matches!(
            parse_with(&args(&["fleet"]), None, AGENTS),
            Ok(Cli::Host { .. })
        ));
    }

    #[test]
    fn a_presets_arguments_reach_the_child_in_front_of_the_ones_that_were_typed() {
        // `abeam +fleet --resume` is `claude agent --resume`. Whether it can be
        // *started* is a fact about this machine, so what is asserted is the
        // line either way: the found path carries it in `launch.args`, and the
        // missing path is Claude's own missing message.
        let table = config(FLEET).table();
        let fleet = crate::agent::find_within("fleet", table).unwrap();

        match resolve_within(fleet, &args(&["--resume"]), table) {
            Ok(hosted) => {
                assert_eq!(hosted.launch.args, args(&["agent", "--resume"]));
                // The border says what was asked for; `crate::dispatch` is told
                // what is running, so a preset does not cost the queue its
                // dispatch mode.
                assert_eq!(hosted.name, "fleet");
                assert_eq!(hosted.agent, "claude");
            }
            Err(why) => assert!(
                why.contains("`fleet`") && why.contains("`claude`"),
                "a preset that cannot start says both names: {why}"
            ),
        }
    }

    // --- the opening state -------------------------------------------------

    #[test]
    fn defaults_open_the_session_and_a_preset_overrides_them_field_by_field() {
        // `[defaults]` on its own is every session on this machine.
        let fleet = config(FLEET);
        assert_eq!(
            fleet.opening(None),
            Opening {
                view: RightView::Git,
                focus: Focus::Left,
                zoom: false,
                theme: Theme::Light,
            }
        );

        // ...and the preset says two of the four differently, which is what
        // "field by field" means: `view` and `theme` move, `focus` and `zoom`
        // are the preset agreeing with the defaults rather than silently
        // resetting them.
        assert_eq!(
            fleet.opening(Some("fleet")),
            Opening {
                view: RightView::Queue,
                focus: Focus::Left,
                zoom: false,
                theme: Theme::Dark,
            }
        );
        // Folded, like every other name behind a `+`.
        assert_eq!(fleet.opening(Some("FLEET")), fleet.opening(Some("fleet")));

        // A preset that says nothing about the opening state changes none of
        // it, which is the case that would break if absence were read as a
        // value.
        let quiet = config(
            "[defaults]\nview = \"shell\"\nzoom = true\ntheme = \"light\"\n\
             [preset.q]\nhost = \"claude\"\n",
        );
        assert_eq!(
            quiet.opening(Some("q")),
            Opening {
                view: RightView::Shell,
                focus: Focus::Left,
                zoom: true,
                theme: Theme::Light,
            }
        );

        // A built-in's name finds no preset and gets the defaults: `abeam
        // +claude` is a request to host Claude and says nothing about panes.
        assert_eq!(quiet.opening(Some("claude")).view, RightView::Shell);

        // And with no file at all, abeam opens where it has always opened.
        assert_eq!(Config::default().opening(None), Opening::default());
        assert_eq!(Opening::default().theme, Theme::Dark);
        assert_eq!(Opening::default().view, RightView::Git);
    }

    #[test]
    fn every_word_the_two_vocabularies_take_maps_to_something() {
        // Five views and two sides, spelled as the file spells them. This is
        // the test that fails if a variant is added to `RightView` and its word
        // is not decided on here, which is the right way round: the vocabulary
        // is abeam's promise to a file somebody has already written.
        //
        // The exhaustive `match` below is what makes that sentence true rather
        // than merely intended, and it is here because the sentence was not
        // true when it was written: this was a list of pairs, `RightView::Ask`
        // was added next door, and every test in this file went on passing. A
        // list can only fail to mention a variant. A `match` cannot compile
        // without one — so a new view either gets a word here or states, in
        // this file, why it has none, which is what `Diag`'s arm is.
        for view in [
            RightView::Git,
            RightView::Viewer,
            RightView::Shell,
            RightView::Queue,
            RightView::Diag,
            RightView::Ask,
        ] {
            let word = match view {
                RightView::Git => Some("git"),
                RightView::Viewer => Some("files"),
                RightView::Shell => Some("shell"),
                RightView::Queue => Some("queue"),
                RightView::Ask => Some("ask"),
                // Outside the vocabulary on purpose; [`View`]'s own docs carry
                // the argument, and there is nothing to assert about a word
                // that does not exist.
                RightView::Diag => None,
            };
            let Some(word) = word else { continue };
            let config = config(&format!("[defaults]\nview = \"{word}\"\n"));
            assert_eq!(config.opening(None).view, view, "`{word}`");
        }
        for (word, focus) in [("left", Focus::Left), ("right", Focus::Right)] {
            let config = config(&format!("[defaults]\nfocus = \"{word}\"\n"));
            assert_eq!(config.opening(None).focus, focus, "`{word}`");
        }
        for (word, theme) in [("dark", Theme::Dark), ("light", Theme::Light)] {
            let config = config(&format!("[defaults]\ntheme = \"{word}\"\n"));
            assert_eq!(config.opening(None).theme, theme, "`{word}`");
        }
    }
}
