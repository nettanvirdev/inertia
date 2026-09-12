//! What is known about a model without asking.
//!
//! There is deliberately **no supported-models list**. A provider is "a URL the
//! user typed", and any model id they enter works. What is tabulated here is
//! only the two numbers no endpoint reliably reports - the context window and
//! the output ceiling - and only so the UI can draw a usage meter and the
//! request builder can avoid asking for more output than a model will give.
//!
//! Matching is by **longest id prefix**, so a new dated release
//! (`claude-sonnet-4-5-20260101`) inherits its family's numbers the day it
//! ships rather than falling off the table.

/// `(id prefix, context window, max output)`.
///
/// Ordered longest-prefix-first is *not* required - the lookup picks the
/// longest match itself, so entries can be added anywhere.
const MODELS: &[(&str, u32, u32)] = &[
    // Anthropic
    ("claude-opus-4", 200_000, 32_000),
    ("claude-sonnet-4", 200_000, 64_000),
    ("claude-haiku-4", 200_000, 32_000),
    ("claude-3-7-sonnet", 200_000, 64_000),
    ("claude-3-5-sonnet", 200_000, 8_192),
    ("claude-3-5-haiku", 200_000, 8_192),
    ("claude-3-opus", 200_000, 4_096),
    ("claude-3-haiku", 200_000, 4_096),
    ("claude-", 200_000, 8_192),
    // OpenAI
    ("gpt-5", 400_000, 128_000),
    ("gpt-4.1", 1_047_576, 32_768),
    ("gpt-4o-mini", 128_000, 16_384),
    ("gpt-4o", 128_000, 16_384),
    ("gpt-4-turbo", 128_000, 4_096),
    ("gpt-4", 8_192, 4_096),
    ("gpt-3.5-turbo", 16_385, 4_096),
    ("o4-mini", 200_000, 100_000),
    ("o3-mini", 200_000, 100_000),
    ("o3", 200_000, 100_000),
    ("o1", 200_000, 100_000),
];

fn lookup(model: &str) -> Option<&'static (&'static str, u32, u32)> {
    let id = model.to_ascii_lowercase();
    MODELS
        .iter()
        .filter(|(prefix, _, _)| id.starts_with(prefix))
        // Longest prefix wins, so `claude-sonnet-4` beats the `claude-`
        // catch-all and a dated variant matches its family.
        .max_by_key(|(prefix, _, _)| prefix.len())
}

/// Context window in tokens, if known.
pub fn context_window(model: &str) -> Option<u32> {
    lookup(model).map(|(_, context, _)| *context)
}

/// Maximum output tokens, if known.
pub fn max_output(model: &str) -> Option<u32> {
    lookup(model).map(|(_, _, output)| *output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_model_reports_both_numbers() {
        assert_eq!(context_window("claude-sonnet-4"), Some(200_000));
        assert_eq!(max_output("claude-sonnet-4"), Some(64_000));
    }

    /// The reason for prefix matching: a release that did not exist when this
    /// table was written still gets its family's numbers.
    #[test]
    fn a_dated_release_inherits_its_family() {
        assert_eq!(max_output("claude-sonnet-4-5-20260101"), Some(64_000));
        assert_eq!(context_window("gpt-4o-2024-11-20"), Some(128_000));
    }

    #[test]
    fn the_longest_prefix_wins() {
        // Not the `claude-` catch-all, and not `gpt-4`.
        assert_eq!(max_output("claude-opus-4-1"), Some(32_000));
        assert_eq!(context_window("gpt-4o-mini"), Some(128_000));
        assert_eq!(context_window("gpt-4-turbo"), Some(128_000));
        assert_eq!(context_window("gpt-4"), Some(8_192));
    }

    #[test]
    fn model_ids_match_case_insensitively() {
        assert_eq!(context_window("Claude-Sonnet-4"), Some(200_000));
    }

    /// An unknown model is not an error - the user may well have typed
    /// something real that no table has caught up with.
    #[test]
    fn an_unknown_model_reports_nothing_rather_than_guessing() {
        assert_eq!(context_window("llama-3-70b"), None);
        assert_eq!(max_output("some-local-model"), None);
    }
}
