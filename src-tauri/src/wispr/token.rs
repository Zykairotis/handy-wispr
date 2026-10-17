//! Reads the Wispr desktop app's session so Handy can reuse its sign-in.
//!
//! Read-only on purpose: the token is never stored or logged, and never
//! refreshed here, because refreshing rotates the refresh token and signs the
//! desktop app out. When it has expired, the Wispr app must refresh it.

use serde::Serialize;
use serde_json::Value;
use specta::Type;
use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bearer token whose `Debug` output never contains the secret.
#[derive(Clone)]
pub struct AccessToken(pub(super) String);

impl AccessToken {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum TokenStatus {
    Ok,
    Expired,
    Missing,
}

#[derive(Debug)]
pub enum TokenError {
    Missing,
    Expired,
    Malformed,
}

impl TokenError {
    pub fn status(&self) -> TokenStatus {
        match self {
            TokenError::Expired => TokenStatus::Expired,
            TokenError::Missing | TokenError::Malformed => TokenStatus::Missing,
        }
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TokenError::Missing => "Wispr Flow session not found; sign in to the Wispr app",
            TokenError::Expired => "Wispr Flow session expired; open the Wispr app to refresh it",
            TokenError::Malformed => "Wispr Flow session file is not in the expected format",
        })
    }
}

impl std::error::Error for TokenError {}

fn session_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("Wispr Flow").join("session.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pure parser: `session.json` maps `sb-<project>-auth-token` to a JSON string
/// that holds `access_token` and `expires_at` (unix seconds).
fn parse_session(raw: &str, now: u64) -> Result<AccessToken, TokenError> {
    let root: Value = serde_json::from_str(raw).map_err(|_| TokenError::Malformed)?;
    let entry = root
        .as_object()
        .and_then(|o| {
            o.iter()
                .find(|(k, _)| k.starts_with("sb-") && k.ends_with("-auth-token"))
        })
        .map(|(_, v)| v)
        .ok_or(TokenError::Missing)?;
    let session: Value = match entry {
        Value::String(s) => serde_json::from_str(s).map_err(|_| TokenError::Malformed)?,
        other => other.clone(),
    };
    let token = session
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or(TokenError::Malformed)?;
    if session
        .get("expires_at")
        .and_then(Value::as_u64)
        .is_some_and(|exp| exp <= now)
    {
        return Err(TokenError::Expired);
    }
    Ok(AccessToken(token.to_string()))
}

pub fn load() -> Result<AccessToken, TokenError> {
    let path = session_path().ok_or(TokenError::Missing)?;
    let raw = std::fs::read_to_string(path).map_err(|_| TokenError::Missing)?;
    parse_session(&raw, now_unix())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(inner: &str) -> String {
        serde_json::json!({
            "other": "x",
            "sb-abc123-auth-token": inner,
        })
        .to_string()
    }

    #[test]
    fn parses_double_encoded_session() {
        let raw = session(r#"{"access_token":"jwt-value","expires_at":2000}"#);
        assert_eq!(parse_session(&raw, 1000).unwrap().expose(), "jwt-value");
    }

    #[test]
    fn rejects_expired() {
        let raw = session(r#"{"access_token":"jwt-value","expires_at":1000}"#);
        assert!(matches!(
            parse_session(&raw, 1000),
            Err(TokenError::Expired)
        ));
    }

    #[test]
    fn missing_and_malformed() {
        assert!(matches!(
            parse_session(r#"{"other":1}"#, 0),
            Err(TokenError::Missing)
        ));
        assert!(matches!(
            parse_session("nope", 0),
            Err(TokenError::Malformed)
        ));
        assert!(matches!(
            parse_session(&session(r#"{"expires_at":5}"#), 0),
            Err(TokenError::Malformed)
        ));
    }

    #[test]
    fn debug_never_prints_secret() {
        let raw = session(r#"{"access_token":"super-secret","expires_at":2000}"#);
        let token = parse_session(&raw, 1000).unwrap();
        assert!(!format!("{token:?}").contains("super-secret"));
    }
}
