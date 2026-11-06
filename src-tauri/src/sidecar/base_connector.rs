use crate::sidecar::{
    http::AppState,
    mcp::{jsonrpc, server},
    services::settings,
    db::server_repo::ServerRepository,
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
    pub observed_agents: Vec<AgentIdentity>,
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
            observed_agents: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentity {
    pub user_id: String,
    pub client_id: String,
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
        "Bridge authorization rejected by Convex"
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
const PAIRED_KEYCHAIN_ACCOUNT: &str = "paired-connection-secret";

fn paired_secret() -> Result<String, String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, PAIRED_KEYCHAIN_ACCOUNT)
        .map_err(|_| "System credential store unavailable".to_string())?;
    if let Ok(value) = entry.get_password() {
        return Ok(value);
    }
    let secret = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    entry.set_password(&secret).map_err(|_| "System credential store unavailable".to_string())?;
    Ok(secret)
}

async fn base_access_token(state: &AppState) -> Result<String, String> {
    let server = ServerRepository::new(&state.db).find_all()?
        .into_iter()
        .find(|s| s.name == "moor_base_mcp" && s.connection_type == "http")
        .ok_or("Register and authorize moor_base_mcp first")?;
    let url = server.url.ok_or("Base MCP URL missing")?;
    let authorization = state.server_manager.authorization();
    let provider = authorization.credential_provider(&server.id, &url).await?
        .ok_or("Base MCP OAuth authorization required")?;
    let headers = provider.headers().await.map_err(|_| "Base OAuth token unavailable".to_string())?;
    headers.get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .ok_or("Base OAuth token unavailable".into())
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
    let raw = request["rpcJson"].as_str().unwrap_or("");
    let rpc: Json = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}}),
    };
    let id = rpc.get("id").cloned().unwrap_or(Json::Null);
    let paired = request.get("clientId").is_some();
    let profile = if paired {
        let user_id = request["userId"].as_str().unwrap_or("");
        let client_id = request["clientId"].as_str().unwrap_or("");
        let key = format!("{user_id}:{client_id}");
        let config = settings::get_settings(&state.db).ok();
        let profile = config.as_ref().and_then(|s| s.bridge.agent_profiles.get(&key));
        match profile {
            Some(profile) => profile.clone(),
            None => return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32003,"message":"Agent not assigned to a Moor profile"}}),
        }
    } else {
        request["profileId"].as_str().unwrap_or("").to_string()
    };
    if crate::sidecar::db::profile_repo::ProfileRepository::new(&state.db)
        .find_by_id(&profile)
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
        Some(&profile),
    )
    .await
}
async fn process(
    state: Arc<AppState>,
    client: &mut ConvexClient,
    secret: &str,
    row: &Json,
    paired: bool,
) -> Result<(), String> {
    let id = row["id"].as_str().ok_or("Missing queue ID")?;
    let mut claim_args = args(secret);
    claim_args.insert("id".into(), id.into());
    let claim_function = if paired { "moorBridgePaired:claim" } else { "moorBridge:claim" };
    if require_value(
        client
            .mutation(claim_function, claim_args)
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
            .mutation(if paired { "moorBridgePaired:complete" } else { "moorBridge:complete" }, finish_args)
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
        let secret = match if config.paired { paired_secret() } else { load_secret() } {
            Ok(secret) => secret,
            Err(_) => {
                let mut health = status.write().await;
                health.connection = if config.paired { "Pairing credential unavailable" } else { "Connection secret not configured" }.into();
                health.last_error = Some("System credential store unavailable");
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
            if config.paired {
                let access_token = base_access_token(&state).await?;
                let mut pair_args = args(&secret);
                pair_args.insert("accessToken".into(), access_token.into());
                require_value(client.mutation("moorBridgePaired:pair", pair_args).await.map_err(|e| e.to_string())?)?;
            }
            let mut subscription = client
                .subscribe(if config.paired { "moorBridgePaired:pending" } else { "moorBridge:pending" }, args(&secret))
                .await
                .map_err(|e| e.to_string())?;
            let renew_at = tokio::time::Instant::now() + Duration::from_secs(30 * 60);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
            heartbeat.tick().await;
            loop {
                let result = tokio::select! {
                    next = subscription.next() => next,
                    _ = tokio::time::sleep_until(renew_at), if config.paired => return Ok::<(), String>(()),
                    _ = heartbeat.tick(), if config.paired => {
                        require_value(client.mutation("moorBridgePaired:heartbeat", args(&secret)).await.map_err(|e| e.to_string())?)?;
                        continue;
                    },
                };
                let Some(result) = result else { break; };
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
                    if config.paired {
                        if let (Some(user_id), Some(client_id)) = (row["userId"].as_str(), row["clientId"].as_str()) {
                            let agent = AgentIdentity { user_id: user_id.to_string(), client_id: client_id.to_string() };
                            let mut health = status.write().await;
                            if !health.observed_agents.contains(&agent) {
                                health.observed_agents.push(agent);
                                if health.observed_agents.len() > 10 { health.observed_agents.remove(0); }
                            }
                        }
                    }
                    if let Err(error) = process(state.clone(), &mut client, &secret, &row, config.paired).await {
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

    #[tokio::test]
    async fn unknown_paired_client_is_denied_without_falling_back_to_active_profile() {
        let dir = std::env::temp_dir().join(format!("moor-paired-test-{}", uuid::Uuid::new_v4()));
        let state = AppState::for_test(&dir);
        let result = execute(
            state,
            &json!({
                "userId": "owner",
                "clientId": "unassigned-agent",
                "rpcJson": "{\"jsonrpc\":\"2.0\",\"id\":17,\"method\":\"tools/list\"}"
            }),
        ).await;
        assert_eq!(result["id"], 17);
        assert_eq!(result["error"]["code"], -32003);
        let _ = std::fs::remove_dir_all(dir);
    }

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
            "Bridge authorization rejected by Convex"
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
