use crate::sidecar::{
    http::AppState,
    mcp::{jsonrpc, server},
    services::settings,
};
use convex::{ConvexClient, FunctionResult, Value};
use futures::StreamExt;
use serde_json::{json, Value as Json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::RwLock;

pub type BridgeStatus = Arc<RwLock<String>>;
const KEYCHAIN_SERVICE: &str = "com.specdest.moor-dev.base-connector";
const KEYCHAIN_ACCOUNT: &str = "connection-secret";

pub fn store_secret(secret: &str) -> Result<(), String> {
    if secret.len() < 32 {
        return Err("Connection secret must be at least 32 characters".into());
    }
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| e.to_string())?
        .set_password(secret)
        .map_err(|e| e.to_string())
}
pub fn has_secret() -> bool {
    load_secret().is_ok()
}
fn load_secret() -> Result<String, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| e.to_string())?
        .get_password()
        .map_err(|e| e.to_string())
}
fn args(secret: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([("secret".to_string(), Value::String(secret.to_string()))])
}
fn require_value(result: FunctionResult) -> Result<Value, String> {
    match result {
        FunctionResult::Value(v) => Ok(v),
        FunctionResult::ErrorMessage(e) => Err(e),
        FunctionResult::ConvexError(_) => Err("Convex rejected connector operation".into()),
    }
}
fn decode_pending(result: FunctionResult) -> Result<Vec<Json>, String> {
    let value = require_value(result)?;
    let json: Json = value.into();
    serde_json::from_value(json).map_err(|e| format!("Invalid pending queue: {e}"))
}
async fn execute(state: Arc<AppState>, request: &Json) -> Json {
    let profile = request["profileId"].as_str().unwrap_or("");
    let raw = request["rpcJson"].as_str().unwrap_or("");
    let rpc: Json = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            return json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})
        }
    };
    let id = rpc.get("id").cloned().unwrap_or(Json::Null);
    if crate::sidecar::db::profile_repo::ProfileRepository::new(&state.db)
        .find_by_id(profile)
        .ok()
        .flatten()
        .is_none()
    {
        return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32002,"message":"Unknown profile"}});
    }
    let Some((id, method, params)) = jsonrpc::parse_request_value(&rpc) else {
        return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Invalid request"}});
    };
    server::handle_request_for_profile(
        id,
        &method,
        params,
        state,
        Some("Base connector"),
        Some(profile),
    )
    .await
}
async fn process(
    state: Arc<AppState>,
    client: &mut ConvexClient,
    secret: &str,
    row: &Json,
) -> Result<(), String> {
    let id = row["id"].as_str().ok_or("Missing queue ID")?;
    let mut claim_args = args(secret);
    claim_args.insert("id".into(), id.into());
    if require_value(
        client
            .mutation("moorBridge:claim", claim_args)
            .await
            .map_err(|e| e.to_string())?,
    )? != Value::Boolean(true)
    {
        return Ok(());
    }
    // Claim is atomic. Never execute again if completion fails or the client reconnects.
    let response = execute(state, row).await.to_string();
    let mut finish_args = args(secret);
    finish_args.insert("id".into(), id.into());
    finish_args.insert("responseJson".into(), response.into());
    require_value(
        client
            .mutation("moorBridge:complete", finish_args)
            .await
            .map_err(|e| e.to_string())?,
    )?;
    Ok(())
}
pub async fn run(state: Arc<AppState>, status: BridgeStatus) {
    loop {
        let config = settings::get_settings(&state.db).ok().map(|s| s.bridge);
        let Some(config) = config.filter(|s| s.enabled && !s.deployment_url.is_empty()) else {
            *status.write().await = "Not configured".into();
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };
        let secret = match load_secret() {
            Ok(secret) => secret,
            Err(_) => {
                *status.write().await = "Connection secret not configured".into();
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        *status.write().await = "Connecting".into();
        let connected = async {
            let mut client = ConvexClient::new(&config.deployment_url)
                .await
                .map_err(|e| e.to_string())?;
            let mut subscription = client
                .subscribe("moorBridge:pending", args(&secret))
                .await
                .map_err(|e| e.to_string())?;
            *status.write().await = "Connected".into();
            while let Some(result) = subscription.next().await {
                let rows = decode_pending(result)?;
                for row in rows {
                    if let Err(error) = process(state.clone(), &mut client, &secret, &row).await {
                        tracing::warn!("Moor connector request failed: {error}");
                    }
                }
            }
            Err::<(), String>("Convex subscription closed".into())
        }
        .await;
        if let Err(error) = connected {
            tracing::warn!("Moor connector disconnected: {error}");
            *status.write().await = "Disconnected".into();
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
