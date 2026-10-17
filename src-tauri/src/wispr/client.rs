//! One-shot client for the Wispr voice-actions realtime socket: send a whole
//! recording, return the final text. Final-result events only; partials,
//! context and tool calls are not used.

use super::codec::encode_frames;
use super::token::{self, AccessToken, TokenError};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

pub const ENDPOINT: &str = "wss://api.wisprflow.ai/api/v1/voice-actions/realtime";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);
const INSTRUCTIONS: &str = "Transcribe exactly what I say, verbatim.";

#[derive(Debug)]
pub enum WisprError {
    Token(TokenError),
    Auth,
    Connect(String),
    Timeout(&'static str),
    Server {
        code: Option<String>,
        message: String,
    },
    Protocol(String),
}

impl fmt::Display for WisprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WisprError::Token(e) => write!(f, "{e}"),
            WisprError::Auth => f.write_str("Wispr rejected the session token (HTTP 401/403)"),
            WisprError::Connect(e) => write!(f, "Could not reach Wispr: {e}"),
            WisprError::Timeout(stage) => write!(f, "Wispr timed out while {stage}"),
            WisprError::Server { code, message } => match code {
                Some(code) => write!(f, "Wispr error {code}: {message}"),
                None => write!(f, "Wispr error: {message}"),
            },
            WisprError::Protocol(e) => write!(f, "Unexpected Wispr response: {e}"),
        }
    }
}

impl std::error::Error for WisprError {}

impl From<TokenError> for WisprError {
    fn from(e: TokenError) -> Self {
        WisprError::Token(e)
    }
}

fn connect_error(e: WsError) -> WisprError {
    match e {
        WsError::Http(resp) if matches!(resp.status().as_u16(), 401 | 403) => WisprError::Auth,
        other => WisprError::Connect(other.to_string()),
    }
}

/// Transcribe 16 kHz mono `f32` samples with the signed-in Wispr session.
pub async fn transcribe(samples: Vec<f32>) -> Result<String, WisprError> {
    let token = token::load()?;
    transcribe_with(ENDPOINT, &token, &samples, IDLE_TIMEOUT).await
}

async fn transcribe_with(
    url: &str,
    token: &AccessToken,
    samples: &[f32],
    idle: Duration,
) -> Result<String, WisprError> {
    let frames = encode_frames(samples);
    if frames.is_empty() {
        return Ok(String::new());
    }

    let mut request = url
        .into_client_request()
        .map_err(|e| WisprError::Connect(e.to_string()))?;
    let protocols = HeaderValue::from_str(&format!("access_token, {}", token.expose()))
        .map_err(|_| WisprError::Token(TokenError::Malformed))?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", protocols);

    let (mut ws, _) = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| WisprError::Timeout("connecting"))?
        .map_err(connect_error)?;

    let result = run_session(&mut ws, frames, idle).await;
    // Best effort: the server may already have closed the socket.
    let _ = ws
        .send(json_message(json!({"type": "session.close"})))
        .await;
    let _ = ws.close(None).await;
    result
}

fn json_message(value: Value) -> Message {
    Message::text(value.to_string())
}

async fn send<S>(ws: &mut S, value: Value) -> Result<(), WisprError>
where
    S: futures_util::Sink<Message, Error = WsError> + Unpin,
{
    ws.send(json_message(value))
        .await
        .map_err(|e| WisprError::Connect(e.to_string()))
}

/// Next JSON event, or `None` once the server closes. Non-text frames are skipped.
async fn next_event<S>(
    ws: &mut S,
    idle: Duration,
    stage: &'static str,
) -> Result<Option<Value>, WisprError>
where
    S: futures_util::Stream<Item = Result<Message, WsError>> + Unpin,
{
    loop {
        let frame = timeout(idle, ws.next())
            .await
            .map_err(|_| WisprError::Timeout(stage))?;
        match frame {
            None | Some(Ok(Message::Close(_))) => return Ok(None),
            Some(Err(e)) => return Err(WisprError::Connect(e.to_string())),
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text)
                    .map(Some)
                    .map_err(|_| WisprError::Protocol("non-JSON text frame".into()));
            }
            Some(Ok(_)) => {}
        }
    }
}

fn server_error(event: &Value) -> WisprError {
    WisprError::Server {
        code: event.get("code").and_then(Value::as_str).map(str::to_owned),
        message: event
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_owned(),
    }
}

async fn run_session<S>(
    ws: &mut S,
    frames: Vec<String>,
    idle: Duration,
) -> Result<String, WisprError>
where
    S: futures_util::Stream<Item = Result<Message, WsError>>
        + futures_util::Sink<Message, Error = WsError>
        + Unpin,
{
    send(
        ws,
        json!({
            "type": "session.start",
            "instructions": INSTRUCTIONS,
            "tools": [],
            "context": {"appName": "handy"},
        }),
    )
    .await?;

    loop {
        match next_event(ws, idle, "starting the session").await? {
            None => return Err(WisprError::Protocol("closed before session.ready".into())),
            Some(e) => match e["type"].as_str() {
                Some("session.ready") => break,
                Some("error") => return Err(server_error(&e)),
                _ => {}
            },
        }
    }

    for audio in frames {
        send(
            ws,
            json!({
                "type": "audio.append",
                "audio": audio,
                "encoding": "pcm16",
                "sample_rate": 24000,
                "channels": 1,
            }),
        )
        .await?;
    }
    send(ws, json!({"type": "audio.commit"})).await?;

    let mut text: Option<String> = None;
    loop {
        match next_event(ws, idle, "transcribing").await? {
            None => break,
            Some(e) => match e["type"].as_str() {
                Some("text.final" | "transcript.final") => {
                    text = e["text"].as_str().map(str::to_owned);
                }
                Some("done") => break,
                Some("error") => return Err(server_error(&e)),
                _ => {}
            },
        }
    }
    text.ok_or_else(|| WisprError::Protocol("session ended without final text".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    const TEST_SECRET: &str = "test-secret-jwt";

    /// Mock server: checks the subprotocol, records client event types, then
    /// replays `script` (one text frame per entry) after `session.start`.
    /// `None` entries make the server close the socket.
    #[allow(clippy::result_large_err)]
    async fn mock_server(
        script: Vec<Option<Value>>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let check = |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
                let offered = req.headers()["sec-websocket-protocol"].to_str().unwrap();
                assert_eq!(offered, format!("access_token, {TEST_SECRET}"));
                resp.headers_mut()
                    .insert("sec-websocket-protocol", "access_token".parse().unwrap());
                Ok(resp)
            };
            let mut ws = tokio_tungstenite::accept_hdr_async(stream, check)
                .await
                .unwrap();
            let mut seen = Vec::new();
            let mut script = script.into_iter();
            while let Some(Ok(msg)) = ws.next().await {
                let Message::Text(text) = msg else { continue };
                let event: Value = serde_json::from_str(&text).unwrap();
                let kind = event["type"].as_str().unwrap().to_string();
                seen.push(kind.clone());
                match kind.as_str() {
                    "session.start" | "audio.commit" => {
                        // session.start answers the script's first entries up to
                        // and including session.ready; commit answers the rest.
                        for step in script.by_ref() {
                            match step {
                                None => {
                                    let _ = ws.close(None).await;
                                    return seen;
                                }
                                Some(v) => {
                                    let ready = v["type"] == "session.ready";
                                    ws.send(Message::text(v.to_string())).await.unwrap();
                                    if ready && kind == "session.start" {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    "session.close" => break,
                    _ => {}
                }
            }
            seen
        });
        (url, handle)
    }

    fn token() -> AccessToken {
        AccessToken(TEST_SECRET.to_string())
    }

    fn audio() -> Vec<f32> {
        vec![0.1; 16_000]
    }

    #[tokio::test]
    async fn happy_path_orders_events_and_returns_text() {
        let (url, server) = mock_server(vec![
            Some(json!({"type": "session.ready", "provider": "openai", "model": "gpt-realtime-2"})),
            Some(json!({"type": "text.delta", "text": "hel"})),
            Some(json!({"type": "text.final", "text": "hello world"})),
            Some(json!({"type": "done"})),
        ])
        .await;
        let text = transcribe_with(&url, &token(), &audio(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(text, "hello world");
        assert_eq!(
            server.await.unwrap(),
            [
                "session.start",
                "audio.append",
                "audio.commit",
                "session.close"
            ]
        );
    }

    #[tokio::test]
    async fn large_audio_is_split_into_ordered_frames() {
        let (url, server) = mock_server(vec![
            Some(json!({"type": "session.ready"})),
            Some(json!({"type": "text.final", "text": "ok"})),
            Some(json!({"type": "done"})),
        ])
        .await;
        // 10 s @16k -> 480 000 bytes @24k PCM16 -> 5 frames of <= 96 000.
        transcribe_with(&url, &token(), &vec![0.1; 160_000], Duration::from_secs(5))
            .await
            .unwrap();
        let seen = server.await.unwrap();
        assert_eq!(seen.iter().filter(|k| *k == "audio.append").count(), 5);
        assert_eq!(seen.first().map(String::as_str), Some("session.start"));
        assert_eq!(seen[seen.len() - 2], "audio.commit");
    }

    #[tokio::test]
    async fn server_error_is_reported() {
        let (url, _server) = mock_server(vec![
            Some(json!({"type": "session.ready"})),
            Some(json!({"type": "error", "code": "rate_limited", "message": "slow down"})),
        ])
        .await;
        let err = transcribe_with(&url, &token(), &audio(), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(&err, WisprError::Server { code: Some(c), .. } if c == "rate_limited"));
    }

    #[tokio::test]
    async fn close_before_final_text_is_a_protocol_error() {
        let (url, _server) = mock_server(vec![Some(json!({"type": "session.ready"})), None]).await;
        let err = transcribe_with(&url, &token(), &audio(), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, WisprError::Protocol(_)), "{err}");
    }

    #[tokio::test]
    async fn silent_server_times_out() {
        // Never sends session.ready.
        let (url, _server) = mock_server(vec![]).await;
        let err = transcribe_with(&url, &token(), &audio(), Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(matches!(err, WisprError::Timeout(_)), "{err}");
    }

    #[tokio::test]
    async fn empty_recording_makes_no_connection() {
        let text = transcribe_with("ws://127.0.0.1:1", &token(), &[], Duration::from_secs(1))
            .await
            .unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn errors_never_contain_the_token() {
        let err = WisprError::Connect("refused".into());
        assert!(!err.to_string().contains(TEST_SECRET));
    }
}
