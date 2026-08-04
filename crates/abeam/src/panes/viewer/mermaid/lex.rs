//! The part of mermaid's surface syntax that every diagram type shares:
//! comments, directives, statement separators, and what a label says once the
//! quoting and the HTML have been taken back off it.
//!
//! Shared rather than repeated, because a flowchart and a sequence diagram
//! disagreeing about whether `%%` starts a comment would be two bugs wearing
//! one name.

use std::borrow::Cow;

/// Bracket pairs that may hold a `;` or a `%%` without meaning either.
/// `<` and `>` are absent on purpose: they are arrow syntax far more often
/// than they are a bracket.
const PAIRS: [(char, char); 3] = [('[', ']'), ('(', ')'), ('{', '}')];

/// Every line worth parsing, in order, with comments and directives gone and
/// the whitespace trimmed.
///
/// Blank lines are dropped rather than kept as separators. No diagram type here
/// gives a blank line meaning, and keeping them would put an `is_empty` check
/// at the top of every parse loop below.
pub fn meaningful_lines(source: &str) -> Vec<String> {
    source
        .lines()
        .map(strip_comment)
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Everything before an unquoted `%%`.
///
/// This covers mermaid's `%%{init: {...} }%%` directives as well, and by the
/// same rule rather than by a second one: a directive is a comment whose text
/// mermaid happens to read. abeam has no theme to take from it — the pane owns
/// its palette (see `viewer::theme`) — so the whole line goes.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quoted = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => quoted = !quoted,
            b'%' if !quoted && bytes.get(i + 1) == Some(&b'%') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Split a line on `;`, which mermaid allows as a statement separator, without
/// splitting one that is inside a label.
///
/// `graph TD; A-->B; B-->C` is a whole diagram on one line and is written that
/// way often enough to be worth handling. `A[step one; then two]` is one node
/// and must not become two statements, which is the entire reason this is not
/// `line.split(';')`.
pub fn split_statements(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut depth = 0usize;

    for (i, ch) in line.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            _ if quoted => {}
            _ if PAIRS.iter().any(|(open, _)| *open == ch) => depth += 1,
            _ if PAIRS.iter().any(|(_, close)| *close == ch) => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                push_trimmed(&mut out, &line[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    push_trimmed(&mut out, &line[start..]);
    out
}

fn push_trimmed(out: &mut Vec<String>, piece: &str) {
    let piece = piece.trim();
    if !piece.is_empty() {
        out.push(piece.to_string());
    }
}

/// What a label actually says: quotes off, `<br>` turned into the line break it
/// means, entities decoded, and every remaining run of whitespace collapsed.
///
/// The line break survives as `\n` rather than becoming a space, because a
/// two-line node label is a thing an author asked for and a box has room for.
/// Whoever draws it decides how many lines to honour.
pub fn label(raw: &str) -> String {
    let raw = raw.trim();
    // Mermaid's own quoting: `A["text with ] in it"]`. The quotes are syntax
    // and never part of what the node says.
    let raw = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(raw);

    let broken = break_tags(raw);
    let decoded = entities(&broken);

    // Collapse per line, not across them: collapsing across would undo the
    // break that was just decoded.
    decoded
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `<br>`, `<br/>` and `<br />` in any case, to `\n`. Nothing else HTML is
/// touched: a label containing `<div>` is a label containing `<div>`, and
/// inventing a rendering for arbitrary markup is how a diagram starts lying.
fn break_tags(raw: &str) -> Cow<'_, str> {
    if !raw.contains('<') {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('<') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        match from.find('>') {
            Some(end) => {
                let tag = &from[1..end];
                if tag.trim().trim_end_matches('/').trim().eq_ignore_ascii_case("br") {
                    out.push('\n');
                } else {
                    out.push_str(&from[..=end]);
                }
                rest = &from[end + 1..];
            }
            None => {
                out.push_str(from);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The handful of entities mermaid's own docs tell people to use for characters
/// its parser would otherwise eat, plus the three every author types anyway.
///
/// Deliberately not a general HTML entity table: this is a terminal drawing a
/// node label, and the long tail of named entities is a dependency's worth of
/// table to decode text that would still have to fit in a box.
fn entities(raw: &str) -> Cow<'_, str> {
    const KNOWN: [(&str, &str); 7] = [
        ("#quot;", "\""),
        ("#35;", "#"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&amp;", "&"),
    ];
    if !raw.contains('&') && !raw.contains('#') {
        return Cow::Borrowed(raw);
    }
    let mut out = raw.to_string();
    for (from, to) in KNOWN {
        if out.contains(from) {
            out = out.replace(from, to);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_directives_are_dropped_whole() {
        let src = "%%{init: {'theme':'dark'}}%%\ngraph TD\n%% a note\n  A --> B  %% trailing\n";
        assert_eq!(meaningful_lines(src), ["graph TD", "A --> B"]);
    }

    #[test]
    fn a_percent_inside_a_label_is_not_a_comment() {
        // The one input that makes the quote tracking load-bearing rather than
        // decorative: a node whose text is a percentage.
        let src = "graph TD\n  A[\"100%% done\"] --> B\n";
        assert_eq!(meaningful_lines(src), ["graph TD", "A[\"100%% done\"] --> B"]);
    }

    #[test]
    fn statements_split_on_semicolons_outside_labels() {
        assert_eq!(
            split_statements("graph TD; A-->B; B-->C"),
            ["graph TD", "A-->B", "B-->C"]
        );
        assert_eq!(
            split_statements("A[step one; then two] --> B"),
            ["A[step one; then two] --> B"]
        );
        assert_eq!(split_statements("A --> B;"), ["A --> B"]);
    }

    #[test]
    fn an_unbalanced_bracket_still_terminates_and_keeps_the_text() {
        // Half-written mermaid is the normal case: the agent is still typing
        // and the watcher has already shown us the file.
        assert_eq!(split_statements("A[unclosed --> B"), ["A[unclosed --> B"]);
        assert_eq!(split_statements("A]]] ; B"), ["A]]]", "B"]);
    }

    #[test]
    fn a_label_loses_its_quotes_and_keeps_its_break() {
        assert_eq!(label("\"two   words\""), "two words");
        assert_eq!(label("first<br/>second"), "first\nsecond");
        assert_eq!(label("first<BR>second"), "first\nsecond");
        assert_eq!(label("first<br />second"), "first\nsecond");
    }

    #[test]
    fn markup_that_is_not_a_break_is_left_alone_rather_than_guessed_at() {
        assert_eq!(label("a <b>bold</b> claim"), "a <b>bold</b> claim");
        assert_eq!(label("less < than"), "less < than");
    }

    #[test]
    fn the_entities_mermaid_tells_people_to_type_are_decoded() {
        assert_eq!(label("#quot;quoted#quot;"), "\"quoted\"");
        assert_eq!(label("a &amp; b &lt; c"), "a & b < c");
        assert_eq!(label("issue #35;12"), "issue #12");
    }

    #[test]
    fn nothing_here_panics_on_multibyte_input() {
        // `char_indices` over a label of ideographs, split on a byte index.
        assert_eq!(split_statements("日本[語版]; 次"), ["日本[語版]", "次"]);
        assert_eq!(label("  日本語  "), "日本語");
        assert!(meaningful_lines("graph TD\n  A[\"→ ✔\"] --> B").len() == 2);
    }
}
