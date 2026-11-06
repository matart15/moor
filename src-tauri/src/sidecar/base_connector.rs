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

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeHealth {
    pub connection: String,
    pub last_connected_at_ms: Option<u64>,
    pub last_message_at_ms: Option<u64>,
    pub last_error: Option<&'static str>,
    pub consecutive_failures: u32,
    pub retry_after_seconds: Option<u64>,
}

impl Default for BridgeHealth {
    fn default() -> Self {
        Self {
            connection: "Not configured".into(),
            last_connected_at_ms: None,
            last_message_at_ms: None,
            last_error: None,
            consecutive_failures: 0,
            retry_after_seconds: None,
        }
    }
}

pub type BridgeStatus = Arc<RwLock<BridgeHealth>>;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// Expose error categories, never remote error messages or credentials.
fn failure_reason(message: &str) -> &'static str {
    if message.contains("UNAUTHORIZED") || message.contains("Unauthorized") {
        "Connection secret rejected by Convex"
    } else if message.contains("Invalid pending queue") {
        "Invalid response from Convex"
    } else {
        "Convex connection or subscription failed"
    }
}

fn retry_delay_seconds(failures: u32) -> u64 {
    5_u64
        .saturating_mul(1_u64 << failures.saturating_sub(1).min(4))
        .min(60)
}
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
        FunctionResult::ConvexError(e) => Err(e.message),
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
            let mut health = status.write().await;
            health.connection = "Not configured".into();
            health.retry_after_seconds = None;
            drop(health);
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };
        let secret = match load_secret() {
            Ok(secret) => secret,
            Err(_) => {
                let mut health = status.write().await;
                health.connection = "Connection secret not configured".into();
                health.last_error = Some("Connection secret missing from Keychain");
                health.retry_after_seconds = None;
                drop(health);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        {
            let mut health = status.write().await;
            health.connection = "Connecting".into();
            health.retry_after_seconds = None;
        }
        let connected = async {
            let mut client = ConvexClient::new(&config.deployment_url)
                .await
                .map_err(|e| e.to_string())?;
            let mut subscription = client
                .subscribe("moorBridge:pending", args(&secret))
                .await
                .map_err(|e| e.to_string())?;
            while let Some(result) = subscription.next().await {
                // A successful subscription call is not proof of authentication:
                // the first result may be an UNAUTHORIZED Convex function error.
                let rows = decode_pending(result)?;
                {
                    let mut health = status.write().await;
                    if health.connection != "Connected" {
                        health.connection = "Connected".into();
                        health.last_connected_at_ms = Some(now_ms());
                        health.last_error = None;
                        health.consecutive_failures = 0;
                        health.retry_after_seconds = None;
                    }
                    health.last_message_at_ms = Some(now_ms());
                }
                for row in rows {
                    if let Err(error) = process(state.clone(), &mut client, &secret, &row).await {
                        tracing::warn!("Moor connector request failed: {}", failure_reason(&error));
                    }
                }
            }
            Err::<(), String>("Convex subscription closed".into())
        }
        .await;
        if let Err(error) = connected {
            let reason = failure_reason(&error);
            let delay = {
                let mut health = status.write().await;
                health.connection = "Disconnected".into();
                health.last_error = Some(reason);
                health.consecutive_failures = health.consecutive_failures.saturating_add(1);
                let delay = retry_delay_seconds(health.consecutive_failures);
                health.retry_after_seconds = Some(delay);
                delay
            };
            tracing::warn!("Moor connector disconnected: {reason}; retrying in {delay}s");
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn backs_off_repeated_failures_and_caps_delay() {
        assert_eq!(retry_delay_seconds(1), 5);
        assert_eq!(retry_delay_seconds(2), 10);
        assert_eq!(retry_delay_seconds(3), 20);
        assert_eq!(retry_delay_seconds(4), 40);
        assert_eq!(retry_delay_seconds(5), 60);
        assert_eq!(retry_delay_seconds(u32::MAX), 60);
    }

    #[test]
    fn never_exposes_server_error_or_secret_in_diagnostics() {
        assert_eq!(
            failure_reason("ConvexError: UNAUTHORIZED secret=private"),
            "Connection secret rejected by Convex"
        );
        assert_eq!(
            failure_reason("transport failed: token=private"),
            "Convex connection or subscription failed"
        );
        assert_eq!(
            failure_reason("Invalid pending queue: bad payload"),
            "Invalid response from Convex"
        );
    }
}
