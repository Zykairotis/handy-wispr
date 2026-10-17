use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings, ModelUnloadTimeout, TranscriptionProvider};
use crate::wispr::{self, TokenStatus};
use serde::Serialize;
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[derive(Serialize, Type)]
pub struct ModelLoadStatus {
    is_loaded: bool,
    current_model: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn set_model_unload_timeout(app: AppHandle, timeout: ModelUnloadTimeout) {
    let mut settings = get_settings(&app);
    settings.model_unload_timeout = timeout;
    write_settings(&app, settings);
}

/// Cloud transcription uploads audio, so it cannot be selected before the
/// user has consented.
#[tauri::command]
#[specta::specta]
pub fn change_transcription_provider_setting(
    app: AppHandle,
    provider: TranscriptionProvider,
) -> Result<(), String> {
    let mut settings = get_settings(&app);
    if provider == TranscriptionProvider::Wispr && !settings.wispr_consent {
        return Err("Consent is required before using Wispr cloud transcription".to_string());
    }
    settings.transcription_provider = provider;
    write_settings(&app, settings);
    Ok(())
}

/// Withdrawing consent also returns to local transcription.
#[tauri::command]
#[specta::specta]
pub fn change_wispr_consent_setting(app: AppHandle, enabled: bool) {
    let mut settings = get_settings(&app);
    settings.wispr_consent = enabled;
    if !enabled {
        settings.transcription_provider = TranscriptionProvider::Local;
    }
    write_settings(&app, settings);
}

/// Onboarding path for users who skip downloading a local model. The click
/// that calls this is the consent, so the UI must disclose the upload first.
#[tauri::command]
#[specta::specta]
pub fn enable_wispr_cloud(app: AppHandle) {
    let mut settings = get_settings(&app);
    settings.wispr_consent = true;
    settings.transcription_provider = TranscriptionProvider::Wispr;
    settings.onboarding_completed = true;
    write_settings(&app, settings);
}

#[tauri::command]
#[specta::specta]
pub fn get_wispr_token_status() -> TokenStatus {
    wispr::token_status()
}

#[tauri::command]
#[specta::specta]
pub fn get_model_load_status(
    transcription_manager: State<Arc<TranscriptionManager>>,
) -> Result<ModelLoadStatus, String> {
    Ok(ModelLoadStatus {
        is_loaded: transcription_manager.is_model_loaded(),
        current_model: transcription_manager.get_current_model(),
    })
}

#[tauri::command]
#[specta::specta]
pub fn unload_model_manually(
    transcription_manager: State<Arc<TranscriptionManager>>,
) -> Result<(), String> {
    transcription_manager.request_unload();
    Ok(())
}
