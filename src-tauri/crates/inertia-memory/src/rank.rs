//! Which memories matter right now.
//!
//! BM25 over titles, tags and bodies. That is not a placeholder for embeddings:
//! at the scale a personal workspace reaches it is microseconds, it needs no
//! key, no service and no network, and it degrades honestly - a query whose
//! words appear nowhere returns nothing rather than returning the three
//! least-unrelated things with a confident score attached. Anyone who wants
//! meaning-matching can point the app at a backend that has it.

use std::collections::HashMap;

use serde_json::Value;

use crate::record::{self, INJECT_MAX, MAX_SUMMARY};

/// Words that carry no signal.
///
/// Short on purpose. A long stop list is tuned to English prose, and these are
/// titles and tags written by a model and a person in a hurry, where throwing
/// away half the words to save a few bytes of index costs more matches than it
/// saves noise.
const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "how", "in", "is", "it",
    "of", "on", "or", "that", "the", "this", "to", "was", "what", "when", "where", "which", "with",
];

pub fn terms(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| word.len() > 1 && !STOP.contains(word))
        .map(str::to_string)
        .collect()
}

/// The searchable text of one memory, with the parts that matter counted twice.
///
/// A word in the title or a tag says more about what a memory is *for* than the
/// same word buried in the body, and repeating the field is how a bag-of-words
/// model is told so without a second set of weights to keep in step.
pub fn text_of(memory: &Value) -> String {
    let title = record::text(memory, "title");
    let tags = record::tags(memory).join(" ");
    [
        title.as_str(),
        title.as_str(),
        tags.as_str(),
        tags.as_str(),
        &record::text(memory, "description"),
        &record::text(memory, "body"),
    ]
    .iter()
    .filter(|part| !part.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" ")
}

/// BM25's two dials, at the values everyone uses because they work.
const K1: f64 = 1.2;
const B: f64 = 0.75;

#[derive(Debug)]
struct Doc {
    counts: HashMap<String, usize>,
    length: usize,
}

/// The index for a set of memories.
///
/// Built once and thrown away when a memory changes. At the scale a personal
/// workspace reaches - hundreds, maybe thousands - this is microseconds and
/// there is nothing to gain from anything cleverer. A vector database earns its
/// keep at a million records; here it would be a service to install, back up and
/// explain, in exchange for nothing anyone could measure.
#[derive(Debug)]
pub struct Index {
    docs: Vec<Doc>,
    seen: HashMap<String, usize>,
    total: usize,
    average_length: f64,
}

pub fn index(memories: &[&Value]) -> Index {
    let docs: Vec<Doc> = memories
        .iter()
        .map(|memory| {
            let words = terms(&text_of(memory));
            let mut counts: HashMap<String, usize> = HashMap::new();
            for word in &words {
                *counts.entry(word.clone()).or_insert(0) += 1;
            }
            Doc { counts, length: words.len() }
        })
        .collect();

    let mut seen: HashMap<String, usize> = HashMap::new();
    for doc in &docs {
        for word in doc.counts.keys() {
            *seen.entry(word.clone()).or_insert(0) += 1;
        }
    }

    let total = docs.len().max(1);
    let spread: usize = docs.iter().map(|doc| doc.length).sum();
    let average_length = if spread == 0 {
        1.0
    } else {
        spread as f64 / total as f64
    };

    Index { docs, seen, total, average_length }
}

/// The classic inverse document frequency, floored so a common word cannot go
/// negative.
fn idf(index: &Index, word: &str) -> f64 {
    let n = *index.seen.get(word).unwrap_or(&0) as f64;
    let total = index.total as f64;
    (1.0 + (total - n + 0.5) / (n + 0.5)).ln().max(0.05)
}

/// How long ago, as a number between 0 and 1 that decays over a season.
///
/// Gentle on purpose: a memory from March is not wrong, it is just less likely
/// to be what this conversation is about, and a sharp decay would bury facts
/// that are still true because nobody happened to need them recently.
fn freshness(iso: &str, now_ms: i64) -> f64 {
    let Ok(at) = iso.parse::<jiff::Timestamp>() else {
        return 0.0;
    };
    let days = (now_ms - at.as_millisecond()) as f64 / 86_400_000.0;
    if days <= 0.0 {
        return 1.0;
    }
    1.0 / (1.0 + days / 90.0)
}

fn aged(memory: &Value, now_ms: i64) -> f64 {
    for field in ["lastUsedAt", "updatedAt", "createdAt"] {
        let value = record::text(memory, field);
        if !value.is_empty() {
            return freshness(&value, now_ms);
        }
    }
    0.0
}

/// How much a query is about the memory at this position in the index.
///
/// By position rather than by value: the index holds one document per memory in
/// the order they were given, and looking a document up any other way is how a
/// score ends up belonging to a different memory than the one it is reported
/// against.
fn relevance_of(index: &Index, position: usize, words: &[String]) -> f64 {
    let Some(doc) = index.docs.get(position) else {
        return 0.0;
    };
    let mut relevance = 0.0;
    for word in words {
        let count = *doc.counts.get(word).unwrap_or(&0) as f64;
        if count == 0.0 {
            continue;
        }
        let norm = count * (K1 + 1.0);
        let denom = count + K1 * (1.0 - B + (B * doc.length as f64) / index.average_length);
        relevance += idf(index, word) * (norm / denom);
    }
    relevance
}

/// One memory and what it scored.
#[derive(Debug, Clone)]
pub struct Scored<'a> {
    pub memory: &'a Value,
    /// How much the query is actually about this memory. Zero means the words
    /// appear nowhere in it.
    pub relevance: f64,
    pub score: f64,
}

/// What a pin and recency are worth beside relevance.
#[derive(Debug, Clone, Copy)]
pub struct Weights {
    pub pin: f64,
    pub fresh: f64,
}

impl Default for Weights {
    /// Relevance decides, a pin lifts, recency breaks ties.
    ///
    /// A pinned memory is the user saying "this one always matters", so it gets
    /// a real boost rather than a rounding error - but not so much that a pin
    /// outranks a memory that actually answers the question, because then
    /// pinning six things would be the same as pinning none.
    fn default() -> Self {
        Self { pin: 1.5, fresh: 0.6 }
    }
}

/// Score every memory against a query, best first.
pub fn score<'a>(
    index: &Index,
    memories: &[&'a Value],
    query: &str,
    now_ms: i64,
    weights: Weights,
) -> Vec<Scored<'a>> {
    let words = terms(query);

    let mut rows: Vec<Scored<'a>> = memories
        .iter()
        .enumerate()
        .map(|(position, memory)| {
            let relevance = relevance_of(index, position, &words);
            let pinned = if record::is_pinned(memory) { weights.pin } else { 0.0 };
            let score = relevance + pinned + aged(memory, now_ms) * weights.fresh;
            Scored { memory, relevance, score }
        })
        .collect();

    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| record::text(a.memory, "id").cmp(&record::text(b.memory, "id")))
    });
    rows
}

/// The memories a query is actually about.
///
/// `relevance > 0` is the whole filter and it matters: without it every query
/// returns the pinned memories plus whatever is newest, which looks like recall
/// working and is in fact recall having nothing to say. A pinned memory reaches
/// the model through the injected block regardless.
pub fn search<'a>(memories: &[&'a Value], query: &str, limit: usize, now_ms: i64) -> Vec<&'a Value> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let built = index(memories);
    score(&built, memories, query, now_ms, Weights::default())
        .into_iter()
        .filter(|row| row.relevance > 0.0)
        .take(limit)
        .map(|row| row.memory)
        .collect()
}

/// What goes in the prompt.
///
/// Three tiers, and the middle one is the point.
///
/// Pinned memories and the handover note go first whatever is being asked: the
/// user chose them, or they are what the last conversation left behind. Then the
/// memories that are actually *about* what was just said, scored the way recall
/// scores them. Then, with whatever budget is left, the ones that have proved
/// useful before - a small baseline for the case where the message shares no
/// words with a memory that matters.
///
/// Without the middle tier this is a popularity contest: a hundred memories
/// means the twenty most-used go in regardless of the question, and the one that
/// answers it stays on disk.
///
/// Cut to a byte budget rather than a count, because twenty terse memories and
/// twenty verbose ones are not the same purchase.
pub fn for_prompt<'a>(
    memories: &[&'a Value],
    query: &str,
    budget: usize,
    now_ms: i64,
) -> Vec<&'a Value> {
    // Relevance alone, by position: the tiers below decide what a pin and
    // recency are worth, so scoring them in here as well would count both twice.
    let mut relevance: HashMap<usize, f64> = HashMap::new();
    if !query.trim().is_empty() {
        let built = index(memories);
        let words = terms(query);
        for position in 0..memories.len() {
            let found = relevance_of(&built, position, &words);
            if found > 0.0 {
                relevance.insert(position, found);
            }
        }
    }

    let tier = |position: usize, memory: &Value| -> u8 {
        if record::is_pinned(memory) || record::text(memory, "kind") == "handover" {
            return 0;
        }
        if relevance.contains_key(&position) {
            return 1;
        }
        2
    };

    let mut ranked: Vec<(usize, &'a Value)> = memories.iter().copied().enumerate().collect();
    ranked.sort_by(|(left, a), (right, b)| {
        tier(*left, a)
            .cmp(&tier(*right, b))
            .then_with(|| {
                let by_relevance = relevance.get(right).unwrap_or(&0.0);
                by_relevance
                    .partial_cmp(relevance.get(left).unwrap_or(&0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| use_count(b).cmp(&use_count(a)))
            .then_with(|| {
                aged(b, now_ms)
                    .partial_cmp(&aged(a, now_ms))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut kept = Vec::new();
    let mut spent = 0usize;
    for (_, memory) in ranked {
        if kept.len() >= INJECT_MAX {
            break;
        }
        let line = format!(
            "{}: {}",
            record::text(memory, "title"),
            record::summarise(memory, MAX_SUMMARY)
        );
        // A pinned memory is never dropped for being long. The user pinned it;
        // the budget is for deciding what else fits around it.
        if spent + line.len() > budget && !record::is_pinned(memory) && !kept.is_empty() {
            break;
        }
        kept.push(memory);
        spent += line.len();
    }
    kept
}

fn use_count(memory: &Value) -> i64 {
    memory.get("useCount").and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> i64 {
        jiff::Timestamp::now().as_millisecond()
    }

    #[test]
    fn the_words_that_carry_no_signal_are_dropped() {
        assert_eq!(terms("The quick brown fox"), ["quick", "brown", "fox"]);
        // Single characters are not terms either: one letter carries no more
        // signal than "the" does, and underscores are part of a word because
        // half the identifiers a memory names contain one.
        assert_eq!(terms("a b-c_d!"), ["c_d"]);
    }

    #[test]
    fn a_query_matching_nothing_finds_nothing() {
        let rows = [
            json!({ "id": "a", "title": "Deploys go to fly.io", "body": "not render" }),
            json!({ "id": "b", "title": "Tests run with vitest", "body": "not jest" }),
        ];
        let refs: Vec<&Value> = rows.iter().collect();
        assert!(search(&refs, "kubernetes ingress", 8, now()).is_empty());
        // And a query that matches finds the right one, not merely the first.
        let found = search(&refs, "how do tests run", 8, now());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["id"], "b");
    }

    #[test]
    fn an_empty_query_is_not_a_search() {
        let rows = [json!({ "id": "a", "title": "x", "body": "y" })];
        let refs: Vec<&Value> = rows.iter().collect();
        assert!(search(&refs, "   ", 8, now()).is_empty());
    }

    /// The bug this ordering exists to prevent: a popularity contest in which
    /// the memory that answers the question never reaches the prompt.
    #[test]
    fn the_memory_that_answers_the_question_outranks_the_popular_one() {
        let rows = [
            json!({ "id": "popular", "title": "Coffee preferences", "body": "flat white", "useCount": 90 }),
            json!({ "id": "answer", "title": "Deploys go to fly.io", "body": "never render", "useCount": 0 }),
        ];
        let refs: Vec<&Value> = rows.iter().collect();
        let kept = for_prompt(&refs, "where do deploys go", 2560, now());
        assert_eq!(kept[0]["id"], "answer");
    }

    #[test]
    fn pinned_and_handover_come_first_whatever_was_asked() {
        let rows = [
            json!({ "id": "plain", "title": "Deploys go to fly.io", "body": "x", "useCount": 50 }),
            json!({ "id": "note", "title": "Where we left off", "body": "y", "kind": "handover" }),
            json!({ "id": "pin", "title": "Call me Tanvir", "body": "z", "pinned": true }),
        ];
        let refs: Vec<&Value> = rows.iter().collect();
        let kept = for_prompt(&refs, "deploys", 2560, now());
        let first: Vec<&str> = kept
            .iter()
            .take(2)
            .map(|m| m["id"].as_str().unwrap_or_default())
            .collect();
        assert!(first.contains(&"note") && first.contains(&"pin"));
        assert_eq!(kept[2]["id"], "plain");
    }

    #[test]
    fn the_budget_is_bytes_and_a_pin_is_never_cut_for_length() {
        let long = "x".repeat(400);
        let rows = [
            json!({ "id": "a", "title": "one", "body": long }),
            json!({ "id": "b", "title": "two", "body": long }),
            json!({ "id": "pinned", "title": "three", "body": long, "pinned": true }),
        ];
        let refs: Vec<&Value> = rows.iter().collect();
        // Room for one entry at most.
        let kept = for_prompt(&refs, "", 200, now());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["id"], "pinned");
    }

    /// A pin is the user saying "this one always matters", so it is worth a
    /// real boost rather than a rounding error - but it is not relevance, and
    /// it never turns a memory the query says nothing about into a match.
    #[test]
    fn a_pin_lifts_a_memory_but_is_not_relevance() {
        let rows = [
            json!({ "id": "pinned", "title": "Unrelated", "body": "nothing", "pinned": true }),
            json!({ "id": "plain", "title": "Also unrelated", "body": "nothing" }),
        ];
        let refs: Vec<&Value> = rows.iter().collect();
        let built = index(&refs);
        let ranked = score(&built, &refs, "deploys", now(), Weights::default());
        assert_eq!(ranked[0].memory["id"], "pinned");

        // And recall still finds neither: a pin is not an answer.
        assert!(search(&refs, "deploys", 8, now()).is_empty());
    }
}
