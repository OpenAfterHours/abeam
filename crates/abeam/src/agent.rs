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
//! that overrides the lot. The two differ in one place, and it is a smaller
//! place than it was. A shell is chosen by `ABEAM_SHELL` alone, because no
//! shell is ever named on abeam's command line; an agent is chosen by
//! `ABEAM_AGENT` or by the one token on that line abeam is allowed to read. So
//! this module owns an argument parser as well as a table — an argument parser
//! whose whole vocabulary is a single character.
//!
//! ## The whole rule
//!
//! **Everything on the command line belongs to the hosted agent, except a
//! single leading token beginning `+`, which is abeam's.**
//!
//! `abeam agent` is `claude agent`. `abeam --resume` is `claude --resume`.
//! `abeam -p "fix the tests"` is `claude -p "fix the tests"`, and `uvx abeam`
//! written in front of any command line at all is the session that command line
//! would have started, with two panes around it. What is abeam's is the sigil:
//! `abeam +copilot --resume` hosts Copilot, `abeam +pwsh` hosts a shell, and
//! `+help` and `+version` answer and exit.
//!
//! **One exception and no others**, which is a sentence this module had to be
//! corrected to be able to write. A leading `--` used to be a second one: it
//! stopped abeam reading the line *and was eaten*, so `abeam -- --resume`
//! started `claude --resume` and resumed the session, where `claude --
//! --resume` sends the literal string `--resume` as a prompt. Two command lines
//! spelled the same meaning two different things, which is exactly the
//! divergence the rule above exists to remove. It is forwarded now — see
//! [`parse_with`], where the whole of the reasoning and its consequences are
//! written down — so abeam's argv is byte for byte what the agent would have
//! been handed, and `--` earns its place by fencing abeam's *reading* rather
//! than by being a token abeam keeps.
//!
//! ## What the first word used to mean, and why it stopped
//!
//! It selected. `abeam powershell` meant "host powershell" since long before
//! there was a table, and where this paragraph sits there used to be a section
//! headed "Why there is no `--agent` flag". Its argument was that the
//! positional already selected, so a flag beside it would make `abeam --agent
//! copilot powershell` expressible with no honest answer to what that meant.
//! The argument was about a *second* selector, it was right, and its premise
//! has gone: nothing positional selects now, so there is nothing for `+copilot`
//! to disagree with. It is recorded rather than deleted because the property it
//! was defending is the one this rule keeps — there is exactly one place a
//! selection can be written, and it is impossible to write two.
//!
//! What the positional cost was the thing abeam is for. abeam hosts an agent
//! and draws two panes beside it, and the less `abeam <args>` differs from
//! `claude <args>` the less there is to know — but `abeam agent` looked for a
//! program called `agent`, `abeam --resume` was abeam's business rather than
//! Claude's, and `abeam mcp list` started a program called `mcp`. Every
//! argument that happened not to begin with a dash was a trap, and the traps
//! could not be enumerated, because they are whatever subcommands the hosted
//! agent grows next and that is not a list this file can hold.
//!
//! The one thing the old meaning leaves behind is the refusal in
//! [`parse_with`]: a first token naming anything in the table — a built-in out
//! of [`AGENTS`] or one of the user's presets — is an error and not a
//! re-reading. It is permanent rather than transitional, and the comment at it
//! says why — `abeam claude` is what every older copy of the README tells
//! people to type, and silently passing `claude` to Claude is a confusing
//! failure with abeam's name nowhere on it.
//!
//! ## Why abeam reads one token and then stops
//!
//! This section was headed "Why abeam stops parsing at the first non-flag
//! token", and what happened to it is that the boundary moved rather than that
//! the argument was wrong. Everything from the selector onwards belonged to the
//! child; everything belongs to the child now, and there is no longer anything
//! to stop at.
//!
//! What it was defending is unchanged and worth restating. A multiplexer that
//! quietly ate a flag meant for the thing it is hosting would be wrong in a way
//! that is very hard to see from the outside — and abeam ate four spellings.
//! `-h`, `--help`, `-V` and `--version` were abeam's whenever they came first,
//! so `abeam --help` could not be Claude's help and you had to know to write
//! `abeam claude --help` to get it. All four are the agent's now, which is the
//! right answer to a question typed at the program taking the typing, and it is
//! why abeam's own two words are `+help` and `+version` — spellings nothing
//! else wanted.
//!
//! It also deleted a class of bug rather than guarding against it, which is the
//! strongest thing that can be said for the change. There was an `unknown()`
//! error in this file, and the papercut it was written for is recorded in
//! `main`: `abeam --help` reached `CreateProcessW` as a program named `--help`,
//! and the answer to a question about abeam was a spawn failure naming a flag.
//! That error caught it, and would have had to keep catching every leading
//! dashed token abeam did not recognise, forever. Under this rule nothing
//! *arrives* at a program name by being dashed — a token is either behind the
//! sigil, in which case somebody asked, or it is the agent's — so there is
//! nothing left to catch and the error is gone with the hazard rather than
//! standing in front of it.
//!
//! What this paragraph used to claim was stronger and was false, and the
//! correction is worth more than the claim was, because the claim's own point
//! is that "we added a check for it" and "it stopped being expressible" are
//! very different guarantees. It said a dashed token could no longer be a
//! program name *however it is spelled*. Three spellings refute that, all three
//! confirmed against the built binary: `abeam +--help` makes `--help` the
//! program name, because the sigil takes whatever word is behind it and does
//! not audit its shape; `ABEAM_AGENT=--help` does the same with no `+` anywhere
//! on the line; and `abeam +./-weird` reaches a dash-named *file* through
//! `main::host`'s relative-path branch. All three are somebody asking for a
//! dash-named program, which is a capability rather than a hazard, and the old
//! test suite pinned it deliberately.
//!
//! What actually keeps such a name off `CreateProcessW` is one line in
//! [`crate::launch`]: `find` answers only with a path it has located, and
//! otherwise returns "`--help` was not found on PATH". That is abeam's own
//! sentence about abeam's own search, it predates this change entirely, and it
//! is the mechanism this section should have credited from the start. What the
//! rule above removed was the *route* — `--help` becoming a program name with
//! nobody having named one.
//!
//! ## `ABEAM_AGENT`, which this change made worth setting
//!
//! **Its scope moved, and an earlier draft of this section said it had not.**
//! The words are the same — it names what to host, anything in the table or any
//! program at all, when no `+` token said otherwise; a blank value counts as
//! unset; a `+` token overrides it — and the set of command lines they cover is
//! much larger than it was. The old code consulted the default *only when argv
//! was empty*, because anything else was a positional that selected. The new
//! code consults it on every invocation that does not lead with a `+`.
//!
//! That is the trade this change is, stated as a cost rather than as a feature.
//! `ABEAM_AGENT=copilot abeam --resume` resumes Copilot today; before this it
//! was an unrecognised abeam flag, and the only way to name Copilot was a
//! positional which then shadowed everything you wanted to send after it — a
//! default that stopped applying the moment you had arguments was a default for
//! `abeam` on its own and for nothing else. The same line read the other way:
//! a variable somebody exported in a dotfile three years ago used to touch
//! nothing but bare `abeam`, and now silently redirects `abeam -p "commit my
//! changes"` into a different agent. It is worth setting *because* it applies
//! to everything, and there is no version of that which is only the good half.
//! The mitigations are that a `+` overrides it for one run, that the border
//! says which agent is taking the typing, and that [`nowhere`] names the
//! variable when what it holds cannot be found.
//!
//! **The variable holds a name and never a command line**, so `ABEAM_AGENT=+copilot`
//! — the spelling every other line of this documentation teaches — is refused
//! with the mistake named rather than accepted. Stripping the `+` was written
//! first and reverted: it would make the sigil part of a name in one place and
//! not in the other, which is precisely the thing the sigil is for saying it is
//! not. What is refused is only ever a value abeam was about to use, because
//! [`hosting`] is not reached at all when a `+` token overrode the variable.
//!
//! A preset name is a legal value there and costs no code to make so:
//! `ABEAM_AGENT=fleet abeam --resume` runs the preset's command line with
//! `--resume` on the end of it, because the variable is read through
//! [`Choice::of`] against the same table `+fleet` is read against. It is worth
//! writing down precisely because nothing was added for it — a reader looking
//! for the line that permits it will not find one.
//!
//! ## The table is built at run time, and this file is half of it
//!
//! [`AGENTS`] is still where abeam's own agents are written down and it is no
//! longer the whole table. `crate::config` reads a file out of the user's
//! profile and turns every `[preset.<name>]` in it into a row of exactly this
//! shape, which is what makes `+fleet` a command line rather than a feature:
//! every rule on this page then applies to a preset without a branch anywhere
//! for it. The refusal below grows to preset names. [`Choice::of`] finds one.
//! `+help` still wins over a preset called `help`, because that name is refused
//! when the file is read rather than shadowed when the line is parsed.
//!
//! Two rules keep that arrangement honest and both are enforced in
//! `crate::config`, at load time, which is the only moment at which anything
//! can be said about a name before somebody is waiting for a program to start.
//!
//! **A preset's `host` is resolved against [`AGENTS`] and `PATH`, and never
//! against another preset.** [`find`] is the built-in lookup and stays the
//! built-in lookup for exactly this reason: a preset named `claude` whose host
//! is `claude` is otherwise a row pointing at itself, and the parser has no way
//! to notice. There is no preset chaining, that is a decision rather than a
//! missing feature, and what it costs is naming one preset from another — which
//! is a saving of one line in a config file, bought with a cycle check on the
//! path that decides which program starts.
//!
//! **A preset may not take a built-in's name.** A `[preset.claude]` that
//! shadowed the entry above would make the built-in unreachable, with nothing
//! anywhere on screen saying which of the two was running. It is refused with
//! the conflict named, which is the same standard the refusal below is held to.
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
//! this file's. abeam is a host. Typing `abeam +copilot` is a request to run
//! something, not consent for a network install, and the gap between those two
//! is not one a terminal border can close after the fact. So the only thing
//! this module does with a name it cannot find is say so, and say how to fix
//! it — `gh copilot` included, as a command a person runs themselves.
//!
//! That is why [`Agent`] has no field saying *how* to start something, and why
//! [`Hosted`] has no notion of it either: there is one way. Both have since
//! grown a field for presets, and neither of those is that one — [`Agent::args`]
//! is what to pass and [`Hosted::agent`] is what the thing turned out to be,
//! and both are known before anything is spawned.

use crate::launch::{self, Launch};

/// An agent abeam knows how to start.
///
/// One row of the table a `+` token is read against: a built-in from [`AGENTS`],
/// or a preset `crate::config` read out of the user's profile. The two are one
/// type deliberately. A preset that behaved *almost* like a built-in on the
/// command line would be a second set of rules to learn for no gain, and every
/// reader of this table — the parser, the resolver, the help — would grow a
/// branch asking which kind it was holding.
///
/// Every field is `&'static`, and for a table half of which is read from a file
/// that is a claim about lifetime rather than about literals. `crate::config`
/// builds its rows once, at startup, out of strings it then leaks: the table
/// lives until the process exits whatever happens to it, so saying so outright
/// is cheaper than threading a lifetime through [`Cli`], [`Choice`] and every
/// test that builds a table of its own. That was the version that was written
/// first, and what it produced was a borrow parameter on six signatures to
/// describe memory nothing ever frees.
#[derive(Clone, Copy, Debug)]
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
    ///
    /// A preset whose host is a built-in borrows that built-in's sentence,
    /// because the thing that is missing really is Claude or Copilot. A preset
    /// whose host is an ordinary program has nothing abeam knows how to install,
    /// so its sentence names the config file instead — the reader's next move
    /// there is to open the file and look at what they asked for.
    pub install: &'static str,
    /// What this entry puts in front of the command line's own arguments.
    ///
    /// Empty for every built-in, and the whole of what a preset adds to the
    /// spawn: with `args = ["agent"]`, `abeam +fleet --resume` starts `claude
    /// agent --resume`. In front rather than behind, and it is the only order
    /// that can work — a subcommand is the first word of the line it belongs
    /// to, and everything typed after `+fleet` is being typed *at* whatever the
    /// preset names. Behind, `[preset.fleet] args = ["agent"]` would turn
    /// `abeam +fleet --resume` into `claude --resume agent`, which is a
    /// different command in every agent abeam hosts.
    pub args: &'static [&'static str],
    /// Which of abeam's own agents this actually is, for the questions that are
    /// about *what* is being hosted rather than about what to call it.
    ///
    /// Its own [`name`](Self::name) for a built-in, and the host's name for a
    /// preset — so a `[preset.fleet]` hosting `claude` answers `claude` here
    /// and `fleet` above. The distinction pays for itself in exactly one place
    /// today and it is not a cosmetic one: `crate::dispatch` will only start a
    /// background agent when what abeam is hosting is Claude, and a preset that
    /// answered `fleet` to that question would silently lose the queue's
    /// dispatch mode to a name in a config file.
    ///
    /// Not a `&'static Agent` back into the table, which is what this wants to
    /// be: a preset may host a program that is in no table at all, and a field
    /// that is `None` for that case makes every reader handle a shape that is
    /// really just "a name abeam has nothing else to say about".
    pub hosts: &'static str,
}

/// How to install Copilot, in the words of the platform this was built for.
///
/// The one sentence in [`AGENTS`] that cannot be written once. It is read on
/// the failure path, by somebody who has just been told the agent they asked
/// for is not on their machine, so the whole of its job is to be a command they
/// can type — and `winget` is a Windows package manager, which makes naming it
/// to a reader on Linux worse than naming nothing at all. npm's package is the
/// route Copilot CLI documents on every platform, which is why the Unix half
/// leads with it.
#[cfg(windows)]
const COPILOT_INSTALL: &str = "Install it with `winget install GitHub.Copilot`, or run \
                               `gh copilot` once to fetch it.";
#[cfg(unix)]
const COPILOT_INSTALL: &str = "Install it with `npm i -g @github/copilot`, or run \
                               `gh copilot` once to fetch it.";

/// The agents abeam knows, and the only place their names are written down.
///
/// Half of the table a `+` token is read against, and the only half abeam
/// wrote: the rest is whatever `crate::config` found in the user's profile.
/// Nothing here may be shadowed by that half — see the module docs — so this
/// stays a `const` that a preset is checked against rather than merged into.
pub const AGENTS: &[Agent] = &[
    Agent {
        name: "claude",
        // A built-in adds nothing to the command line it was given. That is
        // `crate::agentstate`'s promise as much as this file's — the record it
        // reads is written by the agent abeam started, and an argument abeam
        // slipped in unasked is an argument in somebody's `ps` output that they
        // did not type.
        args: &[],
        hosts: "claude",
        // One candidate because there is genuinely only one name: every route
        // onto a machine writes the file as plain `claude` — `~/.local/bin`
        // from the native installer, npm's global `bin` from the package, both
        // with a `.exe` or a `.cmd` on the end of them on Windows — and
        // `crate::launch` already knows how to start any of those under the one
        // name they share.
        candidates: &["claude"],
        install: "Install Claude Code with its native installer, or with \
                  `npm i -g @anthropic-ai/claude-code`.",
    },
    Agent {
        name: "copilot",
        args: &[],
        hosts: "copilot",
        candidates: &["copilot"],
        // Two routes on either platform, and `gh copilot` is here as something
        // the reader runs rather than something abeam runs for them. It is
        // worth naming precisely because it is the one that works where the
        // package managers do not: the npm package wants Node 22, which plenty
        // of machines are not on yet.
        install: COPILOT_INSTALL,
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
/// Case-insensitively, because what is being matched is abeam's own table of
/// names and not a file: `abeam +Claude` and `abeam +claude` are the same
/// request, and one of them finding the preset while the other fell through to
/// a `PATH` lookup for a program spelled `Claude` would be a distinction with
/// no visible cause. Whether that lookup then folds case is the filesystem's
/// business and `crate::launch`'s — it does on Windows and does not on Linux —
/// and neither answer belongs in a table abeam wrote itself.
///
/// The **built-in** table and nothing else, which is what makes this the
/// function two other rules are written in terms of rather than an accident of
/// where it was first needed.
///
/// `crate::config` resolves a preset's `host` through here, so a preset can
/// only ever name one of abeam's own agents or a program on `PATH` — never
/// another preset, and so never itself. `crate::dispatch` asks it whether the
/// thing abeam is hosting is Claude, which is a question about an agent abeam
/// wrote down and not about a name somebody chose in a file.
///
/// The whole table — presets included — is [`find_within`], and the parser uses
/// that one. Two functions rather than one with a flag, because the difference
/// between them is not a detail of the lookup: it is which of two questions is
/// being asked.
pub fn find(name: &str) -> Option<&'static Agent> {
    find_within(name, AGENTS)
}

/// The same lookup over a table handed in: the built-ins plus whatever
/// `crate::config` read.
///
/// The only place a table is consulted by name, and deliberately so: the parser
/// reads it to answer `+claude`, and reads it again to refuse a bare `claude`,
/// and those two must agree about what a name is for as long as there is one
/// table.
pub fn find_within<'t>(name: &str, table: &'t [Agent]) -> Option<&'t Agent> {
    table.iter().find(|a| a.name.eq_ignore_ascii_case(name))
}

/// Where a name came from, for the one question that is answered differently
/// depending on the answer: what to say when it turns out to name nothing.
///
/// Two ways in and two quite different mistakes behind them. A `+` token was
/// typed on this command line, and the most likely thing it is not is a prompt
/// that happened to start with a `+` — so its message names the `--` escape. A
/// default was read out of the environment, and the most likely thing wrong
/// with it is that it was set for another machine or another year — so its
/// message names the variable, which nothing else on the screen would.
///
/// Carried on the choice rather than worked out again at the failure, because
/// by then it cannot be: `main::host` holds a program name and a `PATH` walk
/// that failed, and no amount of looking at either says which of the two put
/// the name there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whence {
    /// The `+` token on the command line.
    Sigil,
    /// Whatever abeam falls back to when no `+` token named anything:
    /// `ABEAM_AGENT` when it holds something, and [`DEFAULT`] when it does not.
    /// One variant for both, because the message is the same sentence either
    /// way and abeam's own default is in the table by an invariant a test pins
    /// — so this arm reaching a failure at all means the variable.
    Default,
}

/// What abeam was asked to host.
#[derive(Debug)]
pub enum Choice {
    /// A name in the table: a built-in from [`AGENTS`], or a preset.
    Known(&'static Agent),
    /// Anything else, meaning exactly what `abeam powershell` used to mean and
    /// what `abeam +powershell` means now.
    Program {
        /// As it was typed, sigil already stripped.
        name: String,
        /// Which of the two ways in it came by — see [`Whence`].
        whence: Whence,
    },
}

impl Choice {
    /// The one place a name becomes a selection, whether it was typed behind a
    /// `+` or read out of `ABEAM_AGENT`.
    ///
    /// Which is why the variable takes a preset name with nothing added for it:
    /// there is one function turning a word into a choice, it is this one, and
    /// it has been reading the whole table since the whole table grew presets.
    ///
    /// The two callers differ in one argument and it is the one that is not
    /// about *selection* at all: a name in the table is the same agent whoever
    /// named it, and [`Whence`] is only ever read on the path where the name
    /// found nothing.
    fn of(name: &str, table: &'static [Agent], whence: Whence) -> Self {
        match find_within(name, table) {
            Some(agent) => Choice::Known(agent),
            None => Choice::Program {
                name: name.to_string(),
                whence,
            },
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

/// The one token on the command line that is abeam's.
///
/// A sigil rather than a flag or a subcommand, because it has to be a shape the
/// hosted agent's own command line is least likely to want: `-a` is a flag
/// something already has, and `agent` is a word Claude already takes. Only the
/// first token is read this way and there is only ever one, so a `+` in any
/// other position is a character — `claude config set +x` is a command line
/// somebody has, and it survives.
///
/// **The collision is narrowed and not removed, and this comment used to say
/// otherwise.** It called it impossible. It is not: a prompt may begin with a
/// `+`, and first position is exactly where a prompt lands. `abeam "+1 to
/// shipping this"` reads `1 to shipping this` as a program to host, because the
/// quotes were the shell's and abeam never saw them. That is a real cost of the
/// rule, paid on a line somebody will type, and the two things owed to whoever
/// types it are both here: `--` is the escape, and [`nowhere`] names it in the
/// message rather than leaving a bare "was not found on PATH" to be puzzled
/// over. What is *not* here is a guess about whether the text looks like prose
/// — the parser has one rule and applies it to every line, and a heuristic that
/// read some `+` words as prompts would make the answer depend on the words.
const SIGIL: char = '+';

/// The words behind the sigil that abeam answers itself.
///
/// Two, and the argument for it staying two is at the arm that reads them.
/// Written down as a list *as well as* as two `if`s because `crate::config` has
/// to refuse a preset that takes one: `[preset.help]` is a preset nothing can
/// ever select, since these are answered before the table is consulted at all,
/// and a reader who wrote one would get silence rather than a reason. Two files
/// with their own copy of a list eventually disagree about it, so there is one
/// copy and a test that pins each word to the answer it produces.
pub const RESERVED: &[&str] = &["help", "version"];

/// Read `abeam`'s own arguments, with `ABEAM_AGENT` as the default.
///
/// `table` is the built-ins plus the user's presets — `crate::config::Config::table`
/// builds it, and `main` builds it once before this is called.
pub fn parse(args: &[String], table: &'static [Agent]) -> Result<Cli, String> {
    parse_with(args, std::env::var("ABEAM_AGENT").ok(), table)
}

/// [`parse`], with the default handed in rather than read.
///
/// Split out for the tests, which cannot touch the process environment: it is
/// shared with the two hundred other tests running beside them, and half of
/// those spawn children that inherit it. The table is handed in for the same
/// reason twice over — it is read from a file in the user's profile, and a test
/// that wanted a preset in it would otherwise have to put one there.
pub fn parse_with(
    args: &[String],
    default: Option<String>,
    table: &'static [Agent],
) -> Result<Cli, String> {
    let Some((first, rest)) = args.split_first() else {
        return hosting(default, Vec::new(), table);
    };

    // `--` ends abeam's reading of the line rather than the line itself, and
    // the one thing it still has to cover is a first argument that begins with
    // a `+`. A shell will not do it for you: `abeam "+1 more"` arrives here as
    // a token starting `+` exactly as the unquoted form does, because the
    // quotes are the shell's and abeam never sees them. It is also the escape
    // from the refusal below — `abeam -- claude agent` is how you say `claude`
    // to the default agent and mean the word.
    //
    // **And it is forwarded, `--` and all.** `args` rather than `rest`, which
    // is a one-word change and the whole of what makes the rule at the top of
    // this file literally true rather than nearly true. It used to be `rest`:
    // abeam consumed the token, and `abeam -- --resume` therefore started
    // `claude --resume` and resumed the session, where `claude -- --resume`
    // hands the literal string `--resume` over as a prompt. Two spellings of
    // one command line meaning two different things is the exact defect the
    // flip exists to remove, and documenting the exception would have been
    // keeping the defect and writing it down. Deleting it is cheaper and
    // leaves nothing to remember.
    //
    // The fence is unaffected, because the fence was never the token being
    // eaten: a leading `--` means abeam reads nothing off this line, so neither
    // the `+` branch below nor the refusal below that can fire, whatever the
    // second word is. That is all `--` was ever doing here.
    //
    // Two consequences, both checked against the built binary rather than
    // reasoned about, and both still the behaviour we want:
    //
    //   `abeam -- claude agent`  is now  `claude -- claude agent`
    //   `abeam -- +1 more`       is now  `claude -- +1 more`
    //
    // Right in the only sense abeam gets to have an opinion about, which is
    // that each is *identical to what the agent would have been given* had
    // abeam not been in front of it. Whether `claude -- claude agent` is a
    // prompt or a complaint is Claude's business and changes with Claude's
    // version; what abeam owes the person typing is that the answer does not
    // depend on abeam being there. Under the old behaviour it did — abeam sent
    // `claude agent` where the same line typed at Claude sent `-- claude
    // agent` — and that is a difference nothing on screen would have explained.
    // Every argument parser that reads a `--` at all reads it as "the rest are
    // operands", which is the reading the message at [`ambiguous`] wants, so
    // this is also the arrangement in which abeam has to teach less.
    if first == "--" {
        return hosting(default, args.to_vec(), table);
    }

    if let Some(word) = first.strip_prefix(SIGIL) {
        // A sigil with nothing behind it is refused rather than defaulted, and
        // that is the opposite of what a blank `ABEAM_AGENT` gets on purpose. A
        // variable is a *default*, and a default nobody set is a default that
        // should not be there; a typed `+` is a *request*, and `abeam + --resume`
        // quietly becoming `abeam --resume` would hand somebody's arguments to a
        // program they did not name — the one thing this parser exists not to do.
        if is_blank(word) {
            return Err(nothing());
        }

        // Trimmed before anything is looked up, and trimmed at both ends
        // because one rule is cheaper to hold than two. `abeam "+claude "` is
        // one token here — the quotes are the shell's — and the table is
        // matched with `eq_ignore_ascii_case`, which is exact about everything
        // that is not case. So without this line a trailing space demoted
        // Claude to a program name and produced "`claude ` was not found on
        // PATH": an agent abeam knows perfectly well, missing, with the reason
        // one invisible character wide.
        //
        // The same trim `is_blank` above already does, made to apply to the
        // whole of what follows rather than to the emptiness check alone —
        // which is what it looked like it did. What it costs is a program whose
        // name really has a space on the end of it, which Windows cannot even
        // create and which nobody on any platform has; what it buys is that
        // `+claude `, `+ claude` and `+claude` are one request, as they read.
        let word = word.trim();

        // Two reserved words, and there is not going to be a third. `+h` and
        // `+V` are deliberately absent: every word behind the sigil is one more
        // program that can never be hosted under its own name, and a short form
        // spends that on two keystrokes of a command nobody types twice. What
        // the set costs today is programs called `help` and `version`, and both
        // are still reachable as a path — `abeam +./help` — which is the whole
        // reason the set is kept small rather than made empty.
        //
        // Folded like the table is, and for `find`'s reason: `abeam +HELP`
        // falling through to a `PATH` lookup for a program spelled `HELP` while
        // `abeam +help` printed this help would be a distinction with no visible
        // cause.
        if word.eq_ignore_ascii_case("help") {
            return Ok(Cli::Help);
        }
        if word.eq_ignore_ascii_case("version") {
            return Ok(Cli::Version);
        }

        return Ok(Cli::Host {
            choice: Choice::of(word, table, Whence::Sigil),
            args: rest.to_vec(),
        });
    }

    // The refusal, and it is permanent rather than transitional. `abeam claude`
    // hosted Claude for the whole of abeam's life before this rule, and it is
    // written that way in a README that is already cloned, already quoted and
    // already in somebody's shell history — people will type it in five years,
    // having read something that was true when it was written. What they get
    // without this line is `claude claude agent`: Claude's own complaint about
    // an argument it does not have, at one remove from anything they did, on a
    // screen that never mentions abeam.
    //
    // A fixed table lookup, and never a `PATH` probe. The tempting version of
    // this refuses any first token that is *both* a plausible program and a
    // plausible argument, which means asking the machine whether `agent` is on
    // `PATH` — and then abeam accepts a command line on one machine and refuses
    // it on the next, for a reason living in a directory nobody mentioned.
    // The table and never `PATH`, and that is now a table with somebody's
    // presets in it: `[preset.fleet]` makes `abeam fleet` a refusal too,
    // because it is the same mistake. A name that selects behind a `+` is a
    // name that used to select in front of one, whoever wrote it down — and the
    // reader who typed it is the one person on the machine most likely to
    // believe `fleet` means their preset. Still without asking the filesystem
    // anything: what grew is a list abeam read at startup, not a probe.
    if let Some(agent) = find_within(first, table) {
        return Err(ambiguous(agent.name, first));
    }

    // And everything else — dashed, blank, a subcommand, a prompt — is the
    // agent's, in the order it was typed.
    hosting(default, args.to_vec(), table)
}

/// Host whatever the default names, with these arguments.
///
/// The default is read on exactly the same terms a `+` token would be, an agent
/// or a program. That is what `ABEAM_SHELL` does, and two overrides that looked
/// alike while meaning different things would be worse than either.
///
/// Fallible for one value only — see the sigil check below — and it is a
/// `Result` rather than a special case at each of the three call sites because
/// all three are places abeam is about to *use* the variable. A caller that
/// only wanted to know what the default was would be a caller that should not
/// be refusing anything.
fn hosting(
    default: Option<String>,
    args: Vec<String>,
    table: &'static [Agent],
) -> Result<Cli, String> {
    // A blank value counts as unset. PowerShell will happily leave
    // `$env:ABEAM_AGENT = ""` behind, and "`` was not found on PATH" names
    // nothing a reader can act on.
    let name = default
        .filter(|name| !is_blank(name))
        .unwrap_or_else(|| DEFAULT.to_string());
    // ...and the same trim the sigil branch does, for the same reason and one
    // more: a variable is very often set by a script, and a trailing space in
    // an `export` line is invisible in every editor there is.
    let name = name.trim();

    // The variable holds a *name* and never a command line, so a `+` in it is a
    // mistake — and the commonest possible one, because `+copilot` is the
    // spelling every other page of this documentation teaches. Named rather
    // than stripped: stripping would make the sigil part of a name here and not
    // on the command line, teaching the opposite of the rule at the top of this
    // file, and it would do it silently. Named rather than left to fail at the
    // spawn, because "`+copilot` was not found on PATH" is true, useless, and
    // depends on whether some machine happens to have a file of that name.
    //
    // [`DEFAULT`] cannot trip this — it is a name in the table, pinned by a
    // test — which is the whole reason the message may say `ABEAM_AGENT`
    // outright rather than hedging about where the value came from.
    if let Some(word) = name.strip_prefix(SIGIL) {
        return Err(sigilled(name, word.trim()));
    }

    Ok(Cli::Host {
        choice: Choice::of(name, table, Whence::Default),
        args,
    })
}

/// A name that names nothing, wherever it came from.
///
/// Whitespace and not merely emptiness, because the two arrive by the same
/// route and neither is a program: PowerShell leaves `$env:ABEAM_AGENT = ""`
/// behind when a variable is cleared and passes `abeam "+ "` through as one
/// token about as readily. "`   ` was not found on PATH" is no more use than
/// "`` was not found on PATH", so one rule covers both spellings on both paths.
fn is_blank(name: &str) -> bool {
    name.trim().is_empty()
}

fn nothing() -> String {
    "abeam was given a `+` with no name behind it, which names nothing it can \
     look for. `abeam` on its own hosts the default agent and hands it the rest \
     of the line; `abeam +<agent>` or `abeam +<program>` hosts what you name. A \
     bare `+` is usually a shell variable that is not set: PowerShell passes \
     `+\"$env:THING\"` on as a lone `+` rather than dropping it."
        .to_string()
}

/// What abeam says about `ABEAM_AGENT=+copilot`.
///
/// `held` is the value as it stands and `word` is what is behind the sigil, so
/// the message can show the correction rather than describe it — the fix is one
/// character and the fastest way to say so is to print both spellings a line
/// apart.
fn sigilled(held: &str, word: &str) -> String {
    format!(
        "ABEAM_AGENT is set to `{held}`, and it holds a name rather than a \
         command line: the `+` is how a name is written *on the command line*, \
         where it says which token is abeam's. There is no token here to mark.\n\
         Set it to `{word}` instead. abeam will not quietly drop the `+` for \
         you: a sigil that was part of a name in one place and a marker in \
         another would be the confusion it exists to prevent."
    )
}

/// What abeam says to a command line that used to mean the other thing.
///
/// Both readings by name, because the reader has just typed something that was
/// correct for years and the failure they are being saved from is one where
/// nothing on screen would have said abeam's name. The two ways out are the
/// same line with one token changed, which is why they are spelled out in full
/// rather than described.
///
/// **The first token and never the whole line**, which is a correction. This
/// used to join `args` with single spaces and print the result inside `Write
/// \`abeam +…\``, so `abeam claude -p "fix the tests & ship it"` advised
/// writing ``abeam +claude -p fix the tests & ship it`` — which in `bash` runs
/// two commands, in PowerShell is a parse error, and in neither is what the
/// reader typed. abeam never saw the shell's quotes and cannot put them back:
/// argv is words by the time it arrives, and re-quoting would mean guessing a
/// shell. The doc comment conceded as much — it called the result "a shape to
/// recognise rather than one to paste" — while the sentence itself said
/// *Write*, which is an instruction to paste.
///
/// So the rewrite is described where it is small enough to be exact and the
/// rest of the line is left alone in words rather than in a quotation: only the
/// first token changes, and that is true of every line this message can be
/// printed for, however many arguments follow it and whatever is in them.
fn ambiguous(name: &str, first: &str) -> String {
    // A built-in and a preset arrive here by the same route and did not arrive
    // from the same past, and one sentence for both was false for the second.
    // `abeam claude` really did host Claude, for the whole of abeam's life
    // before the flip. `abeam fleet` never hosted anybody's preset — presets
    // are read behind the sigil and only there, and `[preset.fleet]` postdates
    // the flip entirely — so telling its author that it "used to host fleet" is
    // abeam being wrong about the one thing the reader knows for certain.
    // What is true of both is that the first word used to name what to start,
    // and `find` is the question that separates them: it reads the built-in
    // table alone, so a name it does not know is one somebody wrote themselves.
    let past = match find(name) {
        Some(_) => format!(
            "`abeam {first}` used to host {name}, which is the whole reason it \
             is refused rather than quietly passed on."
        ),
        None => format!(
            "`abeam {first}` used to host whatever `{first}` was on PATH — \
             never your `{first}` preset, which has only ever been selectable \
             behind the sigil. It is refused for the same reason a built-in's \
             name is: the word means something to abeam."
        ),
    };
    format!(
        "{past} The command line now belongs to the agent, so this would send \
         `{first}` to the default agent instead.\n\
         Change the first word and leave the rest of the line exactly as you \
         typed it: `abeam +{first}` hosts {name}, and `abeam -- {first}` fences \
         abeam off the line so that `{first}` reaches the default agent as the \
         word it is."
    )
}

/// What abeam says when the program it was pointed at is not on the machine.
///
/// [`crate::launch`]'s own sentence first — it is the specific one, and it
/// names the file it went looking for — and then one paragraph about *why
/// abeam was looking for that*, which is the half `launch` cannot know and the
/// half the reader is most often missing.
///
/// The two [`Whence`] arms are two different readers. One typed a `+` a second
/// ago and may have meant a prompt; the other set a variable at some point in
/// the past and has probably forgotten. Neither guesses: the sentence is the
/// same every time for a given route, and what varies is only which route it
/// was, which abeam knows for certain.
///
/// An agent out of the table never comes here — [`missing`] is its answer, and
/// it has candidates and an install sentence to work with. This is for a name
/// abeam knows nothing about beyond who wrote it down.
pub fn nowhere(asked: &str, whence: Whence, why: &str) -> String {
    let because = match whence {
        Whence::Sigil => format!(
            "abeam read `{asked}` as the program to host, because the `+` in \
             front of it is the one token on the command line abeam takes for \
             itself and the word behind it names what to start.\n\
             If you meant it as text, put `--` first: `abeam -- …` fences abeam \
             off the line entirely and the whole of it, `+` included, goes to \
             the default agent. A prompt beginning with a `+` is an ordinary \
             thing to type, and first position is exactly where one lands."
        ),
        Whence::Default => format!(
            "abeam hosts `{asked}` because that is what it falls back to when \
             no `+` token names anything — ABEAM_AGENT when it is set, and \
             `{DEFAULT}` when it is not.\n\
             Unset it or correct it, or override it for this one run with \
             `abeam +<agent>`. It is worth checking even if you did not set it \
             today: the variable applies to every command line now rather than \
             to bare `abeam` alone, so one left in a profile is one that \
             redirects everything."
        ),
    };
    format!("{why}\n\n{because}")
}

/// What `abeam +help` prints.
///
/// Deliberately short. abeam is a terminal user interface with one token of its
/// own; the keys are the interesting part and they are behind `F1`, where they
/// can be read next to the thing they act on. The agents are listed from the
/// table rather than written out, because a help text that can disagree with
/// the table eventually does.
///
/// Which now means the user's presets are in it, and that is the point of
/// listing rather than writing: `+help` on a machine with a config file answers
/// with what *that* machine can host. A preset is not marked out as one — it is
/// a name behind the sigil like any other, and a help text that sorted them
/// into two groups would be teaching a distinction the command line does not
/// have.
pub fn help(table: &[Agent]) -> String {
    let agents: Vec<&str> = table.iter().map(|a| a.name).collect();
    format!(
        "abeam - one window for an AI coding session.

Usage:
  abeam [args...]             host the default agent, and hand it the lot
  abeam +<agent> [args...]    host one of the agents below instead
  abeam +<program> [args...]  host any program on PATH
  abeam -- [args...]          ...when the first argument starts with `+`;
                              the `--` goes to the agent too

Agents: {}

abeam's own, and the only two words it reads (either case):
  +help     this
  +version  the version

Everything else on the command line belongs to the agent, `--help` and
`--version` included: `abeam --resume` resumes the default agent and `abeam
agent` sends it `agent`, exactly as the agent itself would have read them.

ABEAM_AGENT names the default agent — or program, or preset — to host. A `+`
overrides it. A preset is a `[preset.<name>]` block in abeam.toml under your
profile, and is listed above with everything else a `+` can name.

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
    /// same rule rather than an exception to it: `abeam +.\tools\agent.exe`
    /// puts `.\tools\agent.exe` on the border, not the absolute path it became,
    /// and certainly not the `cmd.exe` a Windows npm shim routes through. The
    /// sigil is not part of the name either — it is how abeam was told, not
    /// what it was told.
    ///
    /// One field, and it was briefly two, and it is two again — which is worth
    /// setting out rather than quietly reversing. While the launcher fallback
    /// existed the border had something to say that the line abeam prints on
    /// the way out did not — `copilot · via gh` — so `name` and `title` were
    /// separate, and that pair was deleted because with one way to start an
    /// agent the second field always held the same string as the first.
    ///
    /// [`Hosted::agent`] below is not that field coming back. It differs from
    /// this one exactly when a preset is hosting a built-in — `fleet` here and
    /// `claude` there — which is a case that could not arise until presets did,
    /// and the two answers go to two different readers rather than to the same
    /// one twice.
    pub name: String,
    /// What is actually taking the typing, by the name abeam knows it under.
    ///
    /// The same as [`name`](Self::name) for a built-in and for a program named
    /// outright; the host's name for a preset. Read by the parts of abeam whose
    /// question is *what agent is this* rather than *what do I call it* —
    /// `crate::dispatch` is the whole list today, and its question is whether
    /// `--bg` is a flag the hosted agent has.
    pub agent: String,
    pub launch: Launch,
}

impl Hosted {
    /// A program that was named outright, and so is its own explanation.
    ///
    /// Both names are the typed one. A program abeam was pointed at is not an
    /// agent abeam knows anything about, and answering `crate::dispatch` with
    /// the same word it was given is how it comes to say "abeam is hosting
    /// `pwsh`" rather than guessing on the user's behalf.
    pub fn plain(name: &str, launch: Launch) -> Self {
        Self {
            name: name.to_string(),
            agent: name.to_string(),
            launch,
        }
    }
}

/// Find this agent on the machine, or say what was looked for.
///
/// The table is handed in rather than read from [`AGENTS`], which is a seam
/// that was here for the tests before it was needed for anything else. Copilot
/// is not installed on the machine this was written on and cannot easily be —
/// its npm package wants Node 22 and this box has 20 — so the failure message
/// is only reachable at all with a table whose candidates are known to be
/// absent, and testing it against the real table would mean testing whichever
/// branch the machine happened to make reachable. It now carries the presets
/// too, which is what lets [`missing`] offer one as the alternative that *is*
/// installed.
pub fn resolve_within(agent: &Agent, args: &[String], table: &[Agent]) -> Result<Hosted, String> {
    // Only the last reason is kept, for the same reason `panes::shell::start`
    // keeps only the last: with a list, the earlier entries are the ones
    // expected to be missing, and leading with those is leading with the least
    // informative half of the answer.
    let mut why = String::new();

    // A preset's own arguments, and then the ones that were typed. Built once
    // and before the search, because `crate::launch` is not always handed a
    // program: a Windows npm shim is a `.cmd`, and the arguments for one are
    // quoted *into* the command line `cmd.exe` is pointed at — so a list
    // extended after resolution would be a list that never reached the child.
    let args = &line(agent, args);

    for candidate in agent.candidates {
        match launch::resolve(candidate, args) {
            // The agent's own name, not the absolute path it turned into and
            // not the `cmd.exe` a Windows npm shim routes through. Those are
            // facts about starting it, and the border has 46 columns for facts
            // about what is taking the typing.
            Ok(launch) => {
                return Ok(Hosted {
                    name: agent.name.to_string(),
                    agent: agent.hosts.to_string(),
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

/// The child's whole command line: what this table entry adds, then what was
/// typed.
///
/// A built-in adds nothing, so for `abeam --resume` this is the identity
/// function and the promise `crate::agentstate` relies on — that abeam puts no
/// argument of its own in front of anybody — is unchanged. It was never a
/// promise that *nothing* would be there; it was a promise that abeam would not
/// invent one. A preset's `args` were typed by the user in their own file,
/// which is the same standard as the rest of the line.
fn line(agent: &Agent, typed: &[String]) -> Vec<String> {
    if agent.args.is_empty() {
        return typed.to_vec();
    }
    agent
        .args
        .iter()
        .map(|arg| (*arg).to_string())
        .chain(typed.iter().cloned())
        .collect()
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
///
/// **And it is the answer to `abeam --help` on a machine with no agent on it**,
/// which is the case that put the last line here. Trace it: no `+`, so `--help`
/// is an argument; the default agent is `claude`; `claude` is not on `PATH`; and
/// what comes back is this. A reader who typed `--help` to find out what abeam
/// is has been handed installation advice for a program they may not have been
/// asking about, with no route at all to abeam's own command line — `F1` needs a
/// running agent, and the README needs them to go and find it. One line fixes
/// that, and it is one line rather than a special case in the parser: teaching
/// [`parse_with`] to recognise `--help` would put back the dashed-flag handling
/// the whole rule removed, and it would put it back for one spelling out of the
/// four while `-h` went on failing this way.
///
/// The flag that was typed is deliberately *not* echoed. This function is given
/// an agent and a search, and threading the command line in so that the message
/// could quote it would mean printing somebody's prompt back at them inside a
/// message about a missing program — `abeam -p "the API key is …"` is a line
/// people type. Naming `+help` is what the reader needs; the line they typed is
/// already on the screen above.
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
                "`{}` is installed; `abeam +{}` would host it.",
                other.name, other.name
            ));
        }
    }

    said.push(agent.install.to_string());

    // Last, and a line below the advice rather than part of it, because it
    // answers a different question: not "how do I get the agent" but "what is
    // this program I just typed at". Everybody who reaches this message has
    // just failed to start anything, and some of them were asking about abeam.
    said.push(String::new());
    said.push(
        "(`abeam +help` is abeam's own help. Everything else on the command \
         line, `--help` included, belongs to the agent.)"
            .to_string(),
    );
    said.join("\n")
}

fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Nothing here spawns a child. The programs the resolution tests reach for are
/// [`EVERY_MACHINE`] — the one name `PATH` is certain to find, whichever
/// platform this is — and names beginning `abeam-no-such-`, which are on none.
///
/// This module used to be `#[cfg(all(test, windows))]` entire, and its own
/// header conceded that the parsing half would run anywhere. It does, so it now
/// runs there: an argument parser has no filesystem in it at all, and the three
/// tests that do walk `PATH` differ between the platforms in one string — which
/// is why that string is a constant rather than these being two sets of tests
/// that would then be free to drift apart.
#[cfg(test)]
mod tests {
    use super::*;

    /// The one program every machine of this kind has, and the whole of the
    /// difference between running the resolution tests on Windows and on Linux.
    /// Nothing starts it: it is a name for `PATH` to find and a path to assert
    /// about afterwards.
    #[cfg(windows)]
    const EVERY_MACHINE: &str = "cmd.exe";
    #[cfg(unix)]
    const EVERY_MACHINE: &str = "sh";

    /// A program named as a path rather than as a name, which is the other
    /// thing `ABEAM_AGENT` may hold. Spelled for the platform, because what is
    /// asserted about it is that abeam hands it on untouched and an example
    /// nobody at this keyboard could type is a worse example than one they
    /// could. Never resolved, so it need not be on the machine.
    #[cfg(windows)]
    const A_PATH: &str = r"C:\tools\nu.exe";
    #[cfg(unix)]
    const A_PATH: &str = "/usr/local/bin/nu";

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
            args: &[],
            hosts: "abeam-test-one",
        },
        Agent {
            name: "abeam-test-two",
            candidates: &["abeam-no-such-agent-c"],
            install: "Install abeam-test-two from the test suite.",
            args: &[],
            hosts: "abeam-test-two",
        },
    ];

    /// The same, except that the second agent is one every machine has.
    const PRESENT: &[Agent] = &[
        Agent {
            name: "abeam-test-one",
            candidates: &["abeam-no-such-agent-a"],
            install: "Install abeam-test-one from the test suite.",
            args: &[],
            hosts: "abeam-test-one",
        },
        Agent {
            name: "abeam-test-two",
            candidates: &[EVERY_MACHINE],
            install: "Install abeam-test-two from the test suite.",
            args: &[],
            hosts: "abeam-test-two",
        },
    ];

    /// An agent that is installed, so that the found half is testable too.
    const DIRECT: Agent = Agent {
        name: "abeam-test-direct",
        candidates: &["abeam-no-such-agent-d", EVERY_MACHINE],
        install: "Install abeam-test-direct from the test suite.",
        args: &[],
        hosts: "abeam-test-direct",
    };

    /// The shape `crate::config` builds a preset into, written by hand so that
    /// this file can prove what the shape *does* without a config file in it:
    /// a name of its own, a built-in's candidates behind it, arguments in
    /// front, and `hosts` naming what is really being started.
    const PRESET: &[Agent] = &[Agent {
        name: "abeam-test-fleet",
        candidates: &["abeam-no-such-agent-d", EVERY_MACHINE],
        install: "Install abeam-test-direct from the test suite.",
        args: &["agent", "--fleet"],
        hosts: "abeam-test-direct",
    }];

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    /// The selection as one word — `agent:claude` or `program:powershell` — so
    /// that an assertion can say which of the two it wanted without a table in
    /// it, and the arguments that were left for the child.
    fn chose(argv: &[&str], default: Option<&str>) -> (String, Vec<String>) {
        chose_in(argv, default, AGENTS)
    }

    /// The same, against a table with something in it that abeam did not write.
    fn chose_in(
        argv: &[&str],
        default: Option<&str>,
        table: &'static [Agent],
    ) -> (String, Vec<String>) {
        match parse_with(&args(argv), default.map(String::from), table).expect("a command line") {
            Cli::Host {
                choice: Choice::Known(agent),
                args,
            } => (format!("agent:{}", agent.name), args),
            Cli::Host {
                choice: Choice::Program { name, .. },
                args,
            } => (format!("program:{name}"), args),
            other => panic!("expected something to host, got {other:?}"),
        }
    }

    // --- the table --------------------------------------------------------

    #[test]
    fn every_agent_abeam_knows_is_findable_and_the_default_is_one_of_them() {
        assert_eq!(find("claude").expect("claude is known").name, "claude");
        assert_eq!(find("copilot").expect("copilot is known").name, "copilot");
        // A name that is not in the table is not half-matched into one: it is a
        // program, and `abeam +powershell` has to keep meaning what `abeam
        // powershell` meant.
        assert!(find("powershell").is_none());
        assert!(find("claude-code").is_none());
        assert!(find("").is_none());

        // Case-insensitively, because this table is abeam's own and `abeam
        // +Claude` is the same request as `abeam +claude` on a filesystem that
        // folds case and on one that does not.
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
            // A built-in puts nothing of abeam's on the command line, which is
            // the promise `crate::agentstate` and `crate::dispatch` are written
            // on top of. `args` exists for presets, where the arguments were
            // typed by the user in their own file.
            assert!(
                agent.args.is_empty(),
                "{} adds arguments nobody asked for",
                agent.name
            );
            // ...and answers both name questions with the same word, so that a
            // preset is the only thing that can ever make them differ.
            assert_eq!(agent.hosts, agent.name);
        }

        // `gh copilot` is named in the hint and nowhere else in this file: it
        // is the route that works where the package managers do not — the npm
        // package wants Node 22 — and it is a command the reader runs, not one
        // abeam runs for them. It is the half of the sentence that does not
        // change with the platform, so it is asserted without a `cfg`.
        let copilot = find("copilot").unwrap().install;
        assert!(copilot.contains("gh copilot"), "got: {copilot}");

        // ...and the half that does. This is a message printed to somebody
        // whose agent is missing, so the command in it has to be one their
        // machine could run: `winget` is not on a Linux box, and a hint naming
        // it there would be worse than no hint at all.
        #[cfg(windows)]
        let installer = "winget install GitHub.Copilot";
        #[cfg(unix)]
        let installer = "npm i -g @github/copilot";
        assert!(copilot.contains(installer), "got: {copilot}");

        // Claude's sentence needs no such split: both of the routes it names
        // are spelled the same everywhere, which is why it is one string.
        let claude = find("claude").unwrap().install;
        assert!(claude.contains("npm i -g @anthropic-ai/claude-code"));
        assert!(!claude.contains("winget"), "got: {claude}");
    }

    // --- the flip ---------------------------------------------------------

    #[test]
    fn everything_on_the_command_line_belongs_to_the_agent() {
        // The change, in the lines somebody will actually type. Each of these
        // used to select a program, or be refused, or be answered by abeam;
        // all of them are the agent's now, in the order they were written.
        assert_eq!(
            chose(&["agent"], None),
            ("agent:claude".into(), args(&["agent"])),
            "`abeam agent` has to be `claude agent`"
        );
        assert_eq!(
            chose(&["--resume"], None),
            ("agent:claude".into(), args(&["--resume"]))
        );
        assert_eq!(
            chose(&["-p", "fix the tests"], None),
            ("agent:claude".into(), args(&["-p", "fix the tests"]))
        );
        assert_eq!(chose(&[], None), ("agent:claude".into(), vec![]));

        // Including the four spellings abeam used to answer itself, which is
        // the whole of what a reader loses here and the whole of what they
        // gain: that is Claude's help, and Claude's help is what they asked for.
        for flag in ["-h", "--help", "-V", "--version"] {
            assert_eq!(
                chose(&[flag], None),
                ("agent:claude".into(), args(&[flag])),
                "{flag} is the agent's"
            );
        }

        // Subcommands nobody could have enumerated in advance, which is the
        // argument for the rule rather than an example of it.
        assert_eq!(
            chose(&["mcp", "list"], None),
            ("agent:claude".into(), args(&["mcp", "list"]))
        );
        assert_eq!(
            chose(&["config", "set", "+x"], None),
            ("agent:claude".into(), args(&["config", "set", "+x"]))
        );

        // An empty argument is an ordinary thing to send a program, and used to
        // be refused as a name that named nothing. Only a bare `+` is that now.
        assert_eq!(chose(&[""], None), ("agent:claude".into(), args(&[""])));
        assert_eq!(
            chose(&["", "--resume"], None),
            ("agent:claude".into(), args(&["", "--resume"]))
        );

        // A lone dash, which used to be a program name with a sentence waiting
        // for it and is now a word like any other.
        assert_eq!(chose(&["-"], None), ("agent:claude".into(), args(&["-"])));
    }

    // --- the sigil --------------------------------------------------------

    #[test]
    fn a_leading_plus_token_selects_and_nothing_else_does() {
        // A name in the table picks that agent...
        assert_eq!(chose(&["+copilot"], None), ("agent:copilot".into(), vec![]));
        // ...and one that is not is a program, which is the whole of what the
        // positional used to do and all that moved.
        assert_eq!(chose(&["+pwsh"], None), ("program:pwsh".into(), vec![]));

        // The sigil is stripped: it is how abeam was told, not what it was told,
        // and the border shows the name that was typed behind it.
        assert_eq!(
            chose(&["+powershell", "-NoLogo", "-c", "gci"], None),
            ("program:powershell".into(), args(&["-NoLogo", "-c", "gci"]))
        );

        // Everything after it is the child's, `--help` included.
        assert_eq!(
            chose(&["+copilot", "--resume"], None),
            ("agent:copilot".into(), args(&["--resume"]))
        );
        assert_eq!(
            chose(&["+claude", "--help"], None),
            ("agent:claude".into(), args(&["--help"])),
            "`abeam +claude --help` is a question for Claude"
        );

        // Folded, like the table it is read against.
        assert_eq!(chose(&["+COPILOT"], None), ("agent:copilot".into(), vec![]));
        assert_eq!(chose(&["+Claude"], None), ("agent:claude".into(), vec![]));

        // ...and trimmed, which the table lookup itself is not: it folds case
        // and is otherwise exact, so before this `abeam "+claude "` fell
        // through to `PATH` and reported "`claude ` was not found on PATH" —
        // an agent abeam knows, demoted to a missing program by one character
        // nobody can see. Both ends, because one rule is easier to hold than
        // two and because `is_blank` above has always trimmed both.
        for typed in ["+claude ", "+ claude", "+  claude  ", "+\tclaude"] {
            assert_eq!(
                chose(&[typed], None),
                ("agent:claude".into(), vec![]),
                "`{typed}` is a request for claude"
            );
        }
        // The same for the words abeam answers itself, which are read off the
        // same trimmed string rather than off a second copy of it.
        assert!(matches!(
            parse_with(&args(&["+help "]), None, AGENTS),
            Ok(Cli::Help)
        ));
        // ...and for a program, so that the rule is about the token and not
        // about the table.
        assert_eq!(chose(&["+pwsh "], None), ("program:pwsh".into(), vec![]));

        // A dashed program name is still reachable, which is the capability the
        // old suite pinned and which the docs in this file briefly and wrongly
        // called impossible. Behind the sigil somebody has asked for it by
        // name; what stops `--weird-program` reaching a spawn is `launch::find`
        // failing to locate it, not this parser refusing to say it.
        assert_eq!(
            chose(&["+--weird-program"], None),
            ("program:--weird-program".into(), vec![])
        );
        assert_eq!(
            chose(&["+--help"], None),
            ("program:--help".into(), vec![]),
            "`abeam +--help` names a program called `--help`, and always did"
        );

        // A path is a program like any other here. What makes a *relative* one
        // mean the directory abeam was run in is `main::host`, which is the
        // only place that still knows which directory that was.
        let absolute = format!("{SIGIL}{A_PATH}");
        assert_eq!(
            chose(&[absolute.as_str()], None),
            (format!("program:{A_PATH}"), vec![])
        );
    }

    #[test]
    fn only_the_first_token_can_be_abeams() {
        // A prompt may begin with a `+`, and `config set +x` is a command line
        // somebody has. Anywhere but the front, a `+` is a character.
        assert_eq!(
            chose(&["+claude", "+copilot"], None),
            ("agent:claude".into(), args(&["+copilot"])),
            "the second sigil is Claude's argument, not a second selection"
        );
        assert_eq!(
            chose(&["-p", "+1 for the parser"], None),
            ("agent:claude".into(), args(&["-p", "+1 for the parser"]))
        );
        assert_eq!(
            chose(&["+pwsh", "+help"], None),
            ("program:pwsh".into(), args(&["+help"])),
            "a reserved word is only reserved in the one position abeam reads"
        );
        assert_eq!(
            chose(&["+1"], None),
            ("program:1".into(), vec![]),
            "...and in that position it is read, however unlike a program it looks"
        );
    }

    #[test]
    fn a_double_dash_fences_abeams_reading_and_is_then_handed_on_like_everything_else() {
        // The whole of what `--` does, in one assertion: abeam reads nothing
        // off this line, and the line — `--` and all — is the agent's. The
        // token is *not* consumed, which is the correction. `abeam -- --resume`
        // used to start `claude --resume` and resume the session, where `claude
        // -- --resume` sends the literal string `--resume` as a prompt: one
        // command line, two meanings, depending only on whether abeam was in
        // front of it.
        assert_eq!(
            chose(&["--", "--resume"], None),
            ("agent:claude".into(), args(&["--", "--resume"])),
            "the `--` reaches the child, or abeam is a second parser"
        );

        // The case it exists for: a first argument that genuinely begins with a
        // `+`. A shell will not do it for you — `"+1"` arrives here as a token
        // starting `+` exactly as the unquoted form does — and the fence still
        // works, because fencing was never the same act as being eaten.
        assert_eq!(
            chose(&["--", "+1", "more", "thing"], None),
            ("agent:claude".into(), args(&["--", "+1", "more", "thing"]))
        );
        assert_eq!(
            chose(&["--", "+claude"], None),
            ("agent:claude".into(), args(&["--", "+claude"])),
            "behind the fence a sigil is a character"
        );
        assert_eq!(
            chose(&["--", "+help"], None),
            ("agent:claude".into(), args(&["--", "+help"]))
        );

        // It fences abeam's reading of the line and it does not select: what
        // follows is arguments and never a program to host. `abeam -- pwsh`
        // sends `-- pwsh` to the default agent, which is the reverse of what
        // `--` used to do and is the point of it now.
        assert_eq!(
            chose(&["--", "pwsh"], None),
            ("agent:claude".into(), args(&["--", "pwsh"]))
        );
        // The documented escape from the refusal, and the consequence worth
        // stating out loud: it is `claude -- claude agent` now. What abeam owes
        // here is not an opinion about what Claude does with that — it is that
        // the answer is the same whether or not abeam is in front of the line.
        assert_eq!(
            chose(&["--", "claude", "agent"], Some("copilot")),
            ("agent:copilot".into(), args(&["--", "claude", "agent"]))
        );

        // Every `--` is the child's, including the first. There is no
        // arithmetic to remember about how many abeam eats, because it eats
        // none.
        assert_eq!(
            chose(&["--", "--", "-x"], None),
            ("agent:claude".into(), args(&["--", "--", "-x"]))
        );
        assert_eq!(
            chose(&["-p", "--"], None),
            ("agent:claude".into(), args(&["-p", "--"]))
        );

        // On its own it is a one-token command line for the agent, which is
        // what `claude --` is.
        assert_eq!(chose(&["--"], None), ("agent:claude".into(), args(&["--"])));
    }

    // --- abeam's own two words --------------------------------------------

    #[test]
    fn the_two_reserved_words_are_answered_rather_than_hosted() {
        for word in ["+help", "+HELP", "+Help"] {
            assert!(
                matches!(parse_with(&args(&[word]), None, AGENTS), Ok(Cli::Help)),
                "{word} is abeam's help"
            );
        }
        for word in ["+version", "+VERSION", "+Version"] {
            assert!(
                matches!(parse_with(&args(&[word]), None, AGENTS), Ok(Cli::Version)),
                "{word} is abeam's version"
            );
        }

        // Answered before the default is looked at, so a default nothing can
        // start cannot swallow the question.
        assert!(matches!(
            parse_with(
                &args(&["+help", "extra"]),
                Some("abeam-no-such-agent".into()),
                AGENTS
            ),
            Ok(Cli::Help)
        ));

        // ...and the list `crate::config` refuses preset names against is the
        // same two words, which is what stops a `[preset.help]` being accepted
        // into a table that can never be reached for it.
        assert_eq!(RESERVED, &["help", "version"]);
        for word in RESERVED {
            assert!(
                !matches!(
                    parse_with(&args(&[&format!("{SIGIL}{word}")]), None, AGENTS),
                    Ok(Cli::Host { .. })
                ),
                "`+{word}` is answered rather than hosted"
            );
        }

        // Two words, and the set stays two. `+h` and `+V` are not short forms
        // of these: a short form is one more name that can never be a program,
        // bought with two keystrokes of a command nobody types twice.
        assert_eq!(chose(&["+h"], None), ("program:h".into(), vec![]));
        assert_eq!(chose(&["+V"], None), ("program:V".into(), vec![]));
        // Nor is a word that merely begins with one.
        assert_eq!(chose(&["+helper"], None), ("program:helper".into(), vec![]));

        // The dashed spellings are the agent's now, which is the whole change,
        // and this is the assertion that fails if anyone puts them back.
        for flag in ["-h", "--help", "-V", "--version"] {
            assert!(
                matches!(
                    parse_with(&args(&[flag]), None, AGENTS),
                    Ok(Cli::Host { .. })
                ),
                "{flag} belongs to the agent"
            );
        }
    }

    #[test]
    fn a_sigil_with_nothing_behind_it_is_a_sentence_rather_than_the_default() {
        // The other half of the blank rule, going the other way on purpose. An
        // unset variable is a default that should not be there; a typed `+` is
        // a request, and answering a request for nothing by starting the
        // default agent hands the arguments after it to a program nobody named.
        for typed in ["+", "+ ", "+   ", "+\t"] {
            let refused = parse_with(&args(&[typed]), None, AGENTS)
                .expect_err("a bare sigil is not a program");
            // The diagnosis and not merely the character. `contains('+')` is
            // satisfied by almost any sentence this file can produce — every
            // message here has a `+` in it somewhere — so it pinned nothing;
            // the assertion it replaced said `contains("empty")` and did. This
            // is that strength restored against the wording that is actually
            // there.
            assert!(
                refused.contains("with no name behind it"),
                "the refusal has to name what is wrong: {refused}"
            );
            // What used to come out, and what a reader could do nothing with.
            assert!(
                !refused.contains("was not found on PATH"),
                "the old non-answer is back: {refused}"
            );
        }

        // Especially with arguments after it, which is where defaulting would
        // do real damage.
        assert!(parse_with(&args(&["+", "--resume"]), None, AGENTS).is_err());
        // A default that is set makes no difference — the token is what was
        // asked for, and it asked for nothing.
        assert!(parse_with(&args(&["+"]), Some("copilot".into()), AGENTS).is_err());

        // Behind the fence, and anywhere but the front, it is an argument like
        // any other: there is no sigil there to be empty.
        assert_eq!(
            chose(&["--", "+"], None),
            ("agent:claude".into(), args(&["--", "+"]))
        );
        assert_eq!(
            chose(&["-p", "+"], None),
            ("agent:claude".into(), args(&["-p", "+"]))
        );
    }

    // --- the refusal that keeps the old command line honest ----------------

    #[test]
    fn a_first_word_that_used_to_select_is_refused_rather_than_quietly_reread() {
        // Every name in the table, because this refusal has to grow with it:
        // the hazard is precisely a name abeam itself once treated as a
        // selection, and a new agent brings a new one.
        for agent in AGENTS {
            let refused = parse_with(&args(&[agent.name, "agent"]), None, AGENTS)
                .expect_err("a name that used to select is not silently rewritten");

            // Both readings, named. Somebody has just typed a line that was
            // right for years, and without this they would be reading the
            // agent's complaint about an argument on a screen that never
            // mentions abeam.
            assert!(refused.contains("used to host"), "got: {refused}");
            assert!(refused.contains(agent.name), "got: {refused}");

            // ...and both ways out, each of them this line with its first token
            // changed. They are asserted as whole phrases because either half
            // alone would pass by accident.
            assert!(
                refused.contains(&format!("`abeam +{}`", agent.name)),
                "the way to host it is missing from: {refused}"
            );
            assert!(
                refused.contains(&format!("`abeam -- {}`", agent.name)),
                "the way to mean the word is missing from: {refused}"
            );

            // Case-insensitively, because the table is: somebody typing `abeam
            // Claude` is the same person making the same mistake.
            let shouted = agent.name.to_uppercase();
            assert!(parse_with(&args(&[shouted.as_str()]), None, AGENTS).is_err());
            // With nothing after it, which is the commonest way to arrive here
            // — it is what the old README said to type.
            assert!(parse_with(&args(&[agent.name]), None, AGENTS).is_err());
            // And whatever the default is. This is a fixed lookup on the token
            // rather than a question about what would otherwise have run.
            assert!(parse_with(&args(&[agent.name]), Some("pwsh".into()), AGENTS).is_err());
        }

        // Only the table, and never `PATH`. A word that is not in it is an
        // argument like any other however much it looks like a program, which
        // is what makes this answer the same on every machine — a refusal that
        // depended on what was installed would accept a command line here and
        // reject it on a build server.
        assert_eq!(
            chose(&["claude-code"], None),
            ("agent:claude".into(), args(&["claude-code"]))
        );
        assert_eq!(
            chose(&[EVERY_MACHINE], None),
            ("agent:claude".into(), args(&[EVERY_MACHINE])),
            "a program that is certainly installed is still just a word here"
        );

        // Behind the sigil it is a selection again, and behind the fence it is
        // a word. Those are the two escapes the message names, and they have to
        // do what it says they do.
        assert_eq!(
            chose(&["+claude", "agent"], None),
            ("agent:claude".into(), args(&["agent"]))
        );
        assert_eq!(
            chose(&["--", "claude", "agent"], None),
            ("agent:claude".into(), args(&["--", "claude", "agent"]))
        );
    }

    #[test]
    fn the_refusal_names_a_rewrite_that_can_be_typed_rather_than_the_line_it_was_given() {
        // The line the old message could not survive. abeam is handed argv, so
        // the shell's quotes are gone by the time this file sees anything —
        // joining the words back together with spaces produced `Write `abeam
        // +claude -p fix the tests & ship it``, which in a real shell is two
        // commands and in PowerShell is a parse error. The instruction said
        // "Write", so it was an instruction to type something broken.
        let refused = parse_with(
            &args(&["claude", "-p", "fix the tests & ship it"]),
            None,
            AGENTS,
        )
        .expect_err("a name that used to select is refused whatever follows it");

        // The rewrite is exact, and it is exact because it is small: one token.
        assert!(refused.contains("`abeam +claude`"), "got: {refused}");
        assert!(refused.contains("`abeam -- claude`"), "got: {refused}");
        // ...and the rest of the line is described rather than quoted, so there
        // is nothing in the message that a shell would read differently from
        // the way it was typed.
        assert!(
            !refused.contains("fix the tests"),
            "the line is echoed back unquoted: {refused}"
        );
        assert!(
            !refused.contains(" & "),
            "a shell metacharacter is loose in the message: {refused}"
        );
        // The rest of the line is still accounted for, or a reader would think
        // they had to retype it.
        assert!(
            refused.contains("leave the rest of the line"),
            "got: {refused}"
        );
    }

    #[test]
    fn the_refusal_is_true_of_a_preset_as_well_as_of_a_built_in() {
        // Two pasts, and one sentence for both was false for one of them.
        // `abeam claude` really did host Claude for the whole of abeam's life
        // before the flip.
        let built = parse_with(&args(&["claude"]), None, AGENTS).expect_err("a built-in's name");
        assert!(built.contains("used to host claude"), "got: {built}");

        // `abeam abeam-test-fleet` never hosted anybody's preset: a preset is
        // selected behind the sigil and only there, and presets postdate the
        // flip entirely. What it *did* mean is a `PATH` lookup for a program of
        // that name, which is what the message now says.
        let preset =
            parse_with(&args(&["abeam-test-fleet"]), None, PRESET).expect_err("a preset's name");
        assert!(
            !preset.contains("used to host abeam-test-fleet"),
            "the preset is told a thing that never happened: {preset}"
        );
        assert!(
            preset.contains("never your `abeam-test-fleet` preset"),
            "got: {preset}"
        );
        assert!(preset.contains("on PATH"), "got: {preset}");
        // Both ways out are still there, spelled for the name that was typed.
        assert!(
            preset.contains("`abeam +abeam-test-fleet`"),
            "got: {preset}"
        );
        assert!(
            preset.contains("`abeam -- abeam-test-fleet`"),
            "got: {preset}"
        );
    }

    // --- the default ------------------------------------------------------

    #[test]
    fn abeam_agent_names_what_to_host_and_a_plus_token_overrides_it() {
        // It may name an agent...
        assert_eq!(
            chose(&[], Some("copilot")),
            ("agent:copilot".into(), vec![])
        );
        // ...or any program, exactly as ABEAM_SHELL may — including one named
        // as a path, which is passed on as it was written.
        assert_eq!(
            chose(&[], Some(A_PATH)),
            (format!("program:{A_PATH}"), vec![])
        );

        // The sentence this change bought. A default that applies to a command
        // line with arguments on it is a default worth setting; before this,
        // `--resume` was an unrecognised abeam flag and naming Copilot meant a
        // positional that then shadowed everything after it.
        assert_eq!(
            chose(&["--resume"], Some("copilot")),
            ("agent:copilot".into(), args(&["--resume"]))
        );
        assert_eq!(
            chose(&["-p", "fix the tests"], Some(A_PATH)),
            (format!("program:{A_PATH}"), args(&["-p", "fix the tests"]))
        );

        // Overridden by a `+` token, whichever kind each of them is.
        assert_eq!(
            chose(&["+claude"], Some("copilot")),
            ("agent:claude".into(), vec![])
        );
        assert_eq!(
            chose(&["+pwsh"], Some("copilot")),
            ("program:pwsh".into(), vec![])
        );
        assert_eq!(
            chose(&["+copilot", "--resume"], Some("claude")),
            ("agent:copilot".into(), args(&["--resume"]))
        );

        // PowerShell leaves an emptied variable behind as an empty string, and
        // "`` was not found on PATH" names nothing anyone can act on. Blank
        // rather than empty, because `$env:ABEAM_AGENT = " "` is the same
        // non-answer and used to produce "`   ` was not found on PATH".
        assert_eq!(chose(&[], Some("")), ("agent:claude".into(), vec![]));
        assert_eq!(chose(&[], Some("   ")), ("agent:claude".into(), vec![]));
        assert_eq!(chose(&[], Some("\t")), ("agent:claude".into(), vec![]));
        // ...and it is unset with arguments on the line too, rather than
        // swallowing them into a program nobody named.
        assert_eq!(
            chose(&["--resume"], Some("")),
            ("agent:claude".into(), args(&["--resume"]))
        );

        // Trimmed, like a name behind the sigil: a variable is usually set by a
        // script, and a trailing space in an `export` line is invisible in
        // every editor there is.
        assert_eq!(
            chose(&[], Some("copilot ")),
            ("agent:copilot".into(), vec![])
        );

        // A dashed program is nameable here with no `+` anywhere on the line,
        // which is the second of the three spellings that refute the "a dashed
        // token can never be a program name" claim this file used to make.
        assert_eq!(
            chose(&[], Some("--help")),
            ("program:--help".into(), vec![]),
            "`ABEAM_AGENT=--help` names a program called `--help`"
        );
    }

    #[test]
    fn a_sigil_in_the_variable_is_the_documented_mistake_and_is_named_rather_than_stripped() {
        // `+copilot` is the spelling every other page of abeam's documentation
        // teaches, so it is the value somebody will put in this variable, and
        // what it used to produce was "`+copilot` was not found on PATH" — true,
        // useless, and dependent on whether some machine happens to have a file
        // of that name.
        let refused = parse_with(&[], Some("+copilot".into()), AGENTS)
            .expect_err("the variable holds a name, not a command line");
        assert!(refused.contains("ABEAM_AGENT"), "got: {refused}");
        assert!(refused.contains("`+copilot`"), "got: {refused}");
        // With the correction shown rather than described: the fix is one
        // character and the fastest way to say so is to print both spellings.
        assert!(refused.contains("`copilot`"), "got: {refused}");
        assert!(
            !refused.contains("was not found on PATH"),
            "the old non-answer is back: {refused}"
        );

        // Whatever is behind it, because this is about the shape of the value
        // and not about whether abeam knows the name.
        assert!(parse_with(&[], Some("+pwsh".into()), AGENTS).is_err());
        assert!(parse_with(&[], Some(" +fleet ".into()), AGENTS).is_err());
        // ...and with arguments on the line, which is now every invocation the
        // variable reaches rather than bare `abeam` alone.
        assert!(parse_with(&args(&["--resume"]), Some("+copilot".into()), AGENTS).is_err());

        // Only ever a value abeam was about to use. A `+` token overrides the
        // variable before `hosting` is reached at all, so a stale one cannot
        // refuse a command line that had already said what to host.
        assert_eq!(
            chose(&["+claude"], Some("+copilot")),
            ("agent:claude".into(), vec![]),
            "a variable abeam never consults cannot refuse anything"
        );

        // And the escape, which costs nothing and is worth knowing: a program
        // genuinely called `+copilot` is still hostable, because the sigil the
        // parser strips is the first one.
        assert_eq!(
            chose(&["++copilot"], None),
            ("program:+copilot".into(), vec![])
        );
    }

    #[test]
    fn the_help_names_every_agent_the_table_knows() {
        let help = help(AGENTS);

        // This used to loop over `AGENTS` asserting `help.contains(name)`, and
        // it could not fail: `help` *builds* that line by mapping over the same
        // table, so the loop asserted that a `join` had joined. It is replaced
        // rather than deleted, because the thing it was reaching for is worth
        // pinning — it just has to be pinned against something the function
        // does not derive from the assertion's own input.
        //
        // Two halves do that. The exact line, which fails if the separator, the
        // order or the label changes and if a name is dropped from `AGENTS`...
        assert!(
            help.contains("Agents: claude, copilot"),
            "the agents line is not what a reader was promised: {help}"
        );
        // ...and the same function over a table that is *not* `AGENTS`, which
        // is the only assertion that can tell "listed from the table it was
        // given" apart from "written out and happening to match". Both are
        // needed: the first alone passes for a hardcoded line.
        let elsewhere = super::help(ABSENT);
        assert!(
            elsewhere.contains("Agents: abeam-test-one, abeam-test-two"),
            "the help does not list the table it was handed: {elsewhere}"
        );
        assert!(
            !elsewhere.contains("claude"),
            "the help has `claude` written into it somewhere: {elsewhere}"
        );

        // Including the rows abeam did not write. `+help` is what somebody
        // types when they have forgotten what they called a preset, and a help
        // text listing only the built-ins would be answering a question nobody
        // asked on a machine that has a config file.
        assert!(
            super::help(PRESET).contains("abeam-test-fleet"),
            "a preset is missing from +help"
        );

        assert!(help.contains("ABEAM_AGENT"), "the override has to be in it");
        assert!(help.contains("F1"), "the keys live behind F1, not here");
        // abeam's own two words, in the spelling that reaches them.
        assert!(help.contains("+help") && help.contains("+version"));

        // A help text still offering `--help` as *abeam's* would be documenting
        // the agent's flag as abeam's, which is the exact confusion this rule
        // removed — and the assertion above cannot see that, because it only
        // checks the sigil forms are present and a re-added `-h, --help` row
        // would sit happily beside them. What the shape of a re-added row has
        // in common, whatever it says, is that it begins with a dash: every
        // line in this text either starts with `abeam`, starts with `+`, or is
        // prose. `--help` and `--version` are named in that prose, deliberately
        // and as the *agent's*, so their mere presence proves nothing either
        // way.
        for line in help.lines() {
            assert!(
                !line.trim_start().starts_with('-'),
                "the help offers a dashed flag as abeam's own: {line}"
            );
        }

        // Short, because this is a terminal user interface and not a CLI with
        // fifty options. A page that scrolls is a page nobody reads.
        assert!(help.lines().count() < 30, "the help has grown a scrollbar");

        assert!(version().contains(env!("CARGO_PKG_VERSION")));
    }

    // --- finding it -------------------------------------------------------

    #[test]
    fn an_agent_that_is_installed_is_named_by_its_own_name_and_not_by_its_path() {
        let hosted = resolve_within(&DIRECT, &args(&["--resume"]), PRESENT).expect(EVERY_MACHINE);

        assert_eq!(hosted.name, "abeam-test-direct");
        // Which is worth asserting only because the thing it is *not* is right
        // here: an absolute path is what gets started, and what a 46-column
        // border must never show.
        assert!(hosted.launch.program.is_absolute());
        assert!(hosted.launch.program.ends_with(EVERY_MACHINE));
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
    fn the_message_for_a_missing_agent_is_also_the_answer_to_abeam_dash_dash_help() {
        // Traced, and it is not a hypothetical: `abeam --help` on a machine
        // with no agent installed has no `+` on it, so `--help` is an argument,
        // the default is `claude`, `claude` is not on `PATH`, and this is the
        // whole of what comes back. Somebody asking what abeam is gets install
        // advice for a program they may never have named, and — before this
        // line — no route at all to abeam's own command line. `F1` needs a
        // running agent, so the README was the only way left to find out.
        let refused = resolve_within(&ABSENT[0], &args(&["--help"]), ABSENT).expect_err("nothing");

        assert!(refused.contains("`abeam +help`"), "got: {refused}");
        // ...and why the thing they typed did not answer, which is the half
        // that stops them typing it again.
        assert!(refused.contains("belongs to the agent"), "got: {refused}");

        // The install sentence is still the last piece of *advice*, because it
        // is the one that fixes the problem the reader most likely has.
        let lines: Vec<&str> = refused.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(
            lines[lines.len() - 2],
            ABSENT[0].install,
            "the sentence that fixes it has been pushed off the end: {refused}"
        );
    }

    #[test]
    fn a_program_that_is_nowhere_says_which_of_the_two_ways_in_named_it() {
        // The worst message in this file, before there was a second arm to
        // this function: `abeam "+1 to shipping this"` printed "`1 to shipping
        // this` was not found on PATH" and stopped. No mention of the sigil,
        // none of abeam, and none of the escape — for a line whose likeliest
        // reading is a prompt that happened to begin with a `+`, which is
        // exactly the shape first position collects.
        let typed = nowhere(
            "1 to shipping this",
            Whence::Sigil,
            "`1 to shipping this` was not found on PATH.",
        );
        assert!(typed.contains("not found on PATH"), "got: {typed}");
        assert!(typed.contains('+'), "the sigil is unmentioned: {typed}");
        assert!(
            typed.contains("`abeam -- …`"),
            "the escape is unmentioned: {typed}"
        );
        // Deterministic: the same sentence for a word that looks nothing like
        // prose, because a parser that guessed which was which would give an
        // answer that depended on the words.
        let word = nowhere("pwsh", Whence::Sigil, "`pwsh` was not found on PATH.");
        assert!(word.contains("`abeam -- …`"), "got: {word}");

        // The other way in, and the sentence it needs is a different one: this
        // reader set something once and has probably forgotten, and nothing
        // else on the screen would say the variable is why.
        let stale = nowhere("copilo", Whence::Default, "`copilo` was not found on PATH.");
        assert!(stale.contains("ABEAM_AGENT"), "got: {stale}");
        assert!(stale.contains("`abeam +<agent>`"), "got: {stale}");
        // ...including the thing about it that changed under this rule, which
        // is what makes a stale value worth checking at all.
        assert!(stale.contains("every command line"), "got: {stale}");
        assert!(
            !stale.contains("abeam --"),
            "the sigil reader's escape is not this reader's answer: {stale}"
        );
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
        // With the sigil, because the sentence has to be the command to type
        // and `abeam abeam-test-two` is no longer that command — it would send
        // the name to the agent that is missing.
        assert!(
            refused.contains("`abeam +abeam-test-two` would host it"),
            "the sentence has to be the command to type: {refused}"
        );
    }

    // --- a row somebody else wrote ----------------------------------------

    #[test]
    fn a_preset_is_a_row_of_the_table_and_every_rule_here_applies_to_it() {
        // Selected behind the sigil, folded like the built-ins are, and
        // reachable through the default variable — none of which has a line of
        // its own anywhere: `Choice::of` reads the table it is given and does
        // not care who wrote which row.
        assert_eq!(
            chose_in(&["+abeam-test-fleet"], None, PRESET),
            ("agent:abeam-test-fleet".into(), vec![])
        );
        assert_eq!(
            chose_in(&["+ABEAM-TEST-FLEET"], None, PRESET),
            ("agent:abeam-test-fleet".into(), vec![])
        );
        assert_eq!(
            chose_in(&["--resume"], Some("abeam-test-fleet"), PRESET),
            ("agent:abeam-test-fleet".into(), args(&["--resume"])),
            "`ABEAM_AGENT=<preset>` needed no code and has to keep needing none"
        );

        // And refused in front of the sigil, which is the sentence in the
        // module docs made true: a name that selects behind a `+` is a name
        // somebody will type without one.
        let refused = parse_with(&args(&["abeam-test-fleet", "agent"]), None, PRESET)
            .expect_err("a preset name is a selection, not an argument");
        assert!(refused.contains("used to host"), "got: {refused}");
        // The first token and not the line: `agent` is left where it was typed,
        // and the message says so rather than echoing it back through a `join`
        // that has lost the shell's quoting.
        assert!(
            refused.contains("`abeam +abeam-test-fleet`"),
            "got: {refused}"
        );
        assert!(!refused.contains("fleet agent`"), "got: {refused}");

        // A name that is *not* in the table it was given is still a word, which
        // is what stops this refusal growing into a `PATH` probe by the back
        // door: the same token against the built-in table is an argument.
        assert_eq!(
            chose(&["abeam-test-fleet"], None),
            ("agent:claude".into(), args(&["abeam-test-fleet"]))
        );
    }

    #[test]
    fn a_presets_own_arguments_go_in_front_of_the_ones_that_were_typed() {
        let hosted = resolve_within(&PRESET[0], &args(&["--resume"]), PRESET).expect(EVERY_MACHINE);

        // The whole of the ordering rule, in the line somebody types: `abeam
        // +fleet --resume` is `claude agent --resume` and never `claude
        // --resume agent`, because a subcommand is the first word of the line
        // it belongs to.
        assert_eq!(hosted.launch.args, args(&["agent", "--fleet", "--resume"]));
        // With nothing typed, the preset's own line is the whole line.
        assert_eq!(
            resolve_within(&PRESET[0], &[], PRESET).unwrap().launch.args,
            args(&["agent", "--fleet"])
        );

        // Two names, and they differ here in the one way that matters: the
        // border says what was asked for, and `crate::dispatch` is told what is
        // actually running — otherwise a preset hosting Claude would quietly
        // lose the queue's dispatch mode.
        assert_eq!(hosted.name, "abeam-test-fleet");
        assert_eq!(hosted.agent, "abeam-test-direct");

        // A built-in adds nothing and answers both questions with one word,
        // which is the promise `crate::agentstate` reads this file for.
        let plain = resolve_within(&DIRECT, &args(&["--resume"]), PRESENT).expect(EVERY_MACHINE);
        assert_eq!(plain.launch.args, args(&["--resume"]));
        assert_eq!(plain.name, plain.agent);
    }

    #[test]
    fn a_preset_hosts_name_is_looked_up_in_the_built_in_table_and_not_the_whole_one() {
        // The rule that makes preset chaining impossible rather than merely
        // absent: `find` is the built-in lookup, `crate::config` resolves a
        // host through it, and a preset is therefore never something a preset
        // can name. There is no cycle to check for because there is no edge.
        assert!(find("abeam-test-fleet").is_none());
        assert!(find_within("abeam-test-fleet", PRESET).is_some());

        // ...and the built-ins are in both, because they are what a host may
        // name.
        assert_eq!(find("claude").unwrap().name, "claude");
        assert_eq!(find_within("claude", AGENTS).unwrap().name, "claude");
    }
}
