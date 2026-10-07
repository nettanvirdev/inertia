//! The voice command surface.
//!
//! Six things the settings pane and the composer need from ElevenLabs. There is
//! no turn path here and no tool, so nothing in this file runs unless a person
//! asked for speech.
//!
//! The key is fetched per call rather than held. A cached key survives the user
//! changing it in the Secrets pane, which turns "I fixed the key" into "it
//! still says the key is wrong until you restart", and the read is a JSON file
//! on local disk.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use inertia_store::layout::Document;
use inertia_voice::{decoded_size, ElevenLabs, SpeakRequest};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

/// The secret a fresh install looks for before the user has renamed anything.
const DEFAULT_KEY_SECRET: &str = "ELEVENLABS_API_KEY";

const NO_KEY: &str = "No ElevenLabs API key is stored. Add one under Settings, Secrets.";

/// The most audio a single transcription may carry, decoded.
///
/// Roughly ten minutes of the webm/opus a browser produces, which is far longer
/// than anything anyone dictates into a message box. Not ElevenLabs' limit; the
/// point past which the recording is more likely a bug in the renderer than a
/// person talking.
const MAX_TRANSCRIBE_BYTES: usize = 8 * 1024 * 1024;

/// The most text a single speak request may carry.
///
/// ElevenLabs charges per character and its own per-request cap depends on the
/// model, the lowest being 10k. Rejecting here rather than letting the API do
/// it means a runaway string costs the user a message instead of a month's
/// quota.
const MAX_SPEAK_CHARS: usize = 10_000;

/// How long a fetched voice or model list stays good.
///
/// The settings pane refetches both every time it opens, and an account's
/// voices change when the user adds one, which is rare and deliberate. Ninety
/// seconds is long enough that opening the pane twice to compare two voices
/// costs one round trip, and short enough that a voice cloned in the ElevenLabs
/// dashboard shows up without restarting the app.
const LIST_CACHE: Duration = Duration::from_secs(90);

struct Cached {
    /// Part of the freshness check, because a different key is a different
    /// account with different voices - and serving the previous account's list
    /// would be a wrong answer rather than a slow one.
    key: String,
    at: Instant,
    value: Value,
}

fn cache() -> &'static Mutex<HashMap<&'static str, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(name: &'static str, key: &str) -> Option<Value> {
    let held = cache().lock();
    let hit = held.get(name)?;
    (hit.key == key && hit.at.elapsed() < LIST_CACHE).then(|| hit.value.clone())
}

fn remember(name: &'static str, key: &str, value: Value) {
    cache().lock().insert(
        name,
        Cached {
            key: key.to_string(),
            at: Instant::now(),
            value,
        },
    );
}

/// Dropped when a test or a key change wants the next call to go out for real.
pub fn invalidate() {
    cache().lock().clear();
}

/// Which secret holds the key, and then the key itself.
///
/// The settings document names a secret; it never carries one. That is what
/// lets the workspace be exported or screenshotted without leaking the key, and
/// it is why every path to ElevenLabs comes through here.
fn api_key(state: &AppState) -> Result<String, String> {
    let workspace = state.workspace()?;
    let app =
        inertia_store::collections::read_document(&workspace.layout, Document::App, json!({}));
    let name = app
        .get("elevenLabsKeySecret")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_KEY_SECRET);

    Ok(inertia_store::secrets::get(&workspace.layout, name)
        .map_err(|e| e.to_string())?
        .unwrap_or_default())
}

/// A client, or a message that says exactly what to do about it.
fn client(state: &AppState) -> Result<(ElevenLabs, String), String> {
    let key = api_key(state)?;
    if key.is_empty() {
        return Err(NO_KEY.to_string());
    }
    Ok((ElevenLabs::new(key.clone()), key))
}

/// Whether Settings can offer anything at all yet.
#[tauri::command]
pub fn voice_configured(state: State<'_, AppState>) -> bool {
    api_key(&state).map(|key| !key.is_empty()).unwrap_or(false)
}

/// Proves the key works, rather than that a string is stored.
#[tauri::command]
pub async fn voice_test(state: State<'_, AppState>) -> Result<Value, String> {
    let (client, _) = client(&state)?;
    // A test is the moment to stop trusting anything cached: the user pressed
    // it because something changed.
    invalidate();
    let account = client.whoami().await.map_err(|e| e.to_string())?;
    serde_json::to_value(account).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn voice_voices(state: State<'_, AppState>) -> Result<Value, String> {
    let (client, key) = client(&state)?;
    if let Some(hit) = cached("voices", &key) {
        return Ok(hit);
    }
    let voices = client.voices().await.map_err(|e| e.to_string())?;
    let value = serde_json::to_value(voices).map_err(|e| e.to_string())?;
    remember("voices", &key, value.clone());
    Ok(value)
}

#[tauri::command]
pub async fn voice_models(state: State<'_, AppState>) -> Result<Value, String> {
    let (client, key) = client(&state)?;
    if let Some(hit) = cached("models", &key) {
        return Ok(hit);
    }
    let models = client.models().await.map_err(|e| e.to_string())?;
    let value = serde_json::to_value(models).map_err(|e| e.to_string())?;
    remember("models", &key, value.clone());
    Ok(value)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeakInput {
    pub text: String,
    pub voice_id: String,
    pub model_id: Option<String>,
    pub stability: Option<f64>,
    pub similarity: Option<f64>,
}

#[tauri::command]
pub async fn voice_speak(state: State<'_, AppState>, request: SpeakInput) -> Result<Value, String> {
    let (client, _) = client(&state)?;

    let spoken = request.text.trim();
    if spoken.is_empty() {
        return Err("There is nothing to say.".into());
    }
    let length = spoken.chars().count();
    if length > MAX_SPEAK_CHARS {
        return Err(format!(
            "That is {length} characters and the limit is {MAX_SPEAK_CHARS}. Speak a shorter passage rather than spending the quota on all of it."
        ));
    }

    let speech = client
        .speak(&SpeakRequest {
            text: spoken.to_string(),
            voice_id: request.voice_id,
            model_id: request.model_id,
            stability: request.stability,
            similarity: request.similarity,
        })
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(speech).map_err(|e| e.to_string())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscribeInput {
    /// Base64, as the recorder handed it over.
    pub bytes: String,
    pub mime_type: String,
    pub model_id: Option<String>,
    pub language: Option<String>,
}

#[tauri::command]
pub async fn voice_transcribe(
    state: State<'_, AppState>,
    request: TranscribeInput,
) -> Result<Value, String> {
    let (client, _) = client(&state)?;
    if request.bytes.trim().is_empty() {
        return Err("There is no audio to transcribe.".into());
    }

    // Measured without decoding: allocating eight megabytes only to throw them
    // away is the one thing this guard exists to avoid.
    let size = decoded_size(&request.bytes);
    if size > MAX_TRANSCRIBE_BYTES {
        return Err(format!(
            "That recording is {} MB and the limit is {} MB. Record a shorter passage.",
            size / (1024 * 1024),
            MAX_TRANSCRIBE_BYTES / (1024 * 1024)
        ));
    }

    let transcript = client
        .transcribe(
            &request.bytes,
            &request.mime_type,
            request.model_id.as_deref(),
            request.language.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(transcript).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache has to be keyed on the account, not just the endpoint: two
    /// keys are two sets of voices, and serving the wrong one is a wrong answer
    /// rather than a slow one.
    #[test]
    fn a_cached_list_is_not_served_to_a_different_key() {
        invalidate();
        remember("voices", "key-a", json!([{ "id": "v1" }]));

        assert!(cached("voices", "key-a").is_some());
        assert!(cached("voices", "key-b").is_none());
        invalidate();
    }

    #[test]
    fn invalidating_drops_everything() {
        remember("models", "key-a", json!([]));
        invalidate();
        assert!(cached("models", "key-a").is_none());
    }

    #[test]
    fn a_miss_is_not_an_error() {
        invalidate();
        assert!(cached("voices", "anything").is_none());
    }
}
