//! A synchronous A2A client: discovers Agent Cards and tasks
//! external agents over the JSON-RPC binding.
//!
//! The transport posture mirrors phlow-runtime's rules for outbound
//! calls, because an A2A client is exactly that: proxies are
//! disabled (endpoint resolution is explicit), redirects are not
//! followed (a redirect would silently change the peer), TLS is on
//! via rustls with webpki roots, every call has a timeout, and
//! response bodies are read under [`RESPONSE_BYTES_MAX`] with the
//! read itself capped, not just checked after the fact.

use std::io::{BufRead, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::types::{
    AgentCard, GetTaskParams, ListTasksParams, ListTasksResult, Message, SendMessageParams,
    SendStreamingResponse, Task, TaskIdParams,
};
use crate::util::boot_seed;
use crate::{AGENT_CARD_PATH, RESPONSE_BYTES_MAX};

/// Client configuration beyond the endpoint.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Bearer token to present, when the peer requires one.
    pub token: Option<String>,
    /// Overall request timeout. Default 30 s.
    pub timeout: Duration,
    /// Connection timeout. Default 10 s.
    pub connect_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            token: None,
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

/// Everything a client call can fail with.
#[derive(Debug)]
pub enum A2aError {
    /// The peer answered with a JSON-RPC error.
    Rpc {
        /// The JSON-RPC error code.
        code: i64,
        /// The peer's message.
        message: String,
        /// The peer's structured error data, when present.
        data: Option<serde_json::Value>,
    },
    /// The transport failed (DNS, connect, TLS, timeout, HTTP
    /// status, oversized response).
    Transport(String),
    /// The peer's answer was not well-formed A2A.
    Protocol(String),
}

impl std::fmt::Display for A2aError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            A2aError::Rpc { code, message, .. } => {
                write!(formatter, "A2A error {code}: {message}")
            }
            A2aError::Transport(detail) => write!(formatter, "transport: {detail}"),
            A2aError::Protocol(detail) => write!(formatter, "protocol: {detail}"),
        }
    }
}

impl std::error::Error for A2aError {}

/// A client bound to one agent's interface URL.
pub struct A2aClient {
    interface_url: String,
    card_url: String,
    http: reqwest::blocking::Client,
    token: Option<String>,
    next_id: AtomicU64,
}

impl A2aClient {
    /// Build a client for the given interface URL (the URL an
    /// Agent Card's `supportedInterfaces` entry names).
    ///
    /// # Errors
    ///
    /// [`A2aError::Transport`] if the HTTP client cannot be built
    /// or the URL has no derivable origin for card discovery.
    pub fn new(interface_url: &str, config: ClientConfig) -> Result<A2aClient, A2aError> {
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|err| A2aError::Transport(err.to_string()))?;
        let card_url = derive_card_url(interface_url)?;
        Ok(A2aClient {
            interface_url: interface_url.to_string(),
            card_url,
            http,
            token: config.token,
            next_id: AtomicU64::new(boot_seed("phlow-a2a-client") & 0xFFFF),
        })
    }

    /// Fetch and parse the peer's Agent Card from the well-known
    /// location. The card is inert data: callers must not treat
    /// anything in it as a capability grant.
    pub fn fetch_card(&self) -> Result<AgentCard, A2aError> {
        let mut request = self.http.get(&self.card_url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let mut response = request
            .send()
            .map_err(|err| A2aError::Transport(err.to_string()))?;
        if !response.status().is_success() {
            return Err(A2aError::Transport(format!(
                "card fetch returned HTTP {}",
                response.status()
            )));
        }
        let body = read_capped(&mut response)?;
        serde_json::from_slice(&body)
            .map_err(|err| A2aError::Protocol(format!("agent card did not parse: {err}")))
    }

    /// `SendMessage`: submit a message and wait for the resulting
    /// task (or the peer's task snapshot when its request timeout
    /// returns one still in flight).
    pub fn send_message(&self, message: &Message) -> Result<Task, A2aError> {
        message.validate().map_err(A2aError::Protocol)?;
        let params = SendMessageParams {
            message: message.clone(),
            configuration: None,
            metadata: None,
        };
        let value = self.rpc("SendMessage", &params)?;
        serde_json::from_value(value)
            .map_err(|err| A2aError::Protocol(format!("SendMessage result did not parse: {err}")))
    }

    /// `SendStreamingMessage`: submit and stream updates into
    /// `on_event` until the peer closes the stream (at a terminal
    /// or input-required state). Returns the number of events seen.
    pub fn send_streaming(
        &self,
        message: &Message,
        on_event: &mut dyn FnMut(SendStreamingResponse),
    ) -> Result<usize, A2aError> {
        message.validate().map_err(A2aError::Protocol)?;
        let params = SendMessageParams {
            message: message.clone(),
            configuration: None,
            metadata: None,
        };
        self.stream("SendStreamingMessage", &params, on_event)
    }

    /// `SubscribeToTask`: stream updates for an existing task.
    pub fn subscribe(
        &self,
        task_id: &str,
        on_event: &mut dyn FnMut(SendStreamingResponse),
    ) -> Result<usize, A2aError> {
        let params = TaskIdParams {
            id: task_id.to_string(),
            metadata: None,
        };
        self.stream("SubscribeToTask", &params, on_event)
    }

    /// `GetTask`.
    pub fn get_task(&self, id: &str, history_length: Option<u32>) -> Result<Task, A2aError> {
        let params = GetTaskParams {
            id: id.to_string(),
            history_length,
        };
        let value = self.rpc("GetTask", &params)?;
        serde_json::from_value(value)
            .map_err(|err| A2aError::Protocol(format!("GetTask result did not parse: {err}")))
    }

    /// `ListTasks`.
    pub fn list_tasks(&self, params: &ListTasksParams) -> Result<ListTasksResult, A2aError> {
        let value = self.rpc("ListTasks", params)?;
        serde_json::from_value(value)
            .map_err(|err| A2aError::Protocol(format!("ListTasks result did not parse: {err}")))
    }

    /// `CancelTask`. A peer's refusal (terminal task) surfaces as
    /// [`A2aError::Rpc`] with code -32002.
    pub fn cancel_task(&self, id: &str) -> Result<Task, A2aError> {
        let params = TaskIdParams {
            id: id.to_string(),
            metadata: None,
        };
        let value = self.rpc("CancelTask", &params)?;
        serde_json::from_value(value)
            .map_err(|err| A2aError::Protocol(format!("CancelTask result did not parse: {err}")))
    }

    /// One JSON-RPC round trip returning the `result` value.
    fn rpc<P: serde::Serialize>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<serde_json::Value, A2aError> {
        let mut response = self.post_rpc(method, params)?;
        if !response.status().is_success() {
            return Err(A2aError::Transport(format!(
                "{method} returned HTTP {}",
                response.status()
            )));
        }
        let body = read_capped(&mut response)?;
        let value: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|err| A2aError::Protocol(format!("response was not JSON: {err}")))?;
        unwrap_result(value)
    }

    /// One streaming round trip: POST the call, consume SSE events.
    fn stream<P: serde::Serialize>(
        &self,
        method: &str,
        params: &P,
        on_event: &mut dyn FnMut(SendStreamingResponse),
    ) -> Result<usize, A2aError> {
        let response = self.post_rpc(method, params)?;
        if !response.status().is_success() {
            return Err(A2aError::Transport(format!(
                "{method} returned HTTP {}",
                response.status()
            )));
        }
        let mut reader = std::io::BufReader::new(response);
        let mut events = 0usize;
        let mut data_lines: Vec<String> = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let read = read_line_capped(&mut reader, &mut line)?;
            if read == 0 {
                break; // stream closed
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                if !data_lines.is_empty() {
                    let payload = data_lines.join("\n");
                    data_lines.clear();
                    let value: serde_json::Value =
                        serde_json::from_str(&payload).map_err(|err| {
                            A2aError::Protocol(format!("SSE payload was not JSON: {err}"))
                        })?;
                    let result = unwrap_result(value)?;
                    let event: SendStreamingResponse =
                        serde_json::from_value(result).map_err(|err| {
                            A2aError::Protocol(format!("SSE event did not parse: {err}"))
                        })?;
                    events += 1;
                    on_event(event);
                }
                continue;
            }
            if let Some(data) = trimmed.strip_prefix("data:") {
                data_lines.push(data.trim_start().to_string());
            }
            // Other SSE fields (event:, id:, retry:) are ignored by
            // design: the JSON-RPC envelope carries everything.
        }
        Ok(events)
    }

    fn post_rpc<P: serde::Serialize>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<reqwest::blocking::Response, A2aError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut request = self
            .http
            .post(&self.interface_url)
            .header("A2A-Version", crate::PROTOCOL_VERSION)
            .json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        request
            .send()
            .map_err(|err| A2aError::Transport(err.to_string()))
    }
}

/// Extract `result` or convert the envelope's `error`.
fn unwrap_result(value: serde_json::Value) -> Result<serde_json::Value, A2aError> {
    if let Some(error) = value.get("error") {
        return Err(A2aError::Rpc {
            code: error
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-32603),
            message: error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
            data: error.get("data").cloned(),
        });
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| A2aError::Protocol("response carried neither result nor error".to_string()))
}

/// Read a response body under the cap — the read itself is capped
/// (`take`), so a lying or absent content-length cannot flood us.
fn read_capped(response: &mut reqwest::blocking::Response) -> Result<Vec<u8>, A2aError> {
    let mut body = Vec::new();
    response
        .take(RESPONSE_BYTES_MAX + 1)
        .read_to_end(&mut body)
        .map_err(|err| A2aError::Transport(format!("reading response: {err}")))?;
    if body.len() as u64 > RESPONSE_BYTES_MAX {
        return Err(A2aError::Transport(format!(
            "response exceeded the {RESPONSE_BYTES_MAX}-byte cap"
        )));
    }
    Ok(body)
}

/// Read one SSE line with a sanity cap (a single event cannot be a
/// flood vector either).
fn read_line_capped(reader: &mut impl BufRead, line: &mut String) -> Result<usize, A2aError> {
    let read = reader
        .take(1_048_576)
        .read_line(line)
        .map_err(|err| A2aError::Transport(format!("reading stream: {err}")))?;
    Ok(read)
}

/// The well-known card URL for an interface URL:
/// `<scheme>://<authority>/.well-known/agent-card.json`.
fn derive_card_url(interface_url: &str) -> Result<String, A2aError> {
    let (scheme, rest) = interface_url.split_once("://").ok_or_else(|| {
        A2aError::Transport(format!("interface URL has no scheme: {interface_url}"))
    })?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err(A2aError::Transport(format!(
            "interface URL has no authority: {interface_url}"
        )));
    }
    Ok(format!("{scheme}://{authority}{AGENT_CARD_PATH}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_url_derivation_uses_the_origin() {
        assert_eq!(
            derive_card_url("https://agents.example.com/a2a/v1").expect("derive"),
            "https://agents.example.com/.well-known/agent-card.json"
        );
        assert_eq!(
            derive_card_url("http://127.0.0.1:8443/").expect("derive"),
            "http://127.0.0.1:8443/.well-known/agent-card.json"
        );
        assert!(derive_card_url("agents.example.com").is_err());
    }

    #[test]
    fn rpc_errors_unwrap_with_code_and_message() {
        let envelope = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": -32001, "message": "task not found: x"}
        });
        match unwrap_result(envelope) {
            Err(A2aError::Rpc { code, message, .. }) => {
                assert_eq!(code, -32001);
                assert!(message.contains("task not found"));
            }
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }
}
