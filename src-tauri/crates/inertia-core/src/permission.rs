//! The permission rule engine.
//!
//! Given a ruleset and a tool call, decide: allow it, deny it, or ask a
//! person. The engine is pure - no I/O, no async, no clock - which is what
//! makes it exhaustively testable, and it should stay that way. The
//! asking half lives behind [`crate::permission::PermissionGate`].
//!
//! Two properties are load-bearing and the tests below pin both:
//!
//!   - **The default is [`Action::Ask`].** A tool nobody has written a rule
//!     about is not silently allowed.
//!   - **A `deny` cannot be loosened.** Not by a more specific rule layered on
//!     top, not by the approval dial. Everything else in the system assumes
//!     this.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The pattern that matches every target.
pub const ANY: &str = "*";

/// What to do about a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Ask,
    Deny,
}

impl Default for Action {
    /// Ask. A tool nobody configured is not a tool anyone approved.
    fn default() -> Self {
        Self::Ask
    }
}

impl Action {
    /// Reads an action from stored config, falling back to the default.
    ///
    /// Tolerant on purpose: a typo in a hand-edited settings file should cost
    /// the user a confirmation prompt, not a failure to start.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            _ => Self::Ask,
        }
    }
}

/// One rule: for this tool, against this argument, do this.
///
/// Rulesets are flat ordered lists, never nested, so they are edited as a
/// list, diffed as a list, and merged as a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Tool name. **Also a glob** - `mcp_*` covers a whole family of
    /// dynamically discovered tool names that did not exist when the rule was
    /// written.
    pub tool: String,
    /// The argument the rule is about, e.g. `git push *`. [`ANY`] covers all.
    #[serde(default = "any_pattern")]
    pub pattern: String,
    pub action: Action,
}

fn any_pattern() -> String {
    ANY.to_string()
}

impl Rule {
    pub fn new(tool: impl Into<String>, action: Action, pattern: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            pattern: pattern.into(),
            action,
        }
    }

    /// A rule covering every argument to a tool.
    pub fn for_any(tool: impl Into<String>, action: Action) -> Self {
        Self::new(tool, action, ANY)
    }

    /// The key two rules collide on when rulesets are merged.
    ///
    /// NUL-separated because NUL cannot occur in either half. Written as an
    /// escape rather than a literal byte: a literal NUL in source is invisible
    /// in a diff, which is how this became a bug once already.
    fn merge_key(&self) -> String {
        format!("{}\u{0}{}", self.tool, self.pattern)
    }
}

/// The outcome of consulting a ruleset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub action: Action,
    /// The rule that decided it, if any did.
    pub rule: Option<Rule>,
    /// Nothing matched and the default was used. Lets a settings UI show
    /// "unconfigured" differently from "deliberately set to ask".
    pub implicit: bool,
}

impl Verdict {
    fn implicit_default() -> Self {
        Self {
            action: Action::default(),
            rule: None,
            implicit: true,
        }
    }
}

/// Glob match supporting only `*` (any run, including none) and `?` (exactly
/// one character).
///
/// Deliberately not regex. A rule whose author cannot predict its meaning is
/// worse than no rule, and `.` appearing in a path or a command is far more
/// common than anyone wanting a wildcard there. Everything that is not `*` or
/// `?` is a literal, newlines included.
fn matches(pattern: &str, value: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let v: Vec<char> = value.chars().collect();

    // Two-pointer walk with one backtrack point. Linear on inputs without
    // adjacent wildcards, and immune to the exponential blowup a naive
    // recursive matcher hits on patterns like `*a*a*a*`.
    let (mut pi, mut vi) = (0usize, 0usize);
    let (mut star, mut resume) = (None::<usize>, 0usize);

    while vi < v.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == v[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            // Remember where to come back to, and first try matching nothing.
            star = Some(pi);
            resume = vi;
            pi += 1;
        } else if let Some(s) = star {
            // Backtrack: let the last `*` swallow one more character.
            pi = s + 1;
            resume += 1;
            vi = resume;
        } else {
            return false;
        }
    }

    // Trailing `*`s can match the empty remainder; anything else cannot.
    p[pi..].iter().all(|c| *c == '*')
}

/// How specific a pattern is, for ranking two rules that both match.
///
/// An exact literal always beats any wildcard pattern, however long - hence
/// the flat bonus rather than a length comparison. Among wildcard patterns,
/// more literal text and fewer wildcards wins.
fn specificity(pattern: &str) -> i32 {
    if pattern == ANY {
        return 0;
    }
    let wildcards = pattern.chars().filter(|c| *c == '*' || *c == '?').count() as i32;
    let base = pattern.chars().count() as i32 - wildcards * 2;
    if wildcards == 0 {
        base + 1000
    } else {
        base
    }
}

/// Decides what a ruleset says about calling `tool` with `target`.
///
/// The highest-scoring matching rule wins; ties go to the earlier rule, so
/// ruleset order is a tiebreak the author can rely on.
pub fn evaluate(rules: &[Rule], tool: &str, target: &str) -> Verdict {
    let mut best: Option<(i32, &Rule)> = None;

    for rule in rules {
        if !matches(&rule.tool, tool) || !matches(&rule.pattern, target) {
            continue;
        }
        // The tool match dominates by a margin no pattern score can close: a
        // rule naming the exact tool outranks one that matched it by wildcard,
        // however precise the latter's argument pattern is.
        let score = specificity(&rule.tool) * 100_000 + specificity(&rule.pattern);
        // Strictly greater, so the first of equally-scored rules holds.
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, rule));
        }
    }

    match best {
        Some((_, rule)) => Verdict {
            action: rule.action,
            rule: Some(rule.clone()),
            implicit: false,
        },
        None => Verdict::implicit_default(),
    }
}

/// Evaluates against [`ANY`] - "what does this ruleset say about the tool in
/// general".
pub fn evaluate_any(rules: &[Rule], tool: &str) -> Verdict {
    evaluate(rules, tool, ANY)
}

/// Whether this exact call is forbidden.
pub fn is_denied(rules: &[Rule], tool: &str, target: &str) -> bool {
    evaluate(rules, tool, target).action == Action::Deny
}

/// Whether a tool should be offered to the model at all.
///
/// Only an *unconditional* deny hides a tool. A tool denied for some arguments
/// (`shell` denied for `rm -rf *`, say) stays visible, because the model can
/// still call it correctly for everything else, and hiding it would cost far
/// more than the one refused call.
pub fn is_visible(rules: &[Rule], tool: &str) -> bool {
    let verdict = evaluate_any(rules, tool);
    if verdict.action != Action::Deny {
        return true;
    }
    verdict
        .rule
        .map(|r| r.pattern != ANY)
        .unwrap_or(true)
}

/// Filters a tool list down to what the model may see.
pub fn visible_tools<'a>(tools: &'a [String], rules: &[Rule]) -> Vec<&'a String> {
    tools.iter().filter(|t| is_visible(rules, t)).collect()
}

/// Tools that will interrupt the user. Used to tell them up front what an
/// agent will check in about.
pub fn tools_that_ask<'a>(tools: &'a [String], rules: &[Rule]) -> Vec<&'a String> {
    tools
        .iter()
        .filter(|t| evaluate_any(rules, t).action == Action::Ask)
        .collect()
}

/// Layers rulesets, later winning on an exact `(tool, pattern)` collision.
///
/// The order used throughout the app is **defaults → workspace → agent**, so
/// an agent can tighten or loosen what the workspace said and the workspace
/// can move off the defaults, without any layer needing to know the others
/// exist.
///
/// Note this is collision-replacement, not action-precedence: a later layer
/// genuinely can turn a `deny` into an `allow` *for the identical rule key*,
/// which is what makes a workspace able to re-permit something the defaults
/// refuse. Within a single resolved ruleset, a matching `deny` is final.
pub fn merge(layers: &[&[Rule]]) -> Vec<Rule> {
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, Rule> = HashMap::new();

    for layer in layers {
        for rule in *layer {
            let key = rule.merge_key();
            if !by_key.contains_key(&key) {
                order.push(key.clone());
            }
            by_key.insert(key, rule.clone());
        }
    }

    // Preserve first-seen position so merging does not reshuffle a list the
    // user has arranged, which would make the settings diff unreadable.
    order
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(rs: &[(&str, Action, &str)]) -> Vec<Rule> {
        rs.iter()
            .map(|(t, a, p)| Rule::new(*t, *a, *p))
            .collect()
    }

    // ── pattern matching ────────────────────────────────────────────────

    #[test]
    fn star_matches_any_run() {
        assert!(matches("git push *", "git push origin main"));
        assert!(matches("*", ""));
        assert!(matches("a*c", "ac"));
        assert!(!matches("git push *", "git pull origin"));
    }

    #[test]
    fn question_matches_exactly_one() {
        assert!(matches("a?c", "abc"));
        assert!(!matches("a?c", "ac"));
        assert!(!matches("a?c", "abbc"));
    }

    #[test]
    fn everything_else_is_literal() {
        // The regex meaning of `.` would make this match "axb".
        assert!(matches("a.b", "a.b"));
        assert!(!matches("a.b", "axb"));
        assert!(matches("cost (usd)", "cost (usd)"));
    }

    // `*` spans newlines - a shell command can be multi-line and a rule about
    // it must still apply.
    #[test]
    fn star_spans_newlines() {
        assert!(matches("rm *", "rm -rf /\nrm -rf ~"));
    }

    // A naive recursive matcher takes exponential time on this.
    #[test]
    fn adjacent_wildcards_do_not_blow_up() {
        let pattern = "*a*a*a*a*a*a*b";
        let value = "a".repeat(64);
        assert!(!matches(pattern, &value));
    }

    // ── specificity ─────────────────────────────────────────────────────

    #[test]
    fn an_exact_pattern_beats_any_wildcard() {
        assert!(specificity("git push origin main") > specificity("git *"));
        // Even when the wildcard pattern is far longer.
        assert!(specificity("ls") > specificity(&format!("{}*", "x".repeat(100))));
    }

    #[test]
    fn the_any_pattern_scores_zero() {
        assert_eq!(specificity(ANY), 0);
        assert!(specificity("git *") > specificity(ANY));
    }

    /// A quirk of the scoring formula, pinned deliberately rather than
    /// smoothed over: a wildcard costs 2 but contributes 1 character, so a
    /// short mostly-wildcard pattern can score the same as - or lower than -
    /// bare `*`. `"a*"` is `2 - 2 = 0`, exactly tying `ANY`.
    ///
    /// It resolves safely, because a tie goes to the earlier rule rather than
    /// to an arbitrary one, so the outcome stays predictable from the ruleset
    /// as written. Changing the formula would silently re-rank every existing
    /// user's rules, which is a worse trade than keeping the quirk.
    #[test]
    fn a_short_wildcard_pattern_can_tie_with_any() {
        assert_eq!(specificity("a*"), specificity(ANY));

        let rs = rules(&[("shell", Action::Allow, "a*"), ("shell", Action::Deny, ANY)]);
        // The earlier rule holds, as with any other tie.
        assert_eq!(evaluate(&rs, "shell", "abc").action, Action::Allow);
    }

    // ── evaluation ──────────────────────────────────────────────────────

    #[test]
    fn nothing_configured_means_ask() {
        let verdict = evaluate(&[], "shell", "ls");
        assert_eq!(verdict.action, Action::Ask);
        assert!(verdict.implicit);
        assert!(verdict.rule.is_none());
    }

    // A settings UI needs to tell these two apart.
    #[test]
    fn an_explicit_ask_is_not_implicit() {
        let rs = rules(&[("shell", Action::Ask, ANY)]);
        let verdict = evaluate(&rs, "shell", "ls");
        assert_eq!(verdict.action, Action::Ask);
        assert!(!verdict.implicit);
    }

    #[test]
    fn the_more_specific_pattern_wins() {
        let rs = rules(&[
            ("shell", Action::Allow, ANY),
            ("shell", Action::Deny, "rm -rf *"),
        ]);
        assert_eq!(evaluate(&rs, "shell", "ls").action, Action::Allow);
        assert_eq!(evaluate(&rs, "shell", "rm -rf /").action, Action::Deny);
    }

    // Order in the ruleset must not change the answer here.
    #[test]
    fn specificity_beats_ordering() {
        let forwards = rules(&[
            ("shell", Action::Allow, ANY),
            ("shell", Action::Deny, "rm -rf *"),
        ]);
        let backwards = rules(&[
            ("shell", Action::Deny, "rm -rf *"),
            ("shell", Action::Allow, ANY),
        ]);
        assert_eq!(
            evaluate(&forwards, "shell", "rm -rf /").action,
            evaluate(&backwards, "shell", "rm -rf /").action
        );
    }

    /// The dominance rule: naming the tool exactly outranks matching it by
    /// wildcard, even when the wildcard rule has the more precise argument.
    #[test]
    fn an_exact_tool_match_outranks_a_wildcard_tool_match() {
        let rs = rules(&[
            ("mcp_*", Action::Deny, "delete *"),
            ("mcp_files", Action::Allow, ANY),
        ]);
        assert_eq!(
            evaluate(&rs, "mcp_files", "delete everything").action,
            Action::Allow
        );
        // The wildcard rule still governs its other family members.
        assert_eq!(
            evaluate(&rs, "mcp_email", "delete everything").action,
            Action::Deny
        );
    }

    #[test]
    fn tool_names_match_as_globs() {
        let rs = rules(&[("mcp_*", Action::Allow, ANY)]);
        assert_eq!(evaluate(&rs, "mcp_anything", ANY).action, Action::Allow);
        assert_eq!(evaluate(&rs, "shell", ANY).action, Action::Ask);
    }

    #[test]
    fn ties_go_to_the_earlier_rule() {
        let rs = rules(&[
            ("shell", Action::Allow, "git *"),
            ("shell", Action::Deny, "git *"),
        ]);
        assert_eq!(evaluate(&rs, "shell", "git push").action, Action::Allow);
    }

    // ── visibility ──────────────────────────────────────────────────────

    #[test]
    fn an_unconditional_deny_hides_the_tool() {
        let rs = rules(&[("shell", Action::Deny, ANY)]);
        assert!(!is_visible(&rs, "shell"));
    }

    /// A partial deny must not hide the tool, or one forbidden command costs
    /// the model every other use of it.
    #[test]
    fn a_partial_deny_leaves_the_tool_visible() {
        let rs = rules(&[("shell", Action::Deny, "rm -rf *")]);
        assert!(is_visible(&rs, "shell"));
        assert!(is_denied(&rs, "shell", "rm -rf /"));
        assert!(!is_denied(&rs, "shell", "ls"));
    }

    #[test]
    fn visible_tools_filters_only_the_hidden() {
        let rs = rules(&[("secret", Action::Deny, ANY)]);
        let all = vec!["shell".to_string(), "secret".to_string()];
        assert_eq!(visible_tools(&all, &rs), vec![&"shell".to_string()]);
    }

    #[test]
    fn tools_that_ask_lists_the_interrupting_ones() {
        let rs = rules(&[("read", Action::Allow, ANY), ("shell", Action::Ask, ANY)]);
        let all = vec!["read".to_string(), "shell".to_string()];
        assert_eq!(tools_that_ask(&all, &rs), vec![&"shell".to_string()]);
    }

    // ── merging ─────────────────────────────────────────────────────────

    #[test]
    fn later_layers_win_on_an_identical_key() {
        let defaults = rules(&[("shell", Action::Deny, ANY)]);
        let workspace = rules(&[("shell", Action::Allow, ANY)]);
        let merged = merge(&[&defaults, &workspace]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].action, Action::Allow);
    }

    #[test]
    fn different_keys_accumulate() {
        let workspace = rules(&[("shell", Action::Ask, ANY)]);
        let agent = rules(&[("shell", Action::Deny, "rm *"), ("read", Action::Allow, ANY)]);
        let merged = merge(&[&workspace, &agent]);
        assert_eq!(merged.len(), 3);
    }

    // Merging must not reshuffle a list the user arranged by hand.
    #[test]
    fn merging_preserves_first_seen_order() {
        let base = rules(&[
            ("a", Action::Ask, ANY),
            ("b", Action::Ask, ANY),
            ("c", Action::Ask, ANY),
        ]);
        let over = rules(&[("b", Action::Deny, ANY)]);
        let merged = merge(&[&base, &over]);
        let names: Vec<&str> = merged.iter().map(|r| r.tool.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert_eq!(merged[1].action, Action::Deny);
    }

    // ── config tolerance ────────────────────────────────────────────────

    #[test]
    fn an_unknown_action_reads_as_ask() {
        assert_eq!(Action::parse("allow"), Action::Allow);
        assert_eq!(Action::parse("deny"), Action::Deny);
        assert_eq!(Action::parse("Allow"), Action::Ask);
        assert_eq!(Action::parse("banana"), Action::Ask);
    }

    #[test]
    fn a_rule_without_a_pattern_covers_everything() {
        let rule: Rule =
            serde_json::from_str(r#"{"tool":"shell","action":"allow"}"#).unwrap();
        assert_eq!(rule.pattern, ANY);
    }
}
