//! Real Anthropic Messages API backend with SSE streaming.
//!
//! This module implements [`ChatBackend`] by making HTTP requests to the
//! Anthropic Messages API (`https://api.anthropic.com/v1/messages`). It
//! uses reqwest with rustls for TLS (no system OpenSSL dependency) and
//! parses Server-Sent Events (SSE) from the streaming response.
//!
//! ## Authentication
//!
//! The backend reads credentials from `~/.claude/.credentials.json`,
//! which is the same file Claude Code uses. This means openclank shares
//! the user's Claude Code subscription rather than requiring a separate
//! API key. The credential file contains an OAuth access token in the
//! `claudeAiOauth.accessToken` field.
//!
//! OAuth tokens require Bearer authentication and a specific set of
//! headers including beta flags and a billing signature injected into
//! the system prompt. This is the same protocol that Claude Code itself
//! uses.
//!
//! Alternatively, a standard API key can be passed via the
//! `ANTHROPIC_API_KEY` environment variable, which takes precedence
//! and uses the simpler `x-api-key` header without billing signatures.
//!
//! ## SSE Parsing
//!
//! The Anthropic streaming API returns Server-Sent Events with event
//! types like `content_block_delta`, `content_block_start`, and
//! `message_stop`. Each event has a JSON `data` field. We parse these
//! into [`BackendEvent`]s that the runner feeds into the update function.

use futures::stream::{self, BoxStream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::state::message::{ContentBlock, Message, Role, ToolName, ToolUseId};

use super::traits::{BackendError, BackendEvent, ChatBackend};

/// The Anthropic Messages API endpoint.
const API_URL: &str = "https://api.anthropic.com/v1/messages";

/// The API version header value.
const API_VERSION: &str = "2023-06-01";

/// Beta flags required for OAuth authentication. These must be sent in the
/// `anthropic-beta` header for the API to accept OAuth tokens.
const BETA_FLAGS: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,prompt-caching-scope-2026-01-05,context-management-2025-06-27";

/// The Claude Code version string used in the billing header and user-agent.
/// This should be kept roughly in sync with current Claude Code releases.
const CC_VERSION: &str = "2.1.90";

/// The system identity string that Claude Code injects into every request.
/// Required by the API for OAuth-authenticated requests.
const SYSTEM_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Salt used in the billing header hash computation.
const BILLING_SALT: &str = "59cf53e54c78";

/// Whether this token is an OAuth token (from Claude Code) or a standard
/// API key (from the Anthropic Console).
enum AuthMode {
    /// OAuth token: uses Bearer auth, billing headers, system identity.
    OAuth(String),
    /// Standard API key: uses x-api-key header, no special headers needed.
    ApiKey(String),
}

/// A configured Anthropic API client.
///
/// Holds the HTTP client, credentials, and model name. The HTTP client is
/// reused across requests for connection pooling.
pub struct AnthropicBackend {
    /// The reqwest HTTP client, configured with rustls.
    client: Client,
    /// How to authenticate: OAuth (Bearer + billing) or API key.
    auth: AuthMode,
    /// The model to use (e.g. "claude-sonnet-4-20250514").
    model: String,
    /// An optional user-provided system prompt appended after the
    /// required identity and billing entries.
    system_prompt: Option<String>,
}

impl AnthropicBackend {
    /// Create a new backend. The `api_key` is inspected to determine
    /// auth mode: tokens starting with `sk-ant-oat` are treated as OAuth
    /// tokens, everything else as a standard API key.
    pub fn new(api_key: String, model: String) -> Self {
        let auth = if api_key.starts_with("sk-ant-oat") {
            AuthMode::OAuth(api_key)
        } else {
            AuthMode::ApiKey(api_key)
        };
        Self {
            client: Client::new(),
            auth,
            model,
            system_prompt: None,
        }
    }

    /// Set a system prompt that will be included in every API request.
    /// For OAuth mode, this is appended after the required identity
    /// and billing header entries.
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }

    /// Load credentials from the Claude Code credentials file at
    /// `~/.claude/.credentials.json`. Returns the OAuth access token
    /// if the file exists and contains valid credentials.
    pub fn load_claude_credentials() -> Result<String, String> {
        let home = dirs::home_dir().ok_or("could not determine home directory")?;
        let cred_path = home.join(".claude").join(".credentials.json");

        let contents = std::fs::read_to_string(&cred_path)
            .map_err(|e| format!("could not read {}: {e}", cred_path.display()))?;

        let creds: CredentialsFile = serde_json::from_str(&contents)
            .map_err(|e| format!("could not parse {}: {e}", cred_path.display()))?;

        Ok(creds.claude_ai_oauth.access_token)
    }

    /// Resolve the API key from (in priority order):
    /// 1. `ANTHROPIC_API_KEY` environment variable
    /// 2. Claude Code credentials file (`~/.claude/.credentials.json`)
    pub fn resolve_api_key() -> Result<String, String> {
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            return Ok(key);
        }

        Self::load_claude_credentials()
    }
}

/// The structure of `~/.claude/.credentials.json`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: OAuthCredentials,
}

/// The OAuth credentials within the credentials file.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthCredentials {
    access_token: String,
}

/// A message in the format the Anthropic API expects.
#[derive(Serialize)]
struct ApiMessage {
    role: &'static str,
    content: Vec<ApiContentBlock>,
}

/// A content block in the format the Anthropic API expects.
#[derive(Serialize)]
#[serde(tag = "type")]
enum ApiContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// A system prompt entry in the format the Anthropic API expects.
#[derive(Serialize)]
struct SystemEntry {
    r#type: &'static str,
    text: String,
}

/// The request body sent to the Anthropic Messages API.
#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    stream: bool,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<SystemEntry>,
}

/// Convert our internal messages to the API's format.
fn to_api_messages(messages: &[Message]) -> Vec<ApiMessage> {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role() {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let content = msg
                .content()
                .iter()
                .map(|block| match block {
                    ContentBlock::Text(text) => ApiContentBlock::Text {
                        text: text.clone(),
                    },
                    ContentBlock::ToolUse { id, name, input } => ApiContentBlock::ToolUse {
                        id: id.as_str().to_string(),
                        name: name.as_str().to_string(),
                        input: input.clone(),
                    },
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.as_str().to_string(),
                        content: content.clone(),
                        is_error: *is_error,
                    },
                })
                .collect();
            ApiMessage { role, content }
        })
        .collect()
}

/// Extract the text of the first user message for billing hash computation.
/// Matches the behavior of Claude Code's billing header generator.
fn extract_first_user_text(messages: &[Message]) -> String {
    for msg in messages {
        if matches!(msg.role(), Role::User) {
            return msg.text();
        }
    }
    String::new()
}

/// Compute the billing header string that must be injected as the first
/// system prompt entry for OAuth-authenticated requests.
///
/// The billing header has the format:
///   `x-anthropic-billing-header: cc_version=V.S; cc_entrypoint=E; cch=H;`
///
/// Where:
/// - `V` is the Claude Code version string (e.g. "2.1.90")
/// - `S` is a 3-character suffix derived from hashing sampled message
///   characters with the billing salt and version
/// - `E` is the entrypoint (always "cli" for us)
/// - `H` is the first 5 hex characters of SHA-256 of the first user
///   message text
fn build_billing_header(first_user_text: &str) -> String {
    use sha2::{Digest, Sha256};

    // cch: first 5 hex chars of SHA-256 of the message text.
    let cch_hash = Sha256::digest(first_user_text.as_bytes());
    let cch = &hex::encode(cch_hash)[..5];

    // Version suffix: sample characters at indices 4, 7, 20 from the
    // message text (using "0" for indices beyond the text length), then
    // hash with the billing salt and version string.
    let chars: Vec<char> = first_user_text.chars().collect();
    let mut sampled = String::with_capacity(3);
    for &idx in &[4, 7, 20] {
        if idx < chars.len() {
            sampled.push(chars[idx]);
        } else {
            sampled.push('0');
        }
    }
    let suffix_input = format!("{BILLING_SALT}{sampled}{CC_VERSION}");
    let suffix_hash = Sha256::digest(suffix_input.as_bytes());
    let suffix = &hex::encode(suffix_hash)[..3];

    format!(
        "x-anthropic-billing-header: cc_version={CC_VERSION}.{suffix}; cc_entrypoint=cli; cch={cch};"
    )
}

/// Build the system prompt entries for an OAuth-authenticated request.
/// The first entry is the billing header, the second is the required
/// Claude Code identity string, and the optional third is the user's
/// custom system prompt.
fn build_oauth_system_entries(
    messages: &[Message],
    user_system_prompt: Option<&str>,
) -> Vec<SystemEntry> {
    let first_user_text = extract_first_user_text(messages);
    let billing = build_billing_header(&first_user_text);

    let mut entries = vec![
        SystemEntry {
            r#type: "text",
            text: billing,
        },
        SystemEntry {
            r#type: "text",
            text: SYSTEM_IDENTITY.to_string(),
        },
    ];

    if let Some(prompt) = user_system_prompt {
        entries.push(SystemEntry {
            r#type: "text",
            text: prompt.to_string(),
        });
    }

    entries
}

impl ChatBackend for AnthropicBackend {
    fn send<'a>(
        &'a self,
        messages: &'a [Message],
    ) -> BoxStream<'a, Result<BackendEvent, BackendError>> {
        let api_messages = to_api_messages(messages);

        // Build system entries depending on auth mode.
        let system = match &self.auth {
            AuthMode::OAuth(_) => {
                build_oauth_system_entries(messages, self.system_prompt.as_deref())
            }
            AuthMode::ApiKey(_) => {
                // For standard API keys, just pass the user's system prompt
                // as a single text entry, if provided.
                match &self.system_prompt {
                    Some(prompt) => vec![SystemEntry {
                        r#type: "text",
                        text: prompt.clone(),
                    }],
                    None => Vec::new(),
                }
            }
        };

        let request_body = ApiRequest {
            model: self.model.clone(),
            max_tokens: 8192,
            stream: true,
            messages: api_messages,
            system,
        };

        let fut = async move {
            // Build the request with auth-mode-specific headers.
            let mut req = self
                .client
                .post(API_URL)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json");

            match &self.auth {
                AuthMode::OAuth(token) => {
                    req = req
                        .header("authorization", format!("Bearer {token}"))
                        .header("anthropic-beta", BETA_FLAGS)
                        .header("x-app", "cli")
                        .header("user-agent", format!("claude-cli/{CC_VERSION} (external, cli)"));
                }
                AuthMode::ApiKey(key) => {
                    req = req.header("x-api-key", key);
                }
            }

            let response = req
                .json(&request_body)
                .send()
                .await
                .map_err(|e| BackendError {
                    message: format!("HTTP request failed: {e}"),
                    retryable: e.is_timeout() || e.is_connect(),
                })?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(BackendError {
                    message: format!("API error {status}: {body}"),
                    retryable: status.as_u16() == 429 || status.is_server_error(),
                });
            }

            // Read the full response body and parse SSE events.
            // A more sophisticated implementation would parse SSE
            // incrementally from the byte stream, but collecting
            // first is simpler and sufficient for now.
            let body = response.text().await.map_err(|e| BackendError {
                message: format!("failed to read response body: {e}"),
                retryable: false,
            })?;

            Ok(parse_sse_events(&body))
        };

        // Convert the future into a stream: first await the result,
        // then yield each BackendEvent from the parsed SSE events.
        stream::once(fut)
            .flat_map(|result| match result {
                Ok(events) => stream::iter(events.into_iter().map(Ok)).boxed(),
                Err(e) => stream::once(async move { Err(e) }).boxed(),
            })
            .boxed()
    }
}

/// Parse a raw SSE response body into a list of [`BackendEvent`]s.
///
/// SSE format: each event is one or more lines starting with `event: `
/// or `data: `, separated by blank lines. We care about:
/// - `content_block_delta` with `text_delta` — text chunk
/// - `content_block_start` with `tool_use` — tool call
/// - `message_stop` — end of response
///
/// Events we don't care about (like `message_start`, `ping`) are ignored.
fn parse_sse_events(body: &str) -> Vec<BackendEvent> {
    let mut events = Vec::new();
    let mut current_event_type = String::new();
    let mut current_data = String::new();

    for line in body.lines() {
        if let Some(event_type) = line.strip_prefix("event: ") {
            current_event_type = event_type.trim().to_string();
            current_data.clear();
        } else if let Some(data) = line.strip_prefix("data: ") {
            current_data = data.trim().to_string();

            // Process the event now that we have both type and data.
            if let Some(event) = parse_single_sse_event(&current_event_type, &current_data) {
                events.push(event);
            }
        }
    }

    // Always end with Done if we got any events, so the update
    // function knows streaming is complete.
    if !events.is_empty() {
        if !matches!(events.last(), Some(BackendEvent::Done)) {
            events.push(BackendEvent::Done);
        }
    }

    events
}

/// Parse a single SSE event (type + data JSON) into a [`BackendEvent`].
/// Returns `None` for event types we don't handle.
fn parse_single_sse_event(event_type: &str, data: &str) -> Option<BackendEvent> {
    match event_type {
        "content_block_delta" => {
            let parsed: serde_json::Value = serde_json::from_str(data).ok()?;
            let delta = parsed.get("delta")?;
            let delta_type = delta.get("type")?.as_str()?;

            match delta_type {
                "text_delta" => {
                    let text = delta.get("text")?.as_str()?;
                    Some(BackendEvent::TextDelta(text.to_string()))
                }
                "input_json_delta" => None,
                _ => None,
            }
        }

        "content_block_start" => {
            let parsed: serde_json::Value = serde_json::from_str(data).ok()?;
            let block = parsed.get("content_block")?;
            let block_type = block.get("type")?.as_str()?;

            if block_type == "tool_use" {
                let id_str = block.get("id")?.as_str()?;
                let id = ToolUseId::new(id_str).ok()?;
                let name_str = block.get("name")?.as_str()?;
                let name = ToolName::from_str(name_str)?;
                let input = block
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                Some(BackendEvent::ToolUse { id, name, input })
            } else {
                None
            }
        }

        "message_stop" => Some(BackendEvent::Done),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_streaming_response() {
        let body = "\
event: message_start\n\
data: {\"type\":\"message_start\"}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

        let events = parse_sse_events(body);

        assert_eq!(events.len(), 3);
        match &events[0] {
            BackendEvent::TextDelta(t) => assert_eq!(t, "Hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
        match &events[1] {
            BackendEvent::TextDelta(t) => assert_eq!(t, " world"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
        assert!(matches!(events[2], BackendEvent::Done));
    }

    #[test]
    fn parse_tool_use_response() {
        let body = "\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_abc123\",\"name\":\"bash\",\"input\":{}}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

        let events = parse_sse_events(body);

        assert_eq!(events.len(), 2);
        match &events[0] {
            BackendEvent::ToolUse { id, name, .. } => {
                assert_eq!(id.as_str(), "toolu_abc123");
                assert_eq!(*name, ToolName::Bash);
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        assert!(matches!(events[1], BackendEvent::Done));
    }

    #[test]
    fn parse_ignores_unknown_events() {
        let body = "\
event: ping\n\
data: {}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

        let events = parse_sse_events(body);

        assert_eq!(events.len(), 2);
        match &events[0] {
            BackendEvent::TextDelta(t) => assert_eq!(t, "hi"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn credentials_file_parsing() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-test","refreshToken":"sk-ant-ort01-test","expiresAt":9999999999999,"scopes":["user:inference"],"subscriptionType":"max","rateLimitTier":"default"}}"#;
        let creds: CredentialsFile = serde_json::from_str(json).unwrap();
        assert_eq!(creds.claude_ai_oauth.access_token, "sk-ant-oat01-test");
    }

    #[test]
    fn billing_header_computation() {
        // Verify the billing header matches the expected format and
        // produces deterministic output for the same input.
        let header = build_billing_header("Reply with exactly the word pong");
        assert!(header.starts_with("x-anthropic-billing-header: cc_version="));
        assert!(header.contains("cc_entrypoint=cli"));
        assert!(header.contains("cch="));

        // Same input should produce the same header.
        let header2 = build_billing_header("Reply with exactly the word pong");
        assert_eq!(header, header2);
    }

    #[test]
    fn oauth_token_detected_by_prefix() {
        let backend = AnthropicBackend::new(
            "sk-ant-oat01-test".to_string(),
            "claude-sonnet-4-20250514".to_string(),
        );
        assert!(matches!(backend.auth, AuthMode::OAuth(_)));
    }

    #[test]
    fn api_key_detected_by_prefix() {
        let backend = AnthropicBackend::new(
            "sk-ant-api03-test".to_string(),
            "claude-sonnet-4-20250514".to_string(),
        );
        assert!(matches!(backend.auth, AuthMode::ApiKey(_)));
    }
}
