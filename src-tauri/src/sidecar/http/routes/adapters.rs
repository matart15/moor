//! Owner-only management of virtual adapter definitions and Keychain secrets.
//! The existing /api/* loopback + X-Moor-Token middleware protects these routes.
use crate::sidecar::db::adapter_repo::{AdapterDefinition, AdapterRepository, StoredAdapter};
use crate::sidecar::http::{app_error::AppError, AppState};
use crate::sidecar::services::{adapter_execution, event_bus::Evt};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/adapters", get(list).post(create))
        .route(
            "/api/adapters/{id}",
            get(get_one).put(update).delete(remove),
        )
        .route(
            "/api/adapters/credentials",
            post(create_credential).delete(delete_credential),
        )
}

fn changed(state: &AppState) {
    state.event_bus.emit(Evt::ToolsChanged {
        reason: "adapter".into(),
    });
}

async fn list(State(state): State<Arc<AppState>>) -> Result<Json<Vec<StoredAdapter>>, AppError> {
    Ok(Json(
        AdapterRepository::new(&state.db)
            .list()
            .map_err(AppError::internal)?,
    ))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<StoredAdapter>, AppError> {
    AdapterRepository::new(&state.db)
        .get(&id)
        .map_err(AppError::internal)?
        .map(Json)
        .ok_or_else(|| AppError::not_found("Adapter not found"))
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AdapterDefinition>,
) -> Result<(StatusCode, Json<StoredAdapter>), AppError> {
    adapter_execution::validate_definition(&body).map_err(AppError::validation)?;
    let created = AdapterRepository::new(&state.db)
        .create(&body)
        .map_err(AppError::internal)?;
    if let Err(error) = state.server_manager.register_adapter(&created.id).await {
        let _ = AdapterRepository::new(&state.db).delete(&created.id);
        return Err(AppError::internal(error));
    }
    changed(&state);
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<AdapterDefinition>,
) -> Result<Json<StoredAdapter>, AppError> {
    adapter_execution::validate_definition(&body).map_err(AppError::validation)?;
    if AdapterRepository::new(&state.db)
        .get(&id)
        .map_err(AppError::internal)?
        .is_none()
    {
        return Err(AppError::not_found("Adapter not found"));
    }
    let updated = AdapterRepository::new(&state.db)
        .update(&id, &body)
        .map_err(AppError::validation)?;
    state
        .server_manager
        .update_server_memory(&id, Some(&body.name), None)
        .await;
    changed(&state);
    Ok(Json(updated))
}

async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    if AdapterRepository::new(&state.db)
        .get(&id)
        .map_err(AppError::internal)?
        .is_none()
    {
        return Err(AppError::not_found("Adapter not found"));
    }
    if !state.server_manager.remove_server(&id).await {
        return Err(AppError::internal_public(
            "Adapter removal failed",
            "Unable to remove adapter",
        ));
    }
    changed(&state);
    Ok(Json(json!({"success": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretBody {
    secret: String,
}

async fn create_credential(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SecretBody>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let reference = state
        .server_manager
        .authorization()
        .create_adapter_secret(&body.secret)
        .map_err(|_| {
            AppError::internal_public(
                "Unable to store adapter credential",
                "Credential storage unavailable",
            )
        })?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"credentialRef": reference})),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteSecretBody {
    credential_ref: String,
}

async fn delete_credential(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DeleteSecretBody>,
) -> Result<Json<Value>, AppError> {
    state
        .server_manager
        .authorization()
        .delete_adapter_secret(&body.credential_ref)
        .map_err(|_| {
            AppError::validation("Invalid credential reference or credential store unavailable")
        })?;
    Ok(Json(json!({"success": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::services::server_authorization::{CredentialStore, MemoryCredentialStore};

    #[test]
    fn credential_references_are_isolated_from_oauth_and_values_are_not_saved_in_sqlite() {
        let dir =
            std::env::temp_dir().join(format!("moor-adapter-credential-{}", uuid::Uuid::new_v4()));
        let db = Arc::new(crate::sidecar::db::Database::open(&dir.with_extension("db")).unwrap());
        db.run_migrations().unwrap();
        let events = Arc::new(crate::sidecar::services::event_bus::EventBus::new(8));
        let memory = Arc::new(MemoryCredentialStore::default());
        let auth = crate::sidecar::services::server_authorization::ServerAuthorization::with_credential_store(
            db.clone(), events, memory.clone());
        let reference = auth.create_adapter_secret("super-private-token").unwrap();
        assert!(reference.starts_with("adapter:"));
        assert_eq!(
            auth.adapter_secret(&reference).unwrap(),
            "super-private-token"
        );
        assert!(auth.adapter_secret("oauth:some-other-reference").is_err());
        assert!(!db
            .query_all("SELECT value FROM settings", &[], |row| row
                .get::<_, String>(0))
            .unwrap()
            .iter()
            .any(|value| value.contains("super-private-token")));
        assert!(memory.get(&reference).unwrap().is_some());
        auth.delete_adapter_secret(&reference).unwrap();
        assert!(auth.adapter_secret(&reference).is_err());
        drop(db);
        let _ = std::fs::remove_file(dir.with_extension("db"));
    }
}
