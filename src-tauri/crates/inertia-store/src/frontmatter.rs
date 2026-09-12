//! Markdown with frontmatter, as a record format.
//!
//! Skills use it, agents use it, and anything else whose main content is prose a
//! human will want to edit should use it too. The alternative - JSON with an
//! `"instructions": "line\nline\nline"` field - is the same data in a form
//! nobody can read or diff, which for a file the user is invited to open in
//! their own editor is the whole ballgame.
//!
//! The parser is intentionally small: scalars, inline `[a, b]` lists, block
//! lists, and one level of indented block. The indented block exists for
//! exactly one reason - an agent carries its own permission rules, and
//!
//! ```text
//! permissions:
//!   shell: ask
//!   git push *: deny
//! ```
//!
//! is the only way to write those that a person will get right on the first
//! try. Anything deeper belongs in the body, where a human is going to read it
//! anyway.
//!
//! # Why this is hand-rolled rather than `serde_yaml`
//!
//! A real YAML parser is stricter than the files people actually write here,
//! and its disagreements are all in the direction of losing data:
//! `git push *: deny` is a key containing a colon and a glob, `version: 1.2.3`
//! must stay the string `1.2.3` rather than becoming a float, and a document
//! that fails to parse must degrade to "no frontmatter" instead of erroring out
//! of a list and taking every other skill with it. This is a faithful port of
//! the reference implementation, quirks included, because the files on disk
//! were written against those quirks.

use std::collections::BTreeMap;

/// One frontmatter value. Deliberately shallow: scalars, lists, and one level
/// of map, which is everything the format allows.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Bool(bool),
    Number(f64),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The text a caller wants to show, whatever the value happens to be.
    pub fn to_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => format_number(*n),
            Self::List(items) => items
                .iter()
                .map(Self::to_text)
                .collect::<Vec<_>>()
                .join(", "),
            Self::Map(_) => String::new(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Text(s) => serde_json::Value::String(s.clone()),
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Number(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Self::List(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Map(entries) => serde_json::Value::Object(
                entries
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json()))
                    .collect(),
            ),
        }
    }

    /// The reverse, for writing a record back out.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(match value {
            serde_json::Value::Null => return None,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => Self::Number(n.as_f64()?),
            serde_json::Value::String(s) => Self::Text(s.clone()),
            serde_json::Value::Array(items) => {
                Self::List(items.iter().filter_map(Self::from_json).collect())
            }
            serde_json::Value::Object(entries) => Self::Map(
                entries
                    .iter()
                    .filter_map(|(k, v)| Self::from_json(v).map(|v| (k.clone(), v)))
                    .collect(),
            ),
        })
    }
}

/// A parsed document: what the frontmatter said, and everything below it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub meta: BTreeMap<String, Value>,
    pub body: String,
}

fn unquote(value: &str) -> &str {
    let quoted = (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''));
    // `len() > 1` so a lone quote character is not sliced into nothing.
    if quoted && value.len() > 1 {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

/// Whether a bare token should be read as a number.
///
/// Only `-?digits(.digits)?`, which is what keeps `1.2.3` a version string
/// rather than the float 1.2. Anything with a second dot, a leading plus or an
/// exponent stays text.
fn looks_numeric(value: &str) -> bool {
    let rest = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = match rest.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (rest, None),
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match fraction {
        None => true,
        Some(f) => !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()),
    }
}

fn parse_scalar(raw: &str) -> Value {
    let value = raw.trim();
    if value.is_empty() {
        return Value::Text(String::new());
    }
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
        let inner = &value[1..value.len() - 1];
        return Value::List(
            inner
                .split(',')
                .map(|part| unquote(part.trim()).to_string())
                .filter(|part| !part.is_empty())
                .map(Value::Text)
                .collect(),
        );
    }
    if looks_numeric(value) {
        if let Ok(number) = value.parse::<f64>() {
            return Value::Number(number);
        }
    }
    Value::Text(unquote(value).to_string())
}

/// `key: value` at column zero. The key is an identifier; a line that is not
/// one is skipped rather than failing the document.
fn split_key(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some((key, rest.trim_start_matches([' ', '\t'])))
}

/// The same, inside a block, where the key is a pattern rather than an
/// identifier.
///
/// Greedy up to the LAST colon, because a pattern is far more likely to contain
/// one (`git push *`, `shell/npm run *`) than a block's value is - block values
/// are actions, and it is hosts that are full of `https:`.
fn split_block_key(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.rsplit_once(':')?;
    if key.trim().is_empty() {
        return None;
    }
    Some((key.trim(), rest.trim_start_matches([' ', '\t'])))
}

/// Splits a document into frontmatter and body. No frontmatter means all body.
///
/// Trailing spaces on a fence are allowed: a file typed by hand with one stray
/// space after `---` is otherwise read as having no frontmatter at all, which
/// turns its name and description into body text and gives no clue why the
/// skill went blank.
pub fn parse(text: &str) -> Document {
    let source = text.strip_prefix('\u{feff}').unwrap_or(text);

    let Some((front, body)) = split_fences(source) else {
        return Document {
            meta: BTreeMap::new(),
            body: source.trim().to_string(),
        };
    };

    let mut meta: BTreeMap<String, Value> = BTreeMap::new();
    // The key a following indented line belongs to. A block ends the moment a
    // line comes back to column zero, so nothing has to be closed explicitly.
    let mut block: Option<String> = None;

    for line in front.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // A list item belongs to whatever key is open, at any indentation. YAML
        // allows a block sequence to sit at its parent's own column, and hand
        // written frontmatter uses both, so indentation cannot be the test.
        if let (Some(item), Some(open)) = (list_item(line), block.as_ref()) {
            // The first `- ` decides the shape. Until one arrives an open key
            // is an empty map, because that is what an indented map would need.
            let entry = meta.entry(open.clone()).or_insert_with(|| Value::List(vec![]));
            if !matches!(entry, Value::List(_)) {
                *entry = Value::List(vec![]);
            }
            if let Value::List(items) = entry {
                let value = parse_scalar(item);
                if value != Value::Text(String::new()) {
                    items.push(value);
                }
            }
            continue;
        }

        if line.starts_with([' ', '\t']) {
            if let Some(open) = block.as_ref() {
                let entry = meta.entry(open.clone()).or_insert_with(|| Value::Map(BTreeMap::new()));
                if matches!(entry, Value::List(_)) {
                    continue;
                }
                if let Some((key, rest)) = split_block_key(line.trim()) {
                    if let Value::Map(entries) = entry {
                        entries.insert(key.to_string(), parse_scalar(rest));
                    }
                }
                continue;
            }
        }

        let Some((key, rest)) = split_key(line) else {
            continue;
        };

        if rest.trim().is_empty() {
            // `key:` with nothing after it opens a block. If nothing indented
            // follows, it stays an empty map, which is the honest reading.
            meta.insert(key.to_string(), Value::Map(BTreeMap::new()));
            block = Some(key.to_string());
            continue;
        }

        block = None;
        meta.insert(key.to_string(), parse_scalar(rest));
    }

    Document {
        meta,
        body: body.trim().to_string(),
    }
}

/// A `- item` line, whatever its indentation.
fn list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('-')?;
    // `- ` requires the space: `--flag` in a body-ish line is not a list item.
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some(rest.trim_start())
}

/// Finds the opening and closing `---` fences, returning what lies between them
/// and what follows.
fn split_fences(source: &str) -> Option<(&str, &str)> {
    let after_open = strip_fence_line(source)?;
    let mut offset = 0usize;
    // Walk line by line looking for a closing fence at column zero.
    for line in after_open.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        if content.trim_end_matches([' ', '\t']) == "---" {
            let front = &after_open[..offset];
            let rest = &after_open[offset + line.len()..];
            return Some((front.trim_end_matches(['\n', '\r']), rest));
        }
        offset += line.len();
    }
    None
}

/// Consumes a leading `---` line, tolerating trailing whitespace and CRLF.
fn strip_fence_line(source: &str) -> Option<&str> {
    let rest = source.strip_prefix("---")?;
    let rest = rest.trim_start_matches([' ', '\t']);
    rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))
}

/// `1.0` prints as `1`, so a round trip does not turn an integer into a float.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Whether a scalar has to be quoted to survive being read back.
fn needs_quotes(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let first = value.as_bytes()[0];
    if b" >|&*#{}[]!%@`'\"-".contains(&first) || first.is_ascii_whitespace() {
        return true;
    }
    if value.ends_with([' ', '\t']) {
        return true;
    }
    // A colon followed by whitespace would read as a second key.
    value
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b':' && pair[1].is_ascii_whitespace())
}

fn format_scalar(value: &Value) -> String {
    match value {
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(format_scalar).collect::<Vec<_>>().join(", ")
        ),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(*n),
        Value::Map(_) => String::new(),
        Value::Text(text) => {
            if needs_quotes(text) {
                serde_json::Value::String(text.clone()).to_string()
            } else {
                text.clone()
            }
        }
    }
}

/// Writes a document back out.
///
/// Keys are written in the order given, and empty ones are dropped - a file
/// full of `description:` with nothing after it teaches the reader nothing and
/// makes a diff of a real change harder to see.
pub fn serialize(meta: &[(String, Value)], body: &str) -> String {
    let mut lines: Vec<String> = Vec::new();

    for (key, value) in meta {
        match value {
            Value::Text(text) if text.is_empty() => continue,
            Value::List(items) => {
                if !items.is_empty() {
                    lines.push(format!("{key}: {}", format_scalar(value)));
                }
                continue;
            }
            Value::Map(entries) => {
                if entries.is_empty() {
                    continue;
                }
                lines.push(format!("{key}:"));
                for (inner, v) in entries {
                    lines.push(format!("  {inner}: {}", format_scalar(v)));
                }
                continue;
            }
            _ => {}
        }
        lines.push(format!("{key}: {}", format_scalar(value)));
    }

    format!("---\n{}\n---\n\n{}\n", lines.join("\n"), body.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Value {
        Value::Text(value.to_string())
    }

    #[test]
    fn a_document_without_frontmatter_is_all_body() {
        let parsed = parse("# Just a heading\n\nand prose.");
        assert!(parsed.meta.is_empty());
        assert_eq!(parsed.body, "# Just a heading\n\nand prose.");
    }

    #[test]
    fn scalars_round_trip() {
        let parsed = parse("---\nname: Code review\nenabled: false\n---\n\nBody.");
        assert_eq!(parsed.meta["name"], text("Code review"));
        assert_eq!(parsed.meta["enabled"], Value::Bool(false));
        assert_eq!(parsed.body, "Body.");
    }

    /// `parseFloat` would read this as 1.2 and silently discard the patch
    /// number, which is why the numeric test is deliberately narrow.
    #[test]
    fn a_version_stays_a_string() {
        let parsed = parse("---\nversion: 1.2.3\n---\n\nx");
        assert_eq!(parsed.meta["version"], text("1.2.3"));
    }

    #[test]
    fn a_bare_number_is_a_number() {
        let parsed = parse("---\nweight: 12\nratio: -0.5\n---\n\nx");
        assert_eq!(parsed.meta["weight"], Value::Number(12.0));
        assert_eq!(parsed.meta["ratio"], Value::Number(-0.5));
    }

    #[test]
    fn inline_lists_parse() {
        let parsed = parse("---\ntags: [release, deploy]\n---\n\nx");
        assert_eq!(
            parsed.meta["tags"],
            Value::List(vec![text("release"), text("deploy")])
        );
    }

    /// Every skill format in the wild uses block lists. A parser that turned
    /// one into an empty object would drop data out of a file the user wrote by
    /// hand and which looks correct.
    #[test]
    fn block_lists_parse() {
        let parsed = parse("---\ntags:\n  - release\n  - deploy\n---\n\nx");
        assert_eq!(
            parsed.meta["tags"],
            Value::List(vec![text("release"), text("deploy")])
        );
    }

    /// YAML allows a block sequence at its parent's own column, and hand
    /// written frontmatter uses both.
    #[test]
    fn a_block_list_at_column_zero_still_belongs_to_its_key() {
        let parsed = parse("---\ntags:\n- release\n- deploy\n---\n\nx");
        assert_eq!(
            parsed.meta["tags"],
            Value::List(vec![text("release"), text("deploy")])
        );
    }

    /// The reason indented blocks exist at all.
    #[test]
    fn permission_patterns_may_contain_colons_and_globs() {
        let parsed = parse("---\npermissions:\n  shell: ask\n  git push *: deny\n---\n\nx");
        let Value::Map(rules) = &parsed.meta["permissions"] else {
            panic!("permissions should be a map");
        };
        assert_eq!(rules["shell"], text("ask"));
        assert_eq!(rules["git push *"], text("deny"));
    }

    /// A stray space after the fence used to read as "no frontmatter", which
    /// turned a skill's name into body text and gave no clue why it went blank.
    #[test]
    fn a_fence_may_carry_trailing_whitespace() {
        let parsed = parse("--- \nname: Thing\n--- \n\nBody.");
        assert_eq!(parsed.meta["name"], text("Thing"));
        assert_eq!(parsed.body, "Body.");
    }

    #[test]
    fn crlf_documents_parse() {
        let parsed = parse("---\r\nname: Thing\r\n---\r\n\r\nBody.");
        assert_eq!(parsed.meta["name"], text("Thing"));
        assert_eq!(parsed.body, "Body.");
    }

    #[test]
    fn a_leading_byte_order_mark_is_ignored() {
        let parsed = parse("\u{feff}---\nname: Thing\n---\n\nBody.");
        assert_eq!(parsed.meta["name"], text("Thing"));
    }

    #[test]
    fn an_unterminated_fence_is_all_body() {
        let parsed = parse("---\nname: Thing\n\nstill going");
        assert!(parsed.meta.is_empty());
    }

    #[test]
    fn empty_values_are_not_written() {
        let out = serialize(
            &[
                ("name".into(), text("Thing")),
                ("description".into(), text("")),
            ],
            "Body.",
        );
        assert!(out.contains("name: Thing"));
        assert!(!out.contains("description"));
    }

    #[test]
    fn a_value_that_would_reparse_wrongly_is_quoted() {
        let out = serialize(&[("name".into(), text("weird: value"))], "x");
        assert!(out.contains("name: \"weird: value\""));
        assert_eq!(parse(&out).meta["name"], text("weird: value"));
    }

    #[test]
    fn serialize_then_parse_is_a_round_trip() {
        let mut rules = BTreeMap::new();
        rules.insert("git push *".to_string(), text("deny"));
        let out = serialize(
            &[
                ("name".into(), text("Code review")),
                ("enabled".into(), Value::Bool(true)),
                ("tags".into(), Value::List(vec![text("ci")])),
                ("permissions".into(), Value::Map(rules)),
            ],
            "Do the thing.",
        );
        let parsed = parse(&out);
        assert_eq!(parsed.meta["name"], text("Code review"));
        assert_eq!(parsed.meta["enabled"], Value::Bool(true));
        assert_eq!(parsed.meta["tags"], Value::List(vec![text("ci")]));
        assert_eq!(parsed.body, "Do the thing.");
    }

    #[test]
    fn an_integer_does_not_gain_a_decimal_point_on_the_way_out() {
        let out = serialize(&[("weight".into(), Value::Number(12.0))], "x");
        assert!(out.contains("weight: 12"), "{out}");
    }
}
