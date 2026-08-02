//! The AI tools abeam knows how to start, and how a word on the command line
//! becomes one.
//!
//! abeam hosts one program in its left pane, and for most of its life that
//! program was the string `"claude"` written into `main`. This module is what
//! replaced it: a short table of agents, each with the executables to look for
//! and one sentence about installing it, plus the rules for working out which
//! of them — or which other program entirely — was asked for.
//!
//! It is [`crate::panes::shell`]'s sibling on purpose, down to the shape: a
//! small list of known programs, tried best first, with an environment variable
//! that overrides the lot. The two differ in exactly one place, and the reason
//! is worth stating. A shell is chosen by `ABEAM_SHELL` alone because no shell
//! is ever named on abeam's command line; an agent is usually chosen by the
//! first word after `abeam`, so this module owns an argument parser as well as
//! a table.
//!
//! ## Why there is no `--agent` flag
//!
//! The positional argument already selects the program — `abeam powershell` has
//! meant "host powershell" since long before there was a table. A flag beside
//! it would make `abeam --agent copilot powershell` expressible, and there is
//! no honest answer to what that means. So the first non-flag token is the
//! whole of the selection: a name in [`AGENTS`] picks that agent, and anything
//! else is a program to resolve exactly as before.
//!
//! ## Why abeam stops parsing at the first non-flag token
//!
//! Everything from the selector onwards belongs to the child. `abeam claude
//! --resume` has to resume Claude, and `abeam copilot --help` has to be
//! Copilot's help rather than abeam's — a multiplexer that quietly ate a flag
//! meant for the thing it is hosting would be wrong in a way that is very hard
//! to see from the outside. abeam's own two flags therefore have to come first,
//! and `--` is there for the one case the rule cannot cover on its own: a
//! program whose own name starts with a dash.
//!
//! ## Why an agent that is missing is a sentence and never a download
//!
//! There is a route that would have started Copilot on a machine that does not
//! have it. Modern `gh copilot` is not the retired suggest/explain extension
//! but a launcher: it runs the Copilot CLI from `PATH` if it is there, and
//! downloads it if it is not. abeam had that fallback for a day, with the
//! border reading `copilot · via gh` so that nobody could be fetched a program
//! without seeing it happen.
//!
//! It was taken out on purpose, and the decision was the user's rather than
//! this file's. abeam is a host. Typing `abeam copilot` is a request to run
//! something, not consent for a network install, and the gap between those two
//! is not one a terminal border can close after the fact. So the only thing
//! this module does with a name it cannot find is say so, and say how to fix
//! it — `gh copilot` included, as a command a person runs themselves.
//!
//! That is why [`Agent`] has three fields and not four, and why
//! [`Hosted`] has no notion of *how* something was started: there is one way.

use crate::launch::{self, Launch};

/// An agent abeam knows how to start.
#[derive(Debug)]
pub struct Agent {
    /// What the user types, and what the border shows. Never a path.
    pub name: &'static str,
    /// The executables to look for, best first. A list rather than a name
    /// because renames happen and a machine can be mid-migration; today every
    /// entry here has one, which is the least interesting way for that to be
    /// true.
    ///
    /// This is the whole of the search. When none of these is on the machine
    /// there is no second thing to try — see the module docs.
    pub candidates: &'static [&'static str],
    /// How to install it, in one sentence. Read only on the failure path, and
    /// it is the whole reason that path is worth writing carefully — someone
    /// reading it has abeam in front of them and the agent nowhere.
    pub install: &'static str,
}

/// The agents abeam knows, and the only place their names are written down.
pub const AGENTS: &[Agent] = &[
    Agent {
        name: "claude",
        // One candidate because there is genuinely only one name: the native
        // installer writes `claude.exe` into `~\.local\bin` and npm writes
        // `claude.cmd` into `%APPDATA%\npm`, and `crate::launch` already knows
        // how to start either of those under the one name they share.
        candidates: &["claude"],
        install: "Install Claude Code with its native installer, or with \
                  `npm i -g @anthropic-ai/claude-code`.",
    },
    Agent {
        name: "copilot",
        candidates: &["copilot"],
        // Both routes onto a machine, and `gh copilot` is here as something the
        // reader runs rather than something abeam runs for them. It is worth
        // naming precisely because it is the one that works where the others do
        // not: the npm package wants Node 22, which plenty of machines are not
        // on yet.
        install: "Install it with `winget install GitHub.Copilot`, or run \
                  `gh copilot` once to fetch it.",
    },
];

/// The agent abeam hosts when nothing and nobody named one.
///
/// A name rather than an index, so that reordering [`AGENTS`] cannot silently
/// change what `abeam` on its own does. It has to be a name in the table, which
/// is asserted by a test rather than by a type.
pub const DEFAULT: &str = "claude";

/// The agent this name selects, if it is one abeam knows.
///
/// Case-insensitively, because every other name on this platform is: `PATH`
/// lookups are, file names are, and `abeam Claude` finding `claude.exe` while
/// `abeam claude` finds the preset would be a distinction with no visible
/// cause.
pub fn find(name: &str) -> Option<&'static Agent> {
    AGENTS.iter().find(|a| a.name.eq_ignore_ascii_case(name))
}

/// What abeam was asked to host.
#[derive(Debug)]
pub enum Choice {
    /// A name from [`AGENTS`].
    Known(&'static Agent),
    /// Anything else, meaning exactly what `abeam powershell` has always meant.
    Program(String),
}

impl Choice {
    fn of(name: &str) -> Self {
        match find(name) {
            Some(agent) => Choice::Known(agent),
            None => Choice::Program(name.to_string()),
        }
    }
}

/// What the command line asked abeam to do.
#[derive(Debug)]
pub enum Cli {
    Help,
    Version,
    Host { choice: Choice, args: Vec<String> },
}

/// Read `abeam`'s own arguments, with `ABEAM_AGENT` as the default.
pub fn parse(args: &[String]) -> Result<Cli, String> {
    parse_with(args, std::env::var("ABEAM_AGENT").ok())
}

/// [`parse`], with the default handed in rather than read.
///
/// Split out for the tests, which cannot touch the process environment: it is
/// shared with the two hundred other tests running beside them, and half of
/// those spawn children that inherit it.
pub fn parse_with(args: &[String], default: Option<String>) -> Result<Cli, String> {
    // abeam has two flags and both of them answer and exit, so the first token
    // is the only place one can be — a flag that did *not* end the parse would
    // need a loop here, and there is no such flag. Everything after the first
    // non-flag token belongs to the child, `--help` included.
    if let Some(first) = args.first().filter(|token| is_flag(token)) {
        return match first.as_str() {
            "-h" | "--help" => Ok(Cli::Help),
            "-V" | "--version" => Ok(Cli::Version),
            other => Err(unknown(other)),
        };
    }

    // `--` ends abeam's own parsing rather than the command line: what follows
    // is the program even though it starts with a dash, which is the only way
    // to name one that does. It does not fence off the table — a name in
    // [`AGENTS`] never starts with a dash, so `abeam -- claude` would be a
    // second spelling of `abeam claude` and not a different request.
    let rest = match args.split_first() {
        Some((first, tail)) if first == "--" => tail,
        _ => args,
    };

    let (asked, args) = match rest.split_first() {
        // A typed name that is nothing is refused rather than defaulted, which
        // is the opposite of what an empty `ABEAM_AGENT` gets and is meant to
        // be. A variable is a *default*, and a default nobody set is a default
        // that should not be there; an argument is a *request*, and `abeam ""
        // --resume` quietly becoming `abeam claude --resume` would hand
        // somebody's arguments to a program they did not name — the one thing
        // this parser exists not to do.
        Some((first, _)) if is_blank(first) => return Err(nothing()),
        Some((first, tail)) => (first.clone(), tail.to_vec()),
        // Nothing was named, so the default decides — and it is read on exactly
        // the same terms a typed name would be, an agent or a program. That is
        // what `ABEAM_SHELL` does, and two overrides that looked alike while
        // meaning different things would be worse than either.
        //
        // A blank value counts as unset. PowerShell will happily leave
        // `$env:ABEAM_AGENT = ""` behind, and "``" was not found on PATH" names
        // nothing a reader can act on.
        None => (
            default
                .filter(|name| !is_blank(name))
                .unwrap_or_else(|| DEFAULT.to_string()),
            Vec::new(),
        ),
    };

    Ok(Cli::Host {
        choice: Choice::of(&asked),
        args,
    })
}

/// A token abeam would read as one of its own flags.
///
/// `--` is not one: it is a fence, handled where the fence matters. A lone `-`
/// is not one either — it is a dash with no flag behind it, so it goes through
/// as a program name and fails saying so, which is more use than a complaint
/// about a flag nobody typed.
fn is_flag(token: &str) -> bool {
    token.starts_with('-') && token != "-" && token != "--"
}

/// A name that names nothing, wherever it came from.
///
/// Whitespace and not merely emptiness, because the two arrive by the same
/// route and neither is a program: PowerShell leaves `$env:ABEAM_AGENT = ""`
/// behind when a variable is cleared and passes `abeam " "` through as one
/// token about as readily. "`   ` was not found on PATH" is no more use than
/// "`` was not found on PATH", so one rule covers both spellings on both paths.
fn is_blank(name: &str) -> bool {
    name.trim().is_empty()
}

fn nothing() -> String {
    "abeam was given an empty program name, which names nothing it can look \
     for. `abeam` on its own hosts the default agent, and `abeam <agent>` or \
     `abeam <program>` hosts what you name. An empty argument is usually a \
     shell variable that is not set: PowerShell passes `\"$env:THING\"` on as \
     an empty string rather than dropping it."
        .to_string()
}

fn unknown(flag: &str) -> String {
    format!(
        "`{flag}` is not an abeam flag. abeam takes `-h`/`--help` and \
         `-V`/`--version`, and nothing else: everything from the agent or \
         program name onwards is passed to it, so `abeam claude --resume` \
         resumes Claude. Put `--` first to host a program whose own name \
         starts with a dash."
    )
}

/// What `abeam --help` prints.
///
/// Deliberately short. abeam is a terminal user interface with one argument and
/// two flags; the keys are the interesting part and they are behind `F1`, where
/// they can be read next to the thing they act on. The agents are listed from
/// [`AGENTS`] rather than written out, because a help text that can disagree
/// with the table eventually does.
pub fn help() -> String {
    let agents: Vec<&str> = AGENTS.iter().map(|a| a.name).collect();
    format!(
        "abeam - one window for an AI coding session.

Usage:
  abeam                      host the default agent
  abeam <agent> [args...]    host one of the agents below
  abeam <program> [args...]  host any program on PATH
  abeam -- <program> [...]   ...even one whose name starts with a dash

Agents: {}

Flags:
  -h, --help     this
  -V, --version  the version

Everything from the agent or program onwards is passed to it, so abeam's own
flags have to come first: `abeam claude --resume` resumes Claude.

ABEAM_AGENT names the default. It may be one of the agents above, or any
program.

F1 inside abeam lists the keys.",
        agents.join(", ")
    )
}

pub fn version() -> String {
    format!("abeam {}", env!("CARGO_PKG_VERSION"))
}

/// A resolved agent or program, ready for the left pane.
#[derive(Debug)]
pub struct Hosted {
    /// What to call it: the agent's name, or the program as it was typed.
    /// Never the path it *resolved to* — see [`abeam_pty::PtyConfig::title`].
    /// A program typed as a path keeps the path that was typed, which is the
    /// same rule rather than an exception to it: `abeam .\tools\agent.exe` puts
    /// exactly that on the border, not the absolute path it became, and
    /// certainly not the `cmd.exe` an npm shim routes through.
    ///
    /// One field, and it was briefly two. While the launcher fallback existed
    /// the border had something to say that the line abeam prints on the way
    /// out did not — `copilot · via gh` — so `name` and `title` were separate.
    /// With one way to start an agent there is one name for it, and a second
    /// field that always held the same string would be a question for every
    /// reader after this one.
    pub name: String,
    pub launch: Launch,
}

impl Hosted {
    /// A program that was named outright, and so is its own explanation.
    pub fn plain(name: &str, launch: Launch) -> Self {
        Self {
            name: name.to_string(),
            launch,
        }
    }
}

/// Find this agent on the machine, or say what was looked for.
pub fn resolve(agent: &Agent, args: &[String]) -> Result<Hosted, String> {
    resolve_within(agent, args, AGENTS)
}

/// [`resolve`], over a table handed in rather than [`AGENTS`].
///
/// Split out for the tests, and not only for tidiness: Copilot is not installed
/// on the machine this was written on and cannot easily be — its npm package
/// wants Node 22 and this box has 20 — so the failure message is only reachable
/// at all with a table whose candidates are known to be absent. Testing it
/// against the real table would mean testing whichever branch the machine
/// happened to make reachable, which on a build server is the other one.
fn resolve_within(agent: &Agent, args: &[String], table: &[Agent]) -> Result<Hosted, String> {
    // Only the last reason is kept, for the same reason `panes::shell::start`
    // keeps only the last: with a list, the earlier entries are the ones
    // expected to be missing, and leading with those is leading with the least
    // informative half of the answer.
    let mut why = String::new();

    for candidate in agent.candidates {
        match launch::resolve(candidate, args) {
            // The agent's own name, not the absolute path it turned into and
            // not the `cmd.exe` an npm shim routes through. Those are facts
            // about starting it, and the border has 46 columns for facts about
            // what is taking the typing.
            Ok(launch) => {
                return Ok(Hosted {
                    name: agent.name.to_string(),
                    launch,
                });
            }
            Err(reason) => why = reason,
        }
    }

    // And that is the end of the search. There is no second route to try — see
    // the module docs for the one that was written and then deliberately taken
    // out again.
    Err(missing(agent, table, &why))
}

/// What abeam says when there is nothing to host.
///
/// Every candidate by name, because which list this was is the whole diagnosis
/// — the same standard `panes::shell::failure` is held to.
///
/// The operating system's own reason goes second rather than last, which is the
/// opposite of what the shell pane does. That is not an oversight: the shell
/// pane is 46 columns with a bottom edge, and a four-row wrapped reason put
/// first would push the sentence that fixes the problem off it. This is
/// standard error on a full-width console, where nothing is pushed anywhere and
/// the last line is the one that is read — so the advice goes there.
///
/// Which matters more than it did. This message used to be what a reader saw
/// after abeam had already tried a launcher on their behalf; it is now the only
/// thing standing between them and a bare "not found".
fn missing(agent: &Agent, table: &[Agent], why: &str) -> String {
    let mut said = vec![
        format!("abeam could not start `{}`.", agent.name),
        String::new(),
        format!("Tried: {}.", quoted(agent.candidates)),
    ];
    if !why.is_empty() {
        said.push(why.to_string());
    }
    said.push(String::new());

    // The sentence that saves the ten minutes. The default agent missing on a
    // machine where another one is sitting on `PATH` is one word away from
    // working, and nothing else on this screen would say so.
    //
    // Candidates only, and that was true before there was one route rather than
    // two: `gh` being on `PATH` would have made "`copilot` is installed" a lie,
    // and the first thing it does about that is a download. An alternative
    // worth naming is one that is already there.
    for other in table.iter().filter(|other| other.name != agent.name) {
        if other
            .candidates
            .iter()
            .any(|name| launch::resolve(name, &[]).is_ok())
        {
            said.push(format!(
                "`{}` is installed; `abeam {}` would host it.",
                other.name, other.name
            ));
        }
    }

    said.push(agent.install.to_string());
    said.join("\n")
}

fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Windows-only like the rest of the suite. The parsing half would run
/// anywhere, but what it parses is a list of Windows program names and what it
/// hands them to is a `PATH` walk with `PATHEXT` in it.
///
/// Nothing here spawns a child. The programs the resolution tests reach for are
/// `cmd.exe` — which is on every Windows there is — and names beginning
/// `abeam-no-such-`, which are on none.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// A table whose every agent is certainly absent, so the failure path and
    /// the message are reachable on any machine. This is why `resolve_within`
    /// takes a table: `copilot` cannot be installed on the machine abeam is
    /// developed on and `claude` is, so the real table exercises a different
    /// branch here than it would on a build server with neither.
    const ABSENT: &[Agent] = &[
        Agent {
            name: "abeam-test-one",
            candidates: &["abeam-no-such-agent-a", "abeam-no-such-agent-b"],
            install: "Install abeam-test-one by running `abeam-test-fetch`.",
        },
        Agent {
            name: "abeam-test-two",
            candidates: &["abeam-no-such-agent-c"],
            install: "Install abeam-test-two from the test suite.",
        },
    ];

    /// The same, except that the second agent is one every Windows has.
    const PRESENT: &[Agent] = &[
        Agent {
            name: "abeam-test-one",
            candidates: &["abeam-no-such-agent-a"],
            install: "Install abeam-test-one from the test suite.",
        },
        Agent {
            name: "abeam-test-two",
            candidates: &["cmd.exe"],
            install: "Install abeam-test-two from the test suite.",
        },
    ];

    /// An agent that is installed, so that the found half is testable too.
    const DIRECT: Agent = Agent {
        name: "abeam-test-direct",
        candidates: &["abeam-no-such-agent-d", "cmd.exe"],
        install: "Install abeam-test-direct from the test suite.",
    };

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    /// The selection as one word — `agent:claude` or `program:powershell` — so
    /// that an assertion can say which of the two it wanted without a table in
    /// it, and the arguments that were left for the child.
    fn chose(argv: &[&str], default: Option<&str>) -> (String, Vec<String>) {
        match parse_with(&args(argv), default.map(String::from)).expect("a command line") {
            Cli::Host {
                choice: Choice::Known(agent),
                args,
            } => (format!("agent:{}", agent.name), args),
            Cli::Host {
                choice: Choice::Program(program),
                args,
            } => (format!("program:{program}"), args),
            other => panic!("expected something to host, got {other:?}"),
        }
    }

    // --- the table --------------------------------------------------------

    #[test]
    fn every_agent_abeam_knows_is_findable_and_the_default_is_one_of_them() {
        assert_eq!(find("claude").expect("claude is known").name, "claude");
        assert_eq!(find("copilot").expect("copilot is known").name, "copilot");
        // A name that is not in the table is not half-matched into one: it is a
        // program, and `abeam powershell` has to keep meaning what it meant.
        assert!(find("powershell").is_none());
        assert!(find("claude-code").is_none());
        assert!(find("").is_none());

        // Case-insensitively, like every other name on this platform.
        assert_eq!(find("Claude").expect("Claude is claude").name, "claude");
        assert_eq!(find("COPILOT").expect("COPILOT is copilot").name, "copilot");

        // The one invariant a `&'static str` default cannot carry itself.
        assert!(
            find(DEFAULT).is_some(),
            "the default agent has to be in the table"
        );

        // Every agent's whole story is its candidates and one sentence. There
        // is no second route out of a name that is not on the machine, which is
        // a decision rather than a gap — see the module docs.
        for agent in AGENTS {
            assert!(
                !agent.candidates.is_empty(),
                "{} has nothing to look for",
                agent.name
            );
            assert!(!agent.install.is_empty(), "{} has no way in", agent.name);
        }

        // `gh copilot` is named in the hint and nowhere else in this file: it
        // is the route that works where the other two do not — the npm package
        // wants Node 22 — and it is a command the reader runs, not one abeam
        // runs for them.
        let copilot = find("copilot").unwrap().install;
        assert!(
            copilot.contains("winget install GitHub.Copilot"),
            "got: {copilot}"
        );
        assert!(copilot.contains("gh copilot"), "got: {copilot}");
    }

    // --- selection --------------------------------------------------------

    #[test]
    fn the_first_positional_selects_and_everything_after_it_belongs_to_the_child() {
        // Nothing named at all: the default.
        assert_eq!(chose(&[], None), ("agent:claude".into(), vec![]));

        // A name in the table selects that agent...
        assert_eq!(chose(&["copilot"], None), ("agent:copilot".into(), vec![]));
        // ...and one that is not keeps today's meaning exactly.
        assert_eq!(
            chose(&["powershell"], None),
            ("program:powershell".into(), vec![])
        );

        // The boundary. abeam parses its own flags only up to here, so these
        // are the child's — including the one abeam has a use for itself.
        assert_eq!(
            chose(&["copilot", "--resume"], None),
            ("agent:copilot".into(), args(&["--resume"]))
        );
        assert_eq!(
            chose(&["claude", "-p", "hi"], None),
            ("agent:claude".into(), args(&["-p", "hi"]))
        );
        assert_eq!(
            chose(&["claude", "--help"], None),
            ("agent:claude".into(), args(&["--help"])),
            "`abeam claude --help` is a question for Claude"
        );
        assert_eq!(
            chose(&["powershell", "-NoLogo", "-c", "gci"], None),
            ("program:powershell".into(), args(&["-NoLogo", "-c", "gci"]))
        );
    }

    #[test]
    fn a_double_dash_ends_abeams_parsing_and_not_the_command_line() {
        // The one case the "flags first" rule cannot cover on its own.
        assert_eq!(
            chose(&["--", "--weird-program"], None),
            ("program:--weird-program".into(), vec![])
        );
        assert_eq!(
            chose(&["--", "--weird-program", "-x", "--help"], None),
            ("program:--weird-program".into(), args(&["-x", "--help"]))
        );

        // It fences off abeam's flags, not the table. A name in `AGENTS` never
        // starts with a dash, so this is a second spelling of `abeam claude`
        // rather than a different request.
        assert_eq!(
            chose(&["--", "claude"], None),
            ("agent:claude".into(), vec![])
        );

        // A lone dash is a program name, not a flag with nothing behind it.
        assert_eq!(chose(&["-"], None), ("program:-".into(), vec![]));
    }

    #[test]
    fn abeam_agent_names_the_default_and_a_positional_overrides_it() {
        // It may name an agent...
        assert_eq!(
            chose(&[], Some("copilot")),
            ("agent:copilot".into(), vec![])
        );
        // ...or any program, exactly as ABEAM_SHELL may.
        assert_eq!(
            chose(&[], Some(r"C:\tools\nu.exe")),
            (r"program:C:\tools\nu.exe".into(), vec![])
        );

        // Overridden by anything typed, whichever kind each of them is.
        assert_eq!(
            chose(&["claude"], Some("copilot")),
            ("agent:claude".into(), vec![])
        );
        assert_eq!(
            chose(&["powershell"], Some("copilot")),
            ("program:powershell".into(), vec![])
        );
        assert_eq!(
            chose(&["copilot", "--resume"], Some("claude")),
            ("agent:copilot".into(), args(&["--resume"]))
        );

        // PowerShell leaves an emptied variable behind as an empty string, and
        // "`` was not found on PATH" names nothing anyone can act on. Blank
        // rather than empty, because `$env:ABEAM_AGENT = " "` is the same
        // non-answer and used to produce "`   ` was not found on PATH".
        assert_eq!(chose(&[], Some("")), ("agent:claude".into(), vec![]));
        assert_eq!(chose(&[], Some("   ")), ("agent:claude".into(), vec![]));
        assert_eq!(chose(&[], Some("\t")), ("agent:claude".into(), vec![]));
    }

    #[test]
    fn a_typed_name_that_is_nothing_is_a_sentence_rather_than_the_default() {
        // The other half of the same rule, going the other way on purpose. An
        // unset variable is a default that should not be there; a typed
        // argument is a request, and answering a request for nothing by
        // starting the default agent hands the arguments after it to a program
        // nobody named.
        for typed in ["", " ", "   ", "\t"] {
            let refused =
                parse_with(&args(&[typed]), None).expect_err("an empty name is not a program");
            assert!(refused.contains("empty"), "got: {refused}");
            // What used to come out, and what a reader could do nothing with.
            assert!(
                !refused.contains("was not found on PATH"),
                "the old non-answer is back: {refused}"
            );
        }

        // Especially with arguments after it, which is where defaulting would
        // do real damage.
        assert!(parse_with(&args(&["", "--resume"]), None).is_err());
        // Behind the fence too: `--` ends abeam's flags, it does not make a
        // blank token into a program name.
        assert!(parse_with(&args(&["--", "  "]), None).is_err());
        // A default that is set makes no difference either — the argument is
        // what was asked for, and it asked for nothing.
        assert!(parse_with(&args(&[""]), Some("copilot".into())).is_err());

        // ...and a blank *after* the selector belongs to the child, untouched.
        // An empty argument is an ordinary thing for a program to be sent.
        assert_eq!(
            chose(&["claude", "", "-p"], None),
            ("agent:claude".into(), args(&["", "-p"]))
        );
    }

    // --- abeam's own two flags --------------------------------------------

    #[test]
    fn help_and_version_are_answered_rather_than_spawned() {
        // The papercut this fixes: `abeam --help` used to reach
        // `CreateProcessW` as a program called `--help`.
        for flag in ["--help", "-h"] {
            assert!(
                matches!(parse_with(&args(&[flag]), None), Ok(Cli::Help)),
                "{flag} is help"
            );
        }
        for flag in ["--version", "-V"] {
            assert!(
                matches!(parse_with(&args(&[flag]), None), Ok(Cli::Version)),
                "{flag} is the version"
            );
        }
        // Answered before the selector is even looked at, so a default that
        // does not exist cannot swallow the question.
        assert!(matches!(
            parse_with(
                &args(&["--help", "copilot"]),
                Some("abeam-no-such-agent".into())
            ),
            Ok(Cli::Help)
        ));
    }

    #[test]
    fn an_unrecognised_flag_is_a_sentence_rather_than_a_spawn() {
        let refused = parse_with(&args(&["--colour"]), None).expect_err("not an abeam flag");
        assert!(refused.contains("--colour"), "got: {refused}");
        // Naming the flags that do exist is the whole job: without them this is
        // an error message that leaves you guessing.
        assert!(refused.contains("--help"), "got: {refused}");
        assert!(refused.contains("--version"), "got: {refused}");
        // "`--help` contains `--`" would pass this by accident, so it is the
        // whole phrase: the way out for a program whose name starts with a dash
        // is the one thing an unrecognised-flag message is likely to be about.
        assert!(
            refused.contains("Put `--` first"),
            "the way to name a dashed program is missing from: {refused}"
        );

        // Before the program, which is the only place abeam reads flags.
        assert!(parse_with(&args(&["-x", "claude"]), None).is_err());
        // ...and after it, it is not abeam's to complain about.
        assert_eq!(
            chose(&["claude", "--colour"], None),
            ("agent:claude".into(), args(&["--colour"]))
        );
    }

    #[test]
    fn the_help_names_every_agent_the_table_knows() {
        let help = help();
        for agent in AGENTS {
            assert!(
                help.contains(agent.name),
                "{} is missing from --help",
                agent.name
            );
        }
        assert!(help.contains("ABEAM_AGENT"), "the override has to be in it");
        assert!(help.contains("F1"), "the keys live behind F1, not here");
        assert!(help.contains("--help") && help.contains("--version"));
        // Short, because this is a terminal user interface and not a CLI with
        // fifty options. A page that scrolls is a page nobody reads.
        assert!(help.lines().count() < 30, "the help has grown a scrollbar");

        assert!(version().contains(env!("CARGO_PKG_VERSION")));
    }

    // --- finding it -------------------------------------------------------

    #[test]
    fn an_agent_that_is_installed_is_named_by_its_own_name_and_not_by_its_path() {
        let hosted = resolve_within(&DIRECT, &args(&["--resume"]), PRESENT).expect("cmd.exe");

        assert_eq!(hosted.name, "abeam-test-direct");
        // Which is worth asserting only because the thing it is *not* is right
        // here: an absolute path is what gets started, and what a 46-column
        // border must never show.
        assert!(hosted.launch.program.is_absolute());
        assert!(hosted.launch.program.ends_with("cmd.exe"));
        // The second candidate won, so the list is a list rather than a name
        // with room around it.
        assert_eq!(hosted.launch.args, args(&["--resume"]));
        // The user's arguments reach the child untouched, and nothing of
        // abeam's is added to them — no `--no-banner`, no anything, and now no
        // leading `copilot` or `--` either.
        assert_eq!(
            resolve_within(&DIRECT, &[], PRESENT).unwrap().launch.args,
            Vec::<String>::new(),
            "an agent that was asked for nothing is started with nothing"
        );
    }

    #[test]
    fn an_agent_that_is_not_installed_is_a_message_and_never_a_download() {
        // The reversal this file records. There was a route here that started
        // Copilot on a machine without it, by way of `gh`, and it is gone: a
        // name abeam cannot find produces the sentence and stops. Nothing on
        // this path may reach for a second program, and the only way to say so
        // from outside is that a table with nothing installed in it has exactly
        // one outcome.
        let refused = resolve_within(&ABSENT[0], &[], ABSENT).expect_err("nothing is installed");

        // Every candidate by name — the same standard `panes::shell`'s search
        // is held to, because which list this was is the whole diagnosis.
        for candidate in ABSENT[0].candidates {
            assert!(
                refused.contains(candidate),
                "{candidate} is missing from: {refused}"
            );
        }
        assert!(refused.contains("not found on PATH"), "got: {refused}");
        // The sentence somebody has to act on, and — with the fallback gone —
        // now the only thing between them and a bare "not found".
        assert!(refused.contains(ABSENT[0].install), "got: {refused}");
        // Including a fetch command, which is a thing to read and type rather
        // than a thing abeam did on the way past.
        assert!(refused.contains("abeam-test-fetch"), "got: {refused}");

        // Nothing else in this table is installed either, so nothing is offered
        // — an alternative that is not there is worse than none.
        assert!(
            !refused.contains("abeam-test-two"),
            "an absent agent was offered as an alternative: {refused}"
        );

        // ...and it fails the same way with arguments, which is where a
        // launcher used to insert a `--` of its own.
        let with = resolve_within(&ABSENT[0], &args(&["--resume"]), ABSENT)
            .expect_err("arguments do not conjure a program");
        assert!(with.contains("abeam-no-such-agent-a"), "got: {with}");
    }

    #[test]
    fn an_agent_that_is_missing_says_which_of_the_others_is_not() {
        // The ten minutes this saves: the default agent missing on a machine
        // where another one is sitting on `PATH` is one word away from working,
        // and nothing else abeam prints would say so.
        let refused = resolve_within(&PRESENT[0], &[], PRESENT).expect_err("not installed");

        assert!(
            refused.contains("`abeam-test-two` is installed"),
            "got: {refused}"
        );
        assert!(
            refused.contains("`abeam abeam-test-two` would host it"),
            "the sentence has to be the command to type: {refused}"
        );
    }
}
