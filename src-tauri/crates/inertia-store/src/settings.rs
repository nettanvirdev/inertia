//! Workspace settings.
//!
//! One file per concern, so a hand edit or a git conflict is scoped to the
//! thing it is about rather than to "settings".
//!
//! One rule runs through all of it: **`models.json` never contains an API
//! key.** It holds the *name* of a secret, and the value lives in
//! `secrets.json`. That separation is what lets the rest of the workspace
//! folder be shared, committed, or pasted into a bug report without leaking
//! credentials.

use std::collections::BTreeMap;

use inertia_core::permission::Rule;
use serde::{Deserialize, Serialize};

use crate::fsx;
use crate::layout::{Document, Layout};

/// Which wire protocol a provider speaks.
///
/// Getting this wrong is not a hard failure - it silently loses prompt caching
/// and extended thinking - so it is stored explicitly rather than guessed at
/// request time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Anthropic,
    OpenAi,
}

impl Protocol {
    /// Infers the protocol from a base URL, for records written before `kind`
    /// existed.
    pub fn infer(base_url: &str) -> Self {
        if base_url.contains("anthropic") {
            Self::Anthropic
        } else {
            Self::OpenAi
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecord {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRecord {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub base_url: String,
    /// The **name** of an entry in `secrets.json`. Never the key itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_secret: Option<String>,
    /// Omitted in older records, in which case it is inferred from the URL.
    #[serde(default, rename = "kind", skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Protocol>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelRecord>,
}

fn yes() -> bool {
    true
}

impl ProviderRecord {
    pub fn protocol(&self) -> Protocol {
        self.protocol
            .unwrap_or_else(|| Protocol::infer(&self.base_url))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Models {
    pub providers: Vec<ProviderRecord>,
    /// `"<providerId>/<modelId>"`.
    pub default_model: String,
}

impl Models {
    /// Splits a `provider/model` reference.
    ///
    /// Model ids contain slashes themselves (`openai/gpt-4o` on a gateway), so
    /// only the first separator counts.
    pub fn split_reference(reference: &str) -> Option<(&str, &str)> {
        reference.split_once('/')
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderRecord> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// The provider and model a reference names, if the provider is enabled.
    pub fn resolve<'a>(&'a self, reference: &'a str) -> Option<(&'a ProviderRecord, &'a str)> {
        let (provider_id, model) = Self::split_reference(reference)?;
        let provider = self.provider(provider_id)?;
        provider.enabled.then_some((provider, model))
    }
}

/// Stored credentials. Read as late as possible and never written anywhere
/// else.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Secrets {
    pub entries: BTreeMap<String, SecretEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SecretEntry {
    pub value: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
}

impl Secrets {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.get(name).map(|e| e.value.as_str())
    }
}

/// Permission rules, layered workspace-wide then per agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Permissions {
    pub workspace: Vec<Rule>,
    pub agents: BTreeMap<String, Vec<Rule>>,
}

impl Permissions {
    /// The effective ruleset for an agent: workspace rules, then its own on
    /// top, so an agent can tighten or loosen what the workspace said.
    pub fn for_agent(&self, agent: Option<&str>) -> Vec<Rule> {
        let own = agent
            .and_then(|id| self.agents.get(id))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        inertia_core::permission::merge(&[&self.workspace, own])
    }
}

/// Reads and writes the settings documents.
#[derive(Debug, Clone)]
pub struct Settings {
    layout: Layout,
}

impl Settings {
    pub fn new(layout: Layout) -> Self {
        Self { layout }
    }

    pub fn models(&self) -> Models {
        fsx::read_json::<Models>(&self.layout.document(Document::Models)).0
    }

    pub fn save_models(&self, models: &Models) -> fsx::Result<()> {
        fsx::write_json(&self.layout.document(Document::Models), models)
    }

    pub fn secrets(&self) -> Secrets {
        fsx::read_json::<Secrets>(&self.layout.document(Document::Secrets)).0
    }

    pub fn save_secrets(&self, secrets: &Secrets) -> fsx::Result<()> {
        fsx::write_json(&self.layout.document(Document::Secrets), secrets)
    }

    pub fn permissions(&self) -> Permissions {
        fsx::read_json::<Permissions>(&self.layout.document(Document::Permissions)).0
    }

    pub fn save_permissions(&self, permissions: &Permissions) -> fsx::Result<()> {
        fsx::write_json(&self.layout.document(Document::Permissions), permissions)
    }

    /// Resolves a provider's API key from the secret store.
    ///
    /// An absent key is an empty string rather than an error: a local runtime
    /// needs no key, and refusing to build a provider without one would make
    /// the common local-model setup impossible.
    pub fn api_key_for(&self, provider: &ProviderRecord) -> String {
        let Some(name) = &provider.api_key_secret else {
            return String::new();
        };
        self.secrets().get(name).unwrap_or_default().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::permission::Action;

    fn settings() -> (tempfile::TempDir, Settings) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.scaffold().unwrap();
        (dir, Settings::new(layout))
    }

    fn provider(id: &str, base_url: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.into(),
            name: id.into(),
            enabled: true,
            base_url: base_url.into(),
            api_key_secret: Some("TEST_KEY".into()),
            protocol: None,
            headers: BTreeMap::new(),
            models: Vec::new(),
        }
    }

    #[test]
    fn an_empty_workspace_has_no_providers() {
        let (_dir, settings) = settings();
        assert!(settings.models().providers.is_empty());
    }

    #[test]
    fn models_round_trip() {
        let (_dir, settings) = settings();
        let models = Models {
            providers: vec![provider("anthropic", "https://api.anthropic.com/v1")],
            default_model: "anthropic/claude-sonnet-4".into(),
        };
        settings.save_models(&models).unwrap();

        let read = settings.models();
        assert_eq!(read.providers.len(), 1);
        assert_eq!(read.default_model, "anthropic/claude-sonnet-4");
    }

    /// The separation that lets the workspace folder be shared safely.
    #[test]
    fn the_key_is_never_written_into_models_json() {
        let (dir, settings) = settings();
        settings
            .save_models(&Models {
                providers: vec![provider("anthropic", "https://api.anthropic.com/v1")],
                default_model: String::new(),
            })
            .unwrap();
        settings
            .save_secrets(&Secrets {
                entries: BTreeMap::from([(
                    "TEST_KEY".to_string(),
                    SecretEntry {
                        value: "sk-super-secret".into(),
                        ..Default::default()
                    },
                )]),
            })
            .unwrap();

        let raw = std::fs::read_to_string(dir.path().join("settings/models.json")).unwrap();
        assert!(!raw.contains("sk-super-secret"), "the key leaked: {raw}");
        assert!(raw.contains("TEST_KEY"), "the reference was lost: {raw}");

        // ...and it resolves through the secret store.
        let models = settings.models();
        assert_eq!(
            settings.api_key_for(&models.providers[0]),
            "sk-super-secret"
        );
    }

    /// A local runtime needs no key, and refusing to build one without a key
    /// would make that setup impossible.
    #[test]
    fn a_missing_key_resolves_to_empty_rather_than_failing() {
        let (_dir, settings) = settings();
        let mut record = provider("local", "http://localhost:11434/v1");
        record.api_key_secret = None;
        assert_eq!(settings.api_key_for(&record), "");

        record.api_key_secret = Some("NOT_SET".into());
        assert_eq!(settings.api_key_for(&record), "");
    }

    #[test]
    fn the_protocol_is_inferred_when_absent() {
        assert_eq!(
            provider("x", "https://api.anthropic.com/v1").protocol(),
            Protocol::Anthropic
        );
        assert_eq!(
            provider("x", "http://localhost:11434/v1").protocol(),
            Protocol::OpenAi
        );
    }

    #[test]
    fn an_explicit_protocol_wins_over_inference() {
        let mut record = provider("gateway", "https://gateway.example.com/anthropic");
        record.protocol = Some(Protocol::OpenAi);
        assert_eq!(record.protocol(), Protocol::OpenAi);
    }

    /// Gateways serve models whose ids contain slashes, so only the first one
    /// separates provider from model.
    #[test]
    fn a_model_reference_splits_on_the_first_slash_only() {
        assert_eq!(
            Models::split_reference("openrouter/openai/gpt-4o"),
            Some(("openrouter", "openai/gpt-4o"))
        );
    }

    #[test]
    fn resolving_skips_a_disabled_provider() {
        let mut record = provider("anthropic", "https://api.anthropic.com/v1");
        record.enabled = false;
        let models = Models {
            providers: vec![record],
            default_model: String::new(),
        };
        assert!(models.resolve("anthropic/claude-sonnet-4").is_none());
    }

    // ── permissions ─────────────────────────────────────────────────────

    #[test]
    fn agent_rules_layer_over_workspace_rules() {
        let permissions = Permissions {
            workspace: vec![Rule::for_any("shell", Action::Ask)],
            agents: BTreeMap::from([(
                "atlas".to_string(),
                vec![Rule::for_any("shell", Action::Deny)],
            )]),
        };

        let base = permissions.for_agent(None);
        assert_eq!(base[0].action, Action::Ask);

        let atlas = permissions.for_agent(Some("atlas"));
        assert_eq!(atlas.len(), 1);
        assert_eq!(atlas[0].action, Action::Deny);
    }

    #[test]
    fn an_unknown_agent_gets_the_workspace_rules() {
        let permissions = Permissions {
            workspace: vec![Rule::for_any("shell", Action::Ask)],
            agents: BTreeMap::new(),
        };
        assert_eq!(permissions.for_agent(Some("nobody")).len(), 1);
    }

    /// An older build wrote a different shape here. Ignoring it means falling
    /// back to defaults, which is safe; mis-translating it would not be.
    #[test]
    fn a_legacy_permissions_shape_falls_back_to_defaults() {
        let (dir, settings) = settings();
        std::fs::write(
            dir.path().join("settings/permissions.json"),
            r#"{ "rules": { "shell": true } }"#,
        )
        .unwrap();

        let permissions = settings.permissions();
        assert!(permissions.workspace.is_empty());
        assert!(permissions.agents.is_empty());
    }
}
