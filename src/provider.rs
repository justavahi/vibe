//! A thin abstraction over the supported LLM providers.
//!
//! Both providers expose an OpenAI-compatible chat API. OpenRouter is reached
//! through the `openrouter` crate; Venice AI is reached with a plain HTTP
//! client.

use openrouter::OpenRouter;
use openrouter::completions::Request;
use openrouter::completions::request::{Content, Message};

use crate::config::Vendor;
use crate::session::HistoryEntry;

/// Venice AI API base URL.
pub const VENICE_BASE: &str = "https://api.venice.ai/api/v1";

/// A model offered by a provider.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// The model id used in requests.
    pub id: String,
    /// A human-readable name, if the provider supplies one.
    pub name: String,
}

/// A provider client, built from the selected vendor and its API key.
pub enum Provider {
    /// OpenRouter, via the `openrouter` crate.
    OpenRouter { client: OpenRouter },
    /// Venice AI, via plain HTTP.
    Venice {
        http: reqwest::Client,
        api_key: String,
    },
}

impl Provider {
    /// Builds a provider client for `vendor` using `api_key`.
    pub fn new(vendor: Vendor, api_key: &str) -> Self {
        match vendor {
            Vendor::OpenRouter => Provider::OpenRouter {
                client: OpenRouter::new(api_key),
            },
            Vendor::Venice => Provider::Venice {
                http: reqwest::Client::new(),
                api_key: api_key.to_string(),
            },
        }
    }

    /// Lists the models offered by this provider, sorted by id.
    pub async fn models(&self) -> Result<Vec<ModelInfo>, String> {
        match self {
            Provider::OpenRouter { client } => {
                let mut models = client
                    .models(&[])
                    .await
                    .map_err(|e| describe_openrouter_error(&e))?;

                models.sort_by(|a, b| a.id.cmp(&b.id));

                Ok(models
                    .into_iter()
                    .map(|m| ModelInfo {
                        id: m.id,
                        name: m.name,
                    })
                    .collect())
            }
            Provider::Venice { http, api_key } => {
                let url = format!("{VENICE_BASE}/models");
                let body = get_json(http, &url, api_key).await?;

                let response: VeniceModels = serde_json::from_str(&body)
                    .map_err(|e| format!("failed to parse Venice models: {e}"))?;

                let mut models: Vec<ModelInfo> = response
                    .data
                    .into_iter()
                    .map(|m| {
                        let name = m
                            .name
                            .or_else(|| m.model_spec.and_then(|s| s.name))
                            .unwrap_or_else(|| m.id.clone());
                        ModelInfo { id: m.id, name }
                    })
                    .collect();

                models.sort_by(|a, b| a.id.cmp(&b.id));
                Ok(models)
            }
        }
    }

    /// Sends the full conversation `history` to `model` and returns the
    /// assistant's reply.
    pub async fn chat(&self, model: &str, history: &[HistoryEntry]) -> Result<String, String> {
        match self {
            Provider::OpenRouter { client } => {
                let request = Request {
                    model: Some(model.to_string()),
                    messages: Some(build_messages(history)),
                    ..Request::default()
                };

                let response = client
                    .chat_completion(request)
                    .await
                    .map_err(|e| describe_openrouter_error(&e))?;

                response
                    .choices
                    .first()
                    .and_then(extract_openrouter_text)
                    .ok_or_else(|| "provider returned an empty response".to_string())
            }
            Provider::Venice { http, api_key } => {
                let url = format!("{VENICE_BASE}/chat/completions");
                let messages: Vec<serde_json::Value> = history
                    .iter()
                    .map(|e| serde_json::json!({ "role": e.role, "content": e.content }))
                    .collect();

                let body = serde_json::json!({ "model": model, "messages": messages });
                let response = post_json(http, &url, api_key, &body).await?;

                let parsed: VeniceChat = serde_json::from_str(&response)
                    .map_err(|e| format!("failed to parse Venice response: {e}"))?;

                parsed
                    .choices
                    .into_iter()
                    .next()
                    .and_then(|c| c.message.content)
                    .ok_or_else(|| "provider returned an empty response".to_string())
            }
        }
    }
}

/// Performs an authenticated `GET` and returns the body, or a readable error.
async fn get_json(http: &reqwest::Client, url: &str, api_key: &str) -> Result<String, String> {
    let response = http
        .get(url)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        return Err(format!("Venice error {}: {}", status.as_u16(), body.trim()));
    }

    Ok(body)
}

/// Performs an authenticated `POST` with a JSON body and returns the body.
async fn post_json(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<String, String> {
    let response = http
        .post(url)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        return Err(format!("Venice error {}: {}", status.as_u16(), text.trim()));
    }

    Ok(text)
}

/// Turns stored history into OpenRouter messages.
fn build_messages(history: &[HistoryEntry]) -> Vec<Message> {
    history
        .iter()
        .filter_map(|entry| match entry.role.as_str() {
            "user" => Some(Message::User {
                content: Content::Plain(entry.content.clone()),
                name: None,
                cache_control: None,
            }),
            "assistant" => Some(Message::Assistant {
                content: Some(Content::Plain(entry.content.clone())),
                name: None,
                tool_calls: None,
            }),
            _ => None,
        })
        .collect()
}

/// Pulls the assistant text out of an OpenRouter completion choice.
fn extract_openrouter_text(choice: &openrouter::completions::response::Choice) -> Option<String> {
    use openrouter::completions::response::Choice;

    match choice {
        Choice::NonStreaming(c) => c.message.content.clone(),
        Choice::NonChat(c) => Some(c.text.clone()),
        Choice::Streaming(c) => c.delta.content.clone(),
    }
}

/// Formats an OpenRouter error with the underlying code/message, which the
/// library's `Display` impl otherwise hides.
pub fn describe_openrouter_error(error: &openrouter::Error) -> String {
    match error {
        openrouter::Error::OpenRouter(e) => {
            format!("OpenRouter error {}: {}", e.code, e.message)
        }
        other => other.to_string(),
    }
}

#[derive(serde::Deserialize)]
struct VeniceModels {
    #[serde(default)]
    data: Vec<VeniceModel>,
}

#[derive(serde::Deserialize)]
struct VeniceModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model_spec: Option<VeniceModelSpec>,
}

#[derive(serde::Deserialize)]
struct VeniceModelSpec {
    #[serde(default)]
    name: Option<String>,
}

#[derive(serde::Deserialize)]
struct VeniceChat {
    #[serde(default)]
    choices: Vec<VeniceChoice>,
}

#[derive(serde::Deserialize)]
struct VeniceChoice {
    message: VeniceMessage,
}

#[derive(serde::Deserialize)]
struct VeniceMessage {
    #[serde(default)]
    content: Option<String>,
}
