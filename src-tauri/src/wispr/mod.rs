//! Opt-in cloud transcription through the Wispr voice-actions socket.

mod client;
mod codec;
mod token;

pub use client::transcribe;
pub use token::{load as load_token, TokenStatus};

/// Whether a usable Wispr session is on disk, for the settings UI.
pub fn token_status() -> TokenStatus {
    match load_token() {
        Ok(_) => TokenStatus::Ok,
        Err(e) => e.status(),
    }
}
