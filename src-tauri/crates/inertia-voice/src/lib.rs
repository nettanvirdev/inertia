//! The ElevenLabs HTTP client.
//!
//! Five endpoints: who the key belongs to, which voices the account has, which
//! models it may use, turn text into audio, turn audio into text. That is the
//! whole of what voice mode needs, and it is small enough that the official SDK
//! would be a dependency tree in exchange for five requests.
//!
//! It lives here rather than in the window for the same two reasons the LLM and
//! Composio clients do: a browser would hit CORS on every call, and the API key
//! must never exist in a window that renders model-authored markdown. Audio
//! crosses the bridge as base64, never as a URL the renderer would have to
//! fetch itself - the window's content policy allows no remote origin at all,
//! which is a property worth keeping.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const BASE_URL: &str = "https://api.elevenlabs.io";

/// Long enough for a paragraph of speech, short enough to fail while watching.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// The default voice model.
///
/// Flash is the one worth defaulting to for a chat reply: it is the cheapest
/// per character and its latency is low enough that speech starts while a
/// person is still looking at the message. Quality is chosen per request, so
/// anyone who wants the slower, better model can say so.
pub const DEFAULT_TTS_MODEL: &str = "eleven_flash_v2_5";

/// ElevenLabs has exactly one transcription model worth naming.
pub const DEFAULT_STT_MODEL: &str = "scribe_v1";

/// mp3 at 44.1kHz/128kbps.
///
/// The renderer decodes this with the Web Audio API rather than handing it to
/// an `<audio>` element, so the container has to be something `decodeAudioData`
/// understands everywhere. mp3 is, and it is a third the size of the PCM
/// alternatives over a bridge.
const OUTPUT_FORMAT: &str = "mp3_44100_128";

pub type Result<T> = std::result::Result<T, VoiceError>;

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("{0}")]
    Refused(String),
    #[error("ElevenLabs did not answer in time.")]
    Timeout,
    #[error("Could not reach ElevenLabs: {0}")]
    Unreachable(String),
    #[error("{0}")]
    Invalid(String),
}

/// Turns a failed response into something a person can act on.
///
/// An ElevenLabs failure is nearly always one of four fixable things: the key
/// is wrong, the plan does not cover this model, the voice id does not exist,
/// or the character quota is spent. The status says which; the body usually
/// carries their own words, which beat ours.
fn describe_failure(status: u16, body: &str) -> VoiceError {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|parsed| {
            let detail = parsed.get("detail");
            detail
                .and_then(|d| d.get("message"))
                .and_then(Value::as_str)
                .or_else(|| detail.and_then(|d| d.get("status")).and_then(Value::as_str))
                .or_else(|| parsed.get("message").and_then(Value::as_str))
                .or_else(|| detail.and_then(Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string());

    let detail: String = detail.trim().chars().take(400).collect();

    let lead = match status {
        401 => "ElevenLabs rejected the key. Check it under Settings, Secrets.".to_string(),
        403 => "That key is not allowed to do this. It may be missing a permission, or the plan may not cover it.".to_string(),
        404 => "ElevenLabs has no such voice or model.".to_string(),
        422 => "ElevenLabs refused the request as malformed.".to_string(),
        429 => "Too many requests, or the character quota for this month is spent.".to_string(),
        other => format!("ElevenLabs answered {other}."),
    };

    VoiceError::Refused(if detail.is_empty() {
        lead
    } else {
        format!("{lead} {detail}")
    })
}

/// What the account is, and how much of it is left.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub tier: String,
    pub used: u64,
    pub limit: u64,
    pub remaining: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    /// ElevenLabs' own words for how a voice sounds. They read better than
    /// anything we would invent, and there is no other source.
    pub tone: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceModel {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// mp3 bytes, and what they are.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Speech {
    pub mime_type: String,
    /// Base64, because that is what survives the bridge intact; the renderer
    /// turns it back into an ArrayBuffer and decodes it.
    pub bytes: String,
    pub size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub language: String,
}

/// What to say, and how.
#[derive(Debug, Clone, Default)]
pub struct SpeakRequest {
    pub text: String,
    pub voice_id: String,
    pub model_id: Option<String>,
    pub stability: Option<f64>,
    pub similarity: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ElevenLabs {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl ElevenLabs {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(BASE_URL, api_key)
    }

    pub fn with_base_url(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request
            .header("xi-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    VoiceError::Timeout
                } else {
                    VoiceError::Unreachable(e.to_string())
                }
            })?;

        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(describe_failure(status, &body))
    }

    async fn json(&self, path: &str) -> Result<Value> {
        let response = self.send(self.http.get(self.url(path))).await?;
        response
            .json()
            .await
            .map_err(|e| VoiceError::Unreachable(e.to_string()))
    }

    /// Who the key belongs to.
    ///
    /// The cheapest call ElevenLabs refuses outright when a key is wrong, which
    /// is what makes it the right one behind a Test button: it proves the key
    /// works rather than that a string is stored. The quota comes back with it,
    /// which is the other thing a person wants to know at that moment.
    pub async fn whoami(&self) -> Result<Subscription> {
        let data = self.json("/v1/user/subscription").await?;
        let number = |key: &str| data.get(key).and_then(Value::as_u64).unwrap_or(0);
        let used = number("character_count");
        let limit = number("character_limit");
        Ok(Subscription {
            tier: data
                .get("tier")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or("free")
                .to_string(),
            used,
            limit,
            remaining: limit.saturating_sub(used),
        })
    }

    /// The account's voices, in the shape the settings pane draws.
    pub async fn voices(&self) -> Result<Vec<Voice>> {
        let data = self.json("/v1/voices").await?;
        let rows = data
            .get("voices")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(rows
            .into_iter()
            .filter_map(|voice| {
                let id = voice.get("voice_id").and_then(Value::as_str)?.to_string();
                if id.is_empty() {
                    return None;
                }
                let label = |key: &str| {
                    voice
                        .get("labels")
                        .and_then(|l| l.get(key))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                };
                let tone = [label("accent"), label("description"), label("age")]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");

                let name = voice
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();

                Some(Voice {
                    id,
                    name: if name.is_empty() {
                        "Unnamed".into()
                    } else {
                        name
                    },
                    tone,
                    category: voice
                        .get("category")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                })
            })
            .collect())
    }

    /// The models the account may use for speech, so the pane offers no dead
    /// ones.
    pub async fn models(&self) -> Result<Vec<VoiceModel>> {
        let data = self.json("/v1/models").await?;
        let rows = data.as_array().cloned().unwrap_or_default();

        Ok(rows
            .into_iter()
            .filter(|model| {
                model
                    .get("can_do_text_to_speech")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .filter_map(|model| {
                let id = model.get("model_id").and_then(Value::as_str)?.to_string();
                if id.is_empty() {
                    return None;
                }
                let name = model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                Some(VoiceModel {
                    name: if name.is_empty() { id.clone() } else { name },
                    id,
                    description: model
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                })
            })
            .collect())
    }

    /// Text in, mp3 bytes out.
    ///
    /// The speaking rate is deliberately NOT sent here - it is applied at
    /// playback, where changing it costs nothing and does not spend the
    /// character quota again.
    pub async fn speak(&self, request: &SpeakRequest) -> Result<Speech> {
        let spoken = request.text.trim();
        if spoken.is_empty() {
            return Err(VoiceError::Invalid("There is nothing to say.".into()));
        }
        if request.voice_id.is_empty() {
            return Err(VoiceError::Invalid("No voice was chosen.".into()));
        }

        let mut settings = serde_json::Map::new();
        if let Some(stability) = request.stability.filter(|v| v.is_finite()) {
            settings.insert("stability".into(), json!(stability));
        }
        if let Some(similarity) = request.similarity.filter(|v| v.is_finite()) {
            settings.insert("similarity_boost".into(), json!(similarity));
        }

        let mut body = json!({
            "text": spoken,
            "model_id": request.model_id.as_deref().filter(|m| !m.is_empty()).unwrap_or(DEFAULT_TTS_MODEL),
        });
        if !settings.is_empty() {
            body["voice_settings"] = Value::Object(settings);
        }

        let path = format!(
            "/v1/text-to-speech/{}?output_format={OUTPUT_FORMAT}",
            urlencode(&request.voice_id)
        );
        let response = self
            .send(
                self.http
                    .post(self.url(&path))
                    .header("accept", "audio/mpeg")
                    .json(&body),
            )
            .await?;

        let audio = response
            .bytes()
            .await
            .map_err(|e| VoiceError::Unreachable(e.to_string()))?;

        Ok(Speech {
            mime_type: "audio/mpeg".into(),
            bytes: base64::engine::general_purpose::STANDARD.encode(&audio),
            size: audio.len(),
        })
    }

    /// Audio in, text out.
    ///
    /// The renderer records with MediaRecorder and hands over base64, so the
    /// container is whatever the engine chose - usually webm/opus. ElevenLabs
    /// sniffs the container itself, so the filename here only has to be
    /// plausible.
    pub async fn transcribe(
        &self,
        base64_audio: &str,
        mime_type: &str,
        model_id: Option<&str>,
        language: Option<&str>,
    ) -> Result<Transcript> {
        let audio = base64::engine::general_purpose::STANDARD
            .decode(base64_audio.trim())
            .map_err(|_| VoiceError::Invalid("That recording is not valid base64.".into()))?;
        if audio.is_empty() {
            return Err(VoiceError::Invalid("The recording was empty.".into()));
        }

        let mime = if mime_type.is_empty() {
            "audio/webm"
        } else {
            mime_type
        };
        let filename = if mime.contains("mp4") {
            "recording.mp4"
        } else {
            "recording.webm"
        };

        let part = reqwest::multipart::Part::bytes(audio)
            .file_name(filename)
            .mime_str(mime)
            .map_err(|e| VoiceError::Invalid(e.to_string()))?;

        let mut form = reqwest::multipart::Form::new()
            .text(
                "model_id",
                model_id
                    .filter(|m| !m.is_empty())
                    .unwrap_or(DEFAULT_STT_MODEL)
                    .to_string(),
            )
            .part("file", part);
        if let Some(language) = language.filter(|l| !l.is_empty()) {
            form = form.text("language_code", language.to_string());
        }

        let response = self
            .send(
                self.http
                    .post(self.url("/v1/speech-to-text"))
                    .multipart(form),
            )
            .await?;
        let data: Value = response
            .json()
            .await
            .map_err(|e| VoiceError::Unreachable(e.to_string()))?;

        Ok(Transcript {
            text: data
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            language: data
                .get("language_code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
        })
    }
}

/// Percent-encodes a path segment. Voice ids are opaque strings from
/// ElevenLabs, so one is not assumed to be URL-safe.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The decoded size of base64, without decoding it.
///
/// Allocating an eight-megabyte buffer only to measure it and throw it away is
/// the one thing a size guard exists to avoid, and the arithmetic is exact.
pub fn decoded_size(base64_text: &str) -> usize {
    let text = base64_text.trim();
    if text.is_empty() {
        return 0;
    }
    let padding = if text.ends_with("==") {
        2
    } else if text.ends_with('=') {
        1
    } else {
        0
    };
    (text.len() * 3 / 4).saturating_sub(padding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn server() -> MockServer {
        MockServer::start().await
    }

    fn client(server: &MockServer) -> ElevenLabs {
        ElevenLabs::with_base_url(server.uri(), "xi-test")
    }

    #[tokio::test]
    async fn the_quota_comes_back_with_the_account() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path("/v1/user/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "tier": "creator",
                "character_count": 4000,
                "character_limit": 100000,
            })))
            .mount(&server)
            .await;

        let account = client(&server).whoami().await.unwrap();
        assert_eq!(account.tier, "creator");
        assert_eq!(account.remaining, 96000);
    }

    /// A spent account must not report a negative allowance.
    #[tokio::test]
    async fn an_overspent_quota_floors_at_nothing_left() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path("/v1/user/subscription"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "character_count": 120,
                "character_limit": 100,
            })))
            .mount(&server)
            .await;

        let account = client(&server).whoami().await.unwrap();
        assert_eq!(account.remaining, 0);
        assert_eq!(account.tier, "free");
    }

    #[tokio::test]
    async fn voices_carry_the_labels_elevenlabs_wrote() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path("/v1/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "voices": [
                    { "voice_id": "v1", "name": "Rachel", "category": "premade",
                      "labels": { "accent": "american", "description": "calm", "age": "young" } },
                    { "voice_id": "", "name": "Broken" },
                    { "name": "No id at all" },
                ]
            })))
            .mount(&server)
            .await;

        let voices = client(&server).voices().await.unwrap();
        assert_eq!(voices.len(), 1, "rows without an id are not voices");
        assert_eq!(voices[0].tone, "american, calm, young");
    }

    /// The pane must not offer a model that cannot speak.
    #[tokio::test]
    async fn only_models_that_do_speech_are_listed() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "model_id": "eleven_flash_v2_5", "name": "Flash", "can_do_text_to_speech": true },
                { "model_id": "scribe_v1", "name": "Scribe", "can_do_text_to_speech": false },
            ])))
            .mount(&server)
            .await;

        let models = client(&server).models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "eleven_flash_v2_5");
    }

    #[tokio::test]
    async fn speech_comes_back_as_base64_mp3() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/v1/text-to-speech/v1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(b"ID3fake".to_vec(), "audio/mpeg"),
            )
            .mount(&server)
            .await;

        let speech = client(&server)
            .speak(&SpeakRequest {
                text: "hello".into(),
                voice_id: "v1".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(speech.mime_type, "audio/mpeg");
        assert_eq!(speech.size, 7);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&speech.bytes)
            .unwrap();
        assert_eq!(decoded, b"ID3fake");
    }

    /// Refused before a request is made: an empty string would spend a call and
    /// come back with ElevenLabs' own complaint instead of ours.
    #[tokio::test]
    async fn there_is_nothing_to_say_is_caught_here() {
        let server = server().await;
        let error = client(&server)
            .speak(&SpeakRequest {
                text: "   ".into(),
                voice_id: "v1".into(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(error, VoiceError::Invalid(_)));
    }

    #[tokio::test]
    async fn a_rejected_key_says_what_to_do_about_it() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path("/v1/user/subscription"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "detail": { "message": "Invalid API key" }
            })))
            .mount(&server)
            .await;

        let error = client(&server).whoami().await.unwrap_err();
        let text = error.to_string();
        assert!(text.contains("Settings, Secrets"), "{text}");
        // Their words, kept: they are usually more specific than ours.
        assert!(text.contains("Invalid API key"), "{text}");
    }

    #[tokio::test]
    async fn a_spent_quota_is_named_as_one() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path("/v1/voices"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;

        let text = client(&server).voices().await.unwrap_err().to_string();
        assert!(text.contains("quota"), "{text}");
    }

    #[tokio::test]
    async fn transcription_reads_the_text_and_the_language() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path("/v1/speech-to-text"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "text": "  hello there  ",
                "language_code": "en",
            })))
            .mount(&server)
            .await;

        let audio = base64::engine::general_purpose::STANDARD.encode(b"fake audio");
        let transcript = client(&server)
            .transcribe(&audio, "audio/webm", None, None)
            .await
            .unwrap();
        assert_eq!(transcript.text, "hello there");
        assert_eq!(transcript.language, "en");
    }

    #[tokio::test]
    async fn an_empty_recording_is_refused_before_it_is_sent() {
        let server = server().await;
        let error = client(&server)
            .transcribe("", "audio/webm", None, None)
            .await
            .unwrap_err();
        assert!(matches!(error, VoiceError::Invalid(_)));
    }

    #[test]
    fn base64_is_measured_without_being_decoded() {
        let payload = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 1000]);
        assert_eq!(decoded_size(&payload), 1000);
        assert_eq!(decoded_size(""), 0);
    }

    #[test]
    fn a_voice_id_is_not_assumed_to_be_url_safe() {
        assert_eq!(urlencode("a/b c"), "a%2Fb%20c");
        assert_eq!(urlencode("v1-_.~"), "v1-_.~");
    }
}
