use crate::sidecar::http::app_error::AppError;
use crate::sidecar::http::AppState;
use crate::sidecar::services::event_bus::Evt;
use crate::sidecar::services::settings as settings_store;
use crate::sidecar::services::tool_search::jev_ranker;
use axum::{
    extract::State,
    response::Json,
    routing::{get, post},
    Router,
};
use serde_json::Value;
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/settings", get(get_settings).patch(update_settings))
        .route("/api/settings/reset", post(reset_settings))
        .route(
            "/api/settings/tool-ranking/credential",
            get(get_ranking_credential).post(save_ranking_credential),
        )
}

async fn get_settings(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let settings = settings_store::get_settings(&state.db).map_err(AppError::internal)?;
    Ok(Json(
        serde_json::to_value(settings).map_err(|e| AppError::internal(e.to_string()))?,
    ))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<Value>,
) -> Result<Json<Value>, AppError> {
    let settings = settings_store::update_settings(&state.db, body).map_err(|e| match e {
        settings_store::SettingsError::Validation(m) => AppError::validation(m),
        settings_store::SettingsError::Internal(m) => AppError::internal(m),
    })?;
    let settings_value =
        serde_json::to_value(&settings).map_err(|e| AppError::internal(e.to_string()))?;

    state.event_bus.emit(Evt::SettingsChanged {
        settings: settings_value.clone(),
    });
    Ok(Json(settings_value))
}

async fn reset_settings(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let defaults = settings_store::reset_settings(&state.db).map_err(AppError::internal)?;
    let defaults_value =
        serde_json::to_value(&defaults).map_err(|e| AppError::internal(e.to_string()))?;
    state.event_bus.emit(Evt::SettingsChanged {
        settings: defaults_value.clone(),
    });
    Ok(Json(defaults_value))
}

// Only status and a non-secret Keychain account reference leave the backend.
async fn get_ranking_credential(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let settings = settings_store::get_settings(&state.db).map_err(AppError::internal)?;
    let reference = if settings.advanced.tool_ranking_credential_ref.is_empty() {
        jev_ranker::DEFAULT_CREDENTIAL_REF.to_owned()
    } else {
        settings.advanced.tool_ranking_credential_ref
    };
    let account = reference.clone();
    let configured = tokio::task::spawn_blocking(move || jev_ranker::credential_configured(&account))
        .await
        .map_err(|_| AppError::internal("System credential store is unavailable"))?
        .map_err(AppError::internal)?;
    Ok(Json(serde_json::json!({ "configured": configured, "reference": reference })))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RankingCredentialInput {
    api_key: String,
    reference: Option<String>,
}

fn validate_openrouter_key(key: &str) -> Result<(), AppError> {
    if key.len() < 8 || key.len() > 512 || key.chars().any(char::is_whitespace) {
        return Err(AppError::validation(
            "OpenRouter API key must be 8–512 characters without whitespace",
        ));
    }
    Ok(())
}

async fn save_ranking_credential(
    State(state): State<Arc<AppState>>,
    axum::Json(input): axum::Json<RankingCredentialInput>,
) -> Result<Json<Value>, AppError> {
    let api_key = input.api_key.trim().to_owned();
    validate_openrouter_key(&api_key)?;
    let current = settings_store::get_settings(&state.db).map_err(AppError::internal)?;
    let reference = input
        .reference
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if current.advanced.tool_ranking_credential_ref.is_empty() {
                jev_ranker::DEFAULT_CREDENTIAL_REF
            } else {
                &current.advanced.tool_ranking_credential_ref
            }
        })
        .trim()
        .to_owned();
    let patch = serde_json::json!({ "advanced": { "toolRankingCredentialRef": reference } });
    // Validate the reference against the normal settings constraints BEFORE
    // writing a password. Only the reference ever enters the settings DB.
    settings_store::merge_settings_value(current.clone(), patch.clone())
        .map_err(AppError::validation)?;
    let account = reference.clone();
    tokio::task::spawn_blocking(move || jev_ranker::store_credential(&account, &api_key))
        .await
        .map_err(|_| AppError::internal("System credential store is unavailable"))?
        .map_err(AppError::internal)?;
    if current.advanced.tool_ranking_credential_ref != reference {
        update_settings(State(state), axum::Json(patch)).await?;
    }
    Ok(Json(serde_json::json!({ "configured": true, "reference": reference })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn temp_data_dir(test_name: &str) -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time is before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("moor-settings-route-{test_name}-{timestamp}"))
    }

    fn test_state(data_dir: std::path::PathBuf) -> Arc<AppState> {
        AppState::for_test(&data_dir)
    }

    #[test]
    fn openrouter_key_validation_never_echoes_credentials() {
        let too_long = "x".repeat(513);
        for invalid in ["", "short", "one two three", "key\nwith-newline", too_long.as_str()] {
            let error = validate_openrouter_key(invalid).expect_err("invalid key");
            assert_eq!(error.status_code(), axum::http::StatusCode::BAD_REQUEST);
            assert!(!error.to_string().contains(invalid) || invalid.is_empty());
        }
        validate_openrouter_key("sk-or-v1-example-value").expect("valid key format");
    }

    #[tokio::test]
    async fn get_settings_returns_defaults_for_fresh_store() {
        let data_dir = temp_data_dir("fresh");
        let Json(value) = get_settings(State(test_state(data_dir.clone())))
            .await
            .expect("settings should load");
        assert_eq!(value["advanced"]["sidecarPort"], 9323);
        assert_eq!(value["general"]["minimizeToTrayOnClose"], true);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn reset_settings_returns_defaults_without_writing_json() {
        let data_dir = temp_data_dir("reset");
        let state = test_state(data_dir.clone());
        std::fs::write(
            settings_store::settings_path(&data_dir),
            r#"{"advanced":{"sidecarPort":9333}}"#,
        )
        .expect("failed to write stale json");
        let _ = update_settings(
            State(state.clone()),
            axum::Json(serde_json::json!({ "advanced": { "sidecarPort": 9444 } })),
        )
        .await
        .expect("settings update should succeed");

        let Json(value) = reset_settings(State(state.clone()))
            .await
            .expect("settings reset should succeed");
        assert_eq!(value["advanced"]["sidecarPort"], 9323);
        assert_eq!(
            settings_store::get_settings(&state.db)
                .expect("settings should load")
                .advanced
                .sidecar_port,
            9323
        );
        assert_eq!(
            settings_store::read_settings_file(&data_dir)
                .advanced
                .sidecar_port,
            9333
        );
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn update_settings_rejects_invalid_port_with_validation_error() {
        let data_dir = temp_data_dir("invalid-port");
        let err = update_settings(
            State(test_state(data_dir.clone())),
            axum::Json(serde_json::json!({ "advanced": { "sidecarPort": 80 } })),
        )
        .await
        .expect_err("invalid port should be rejected");
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.code(), "VALIDATION_ERROR");
        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn settings_changed_event_payload_is_settings_object() {
        let data_dir = temp_data_dir("event-payload");
        let state = test_state(data_dir.clone());
        let mut events = state.event_bus.subscribe();

        let Json(value) = update_settings(
            State(state),
            axum::Json(serde_json::json!({
                "general": { "minimizeToTrayOnClose": false }
            })),
        )
        .await
        .expect("settings update should succeed");

        let evt = events
            .recv()
            .await
            .expect("settings event should be emitted");
        assert_eq!(evt.name(), "settings:changed");
        let payload = evt.payload();
        assert_eq!(payload, value);
        assert_eq!(payload["general"]["minimizeToTrayOnClose"], false);
        let _ = std::fs::remove_dir_all(data_dir);
    }
}
