//! How a provider is configured.
//!
//! Deliberately "a URL the user typed plus a key". There is no allowlist of
//! supported providers: any endpoint speaking one of the two protocols works,
//! which is what lets someone point this at a local model, a gateway, or a
//! service that did not exist when the app shipped.

use std::collections::HashMap;

use secrecy::SecretString;

#[derive(Clone)]
pub struct ProviderConfig {
    /// Stable identifier, e.g. `"anthropic"`. Used in logs and to match stored
    /// settings; not sent anywhere.
    pub id: String,
    /// Base URL with no trailing slash, e.g. `https://api.anthropic.com/v1`.
    pub base_url: String,
    /// Wrapped so a stray `{:?}` cannot print it into a log file.
    pub api_key: SecretString,
    /// Extra headers, which take precedence over the defaults this crate sets.
    /// A gateway that needs its own auth header can therefore override ours
    /// rather than fighting it.
    pub headers: HashMap<String, String>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("headers", &self.headers.keys())
            .finish()
    }
}

impl ProviderConfig {
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let base_url: String = base_url.into();
        Self {
            id: id.into(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: SecretString::from(api_key.into()),
            headers: HashMap::new(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// The full URL for one route.
    ///
    /// Resolved through [`crate::base`] rather than joined, because what people
    /// paste into the base-URL field is often not a base URL: a curl example
    /// with the route still on the end, or a gateway mounted with no version in
    /// its path. Joining those naively produced `/anthropic/messages`, which
    /// nobody serves, and reported a working provider as broken.
    pub fn endpoint(&self, route: &str) -> String {
        crate::base::endpoint(&self.base_url, route)
    }

    /// After a 404: try the next base this address could have meant.
    ///
    /// Returns whether there was one, so a caller can retry once rather than
    /// reporting a failure that is a path segment away from success.
    pub fn advance_base(&self) -> bool {
        crate::base::advance(&self.base_url)
    }

    pub fn can_advance_base(&self) -> bool {
        crate::base::can_advance(&self.base_url)
    }

    /// Whether the caller already set this header, case-insensitively.
    ///
    /// Header names are case-insensitive on the wire, so a config setting
    /// `Authorization` must suppress our `authorization` and not end up
    /// sending both.
    pub fn overrides(&self, name: &str) -> bool {
        self.headers.keys().any(|k| k.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_does_not_double_up() {
        let config = ProviderConfig::new("x", "https://api.example.com/v1/", "key");
        assert_eq!(
            config.endpoint("messages"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            config.endpoint("/messages"),
            "https://api.example.com/v1/messages"
        );
    }

    #[test]
    fn header_overrides_are_case_insensitive() {
        let config = ProviderConfig::new("x", "https://e.com", "key")
            .with_header("Authorization", "Bearer other");
        assert!(config.overrides("authorization"));
        assert!(config.overrides("AUTHORIZATION"));
        assert!(!config.overrides("x-api-key"));
    }

    /// The key must not be printable by accident. This is the whole reason for
    /// the `secrecy` wrapper.
    #[test]
    fn debug_output_never_contains_the_key() {
        let config = ProviderConfig::new("x", "https://e.com", "sk-super-secret");
        let printed = format!("{config:?}");
        assert!(!printed.contains("sk-super-secret"), "got {printed}");
    }
}
