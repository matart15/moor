//! Execution and validation for configured virtual API / approved CLI tools.
//! No caller-controlled URL, headers, executable, environment, or cwd.
use crate::sidecar::db::adapter_repo::{AdapterDefinition, AdapterKind};
use crate::sidecar::services::server_manager::ToolCallError;
use regex_lite::Regex;
use reqwest::{Client, Method, Url};
use serde_json::{json, Map, Value};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_IO_BYTES: usize = 1024 * 1024;
const MAX_ARGS: usize = 32;
const MIN_TIMEOUT_MS: u64 = 100;
const MAX_TIMEOUT_MS: u64 = 30_000;

pub fn validate_definition(def: &AdapterDefinition) -> Result<(), String> {
    if def.name.trim().is_empty()
        || def.name.len() > 80
        || def.tool_name.is_empty()
        || def.tool_name.len() > 80
        || !def
            .tool_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || def.description.trim().is_empty()
        || def.description.len() > 2048
    {
        return Err("Invalid adapter name, toolName or description".into());
    }
    if !matches!(
        def.risk_level.as_str(),
        "safe" | "write" | "destructive" | "unknown"
    ) {
        return Err("riskLevel must be safe, write, destructive or unknown".into());
    }
    validate_schema(&def.input_schema)?;
    let properties = def.input_schema["properties"]
        .as_object()
        .ok_or("Invalid properties")?;
    let bounded = |timeout: u64, cap: usize| {
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&timeout) || cap == 0 || cap > MAX_IO_BYTES {
            Err("timeoutMs or response/output size exceeds adapter limits".to_string())
        } else {
            Ok(())
        }
    };
    match &def.adapter {
        AdapterKind::Api {
            base_url,
            path,
            method,
            headers,
            query_fields,
            body_fields,
            credential_ref,
            allow_local_http,
            timeout_ms,
            max_response_bytes,
        } => {
            bounded(*timeout_ms, *max_response_bytes)?;
            let url = fixed_api_url(base_url, path, *allow_local_http)?;
            if url.query().is_some() || url.fragment().is_some() {
                return Err("Fixed API URL must not include a query or fragment".into());
            }
            if !matches!(
                method.to_ascii_uppercase().as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
            ) {
                return Err("Unsupported HTTP method".into());
            }
            if matches!(method.to_ascii_uppercase().as_str(), "GET" | "DELETE")
                && !body_fields.is_empty()
            {
                return Err("GET/DELETE tools cannot include a JSON body".into());
            }
            if headers.len() > 16
                || headers.iter().any(|(name, value)| {
                    !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                        || name.len() > 80
                        || value.len() > 2048
                        || [
                            "auth",
                            "token",
                            "secret",
                            "password",
                            "credential",
                            "api-key",
                            "apikey",
                            "access-key",
                        ]
                        .iter()
                        .any(|sensitive| name.to_ascii_lowercase().contains(sensitive))
                        || [
                            "cookie",
                            "host",
                            "content-length",
                            "transfer-encoding",
                            "connection",
                            "x-api-key",
                        ]
                        .iter()
                        .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
                        || name.to_ascii_lowercase().starts_with("proxy-")
                        || value.chars().any(char::is_control)
                })
            {
                return Err("API headers must be fixed, non-sensitive header values".into());
            }
            if credential_ref.as_deref().is_some_and(|r| {
                r.strip_prefix("adapter:")
                    .and_then(|uuid| uuid::Uuid::parse_str(uuid).ok())
                    .is_none()
            }) {
                return Err("credentialRef must point to a Moor adapter Keychain secret".into());
            }
            if query_fields.len() + body_fields.len() > MAX_ARGS {
                return Err("Too many mapped arguments".into());
            }
            for (field, query_name) in query_fields {
                if !properties.contains_key(field)
                    || query_name.is_empty()
                    || query_name.len() > 80
                    || !query_name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Err("Invalid queryFields mapping".into());
                }
            }
            for field in body_fields {
                if !properties.contains_key(field) || query_fields.contains_key(field) {
                    return Err("Invalid bodyFields mapping".into());
                }
            }
            if properties
                .keys()
                .any(|key| !query_fields.contains_key(key) && !body_fields.contains(key))
            {
                return Err("Each API parameter must have an explicit request mapping".into());
            }
        }
        AdapterKind::Cli {
            bridge_server_id,
            bridge_tool_name,
            executable,
            working_directory,
            fixed_args,
            argument_fields,
            approval_id,
            timeout_ms,
            max_output_bytes,
        } => {
            bounded(*timeout_ms, *max_output_bytes)?;
            // A container-side *approved* typed-argv executor is required. Never
            // delegate to an unrestricted run_command or a native Mac shell.
            if bridge_server_id.is_empty()
                || bridge_tool_name != "execute_approved_cli"
                || approval_id.trim().is_empty()
                || approval_id.len() > 200
            {
                return Err("CLI requires an approved container bridge and approvalId".into());
            }
            if !executable.starts_with('/')
                || !working_directory.starts_with('/')
                || executable.contains("..")
                || working_directory.contains("..")
                || executable.chars().any(char::is_control)
                || working_directory.chars().any(char::is_control)
                || executable.len() > 512
                || working_directory.len() > 512
            {
                return Err(
                    "CLI executable and working directory must be fixed absolute container paths"
                        .into(),
                );
            }
            if matches!(
                executable.rsplit('/').next().unwrap_or(""),
                "sh" | "bash"
                    | "zsh"
                    | "dash"
                    | "fish"
                    | "powershell"
                    | "cmd"
                    | "env"
                    | "python"
                    | "python3"
                    | "node"
                    | "ruby"
                    | "perl"
                    | "php"
                    | "lua"
                    | "busybox"
            ) {
                return Err("Shell and environment interpreters are not allowed".into());
            }
            if fixed_args.len() + argument_fields.len() > MAX_ARGS
                || fixed_args.iter().any(|arg| {
                    arg.len() > 2048
                        || arg.chars().any(char::is_control)
                        || matches!(arg.as_str(), "-c" | "-e" | "--eval" | "--exec")
                })
            {
                return Err("Invalid fixed CLI arguments".into());
            }
            for field in argument_fields {
                if !properties.contains_key(field)
                    || !def.input_schema["required"]
                        .as_array()
                        .is_some_and(|items| items.contains(&json!(field)))
                {
                    return Err("CLI argumentFields must map required schema parameters".into());
                }
            }
            if properties.keys().any(|key| !argument_fields.contains(key)) {
                return Err("Every CLI parameter must have an explicit argv mapping".into());
            }
        }
    }
    Ok(())
}

/// Explicitly bounded JSON-schema subset. Unsupported keywords are rejected
/// at configuration time rather than silently weakening tool validation.
fn validate_schema(schema: &Value) -> Result<(), String> {
    let obj = schema.as_object().ok_or("inputSchema must be an object")?;
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return Err("inputSchema must declare type: object".into());
    }
    let allowed = [
        "type",
        "properties",
        "required",
        "additionalProperties",
        "description",
    ];
    if obj.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Unsupported JSON schema keyword".into());
    }
    if obj
        .get("additionalProperties")
        .is_some_and(|value| value != &json!(false))
    {
        return Err("additionalProperties must be false".into());
    }
    let props = obj
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("inputSchema.properties is required")?;
    if props.len() > MAX_ARGS {
        return Err("Too many schema properties".into());
    }
    let required = obj
        .get("required")
        .map(|v| v.as_array().ok_or("required must be an array"))
        .transpose()?
        .cloned()
        .unwrap_or_default();
    for field in &required {
        if !field.as_str().is_some_and(|s| props.contains_key(s)) {
            return Err("Invalid required parameter".into());
        }
    }
    for (name, prop) in props {
        if name.len() > 80
            || name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err("Invalid parameter name".into());
        }
        let prop = prop
            .as_object()
            .ok_or("Parameter schema must be an object")?;
        if !matches!(
            prop.get("type").and_then(Value::as_str),
            Some("string" | "integer" | "number" | "boolean")
        ) {
            return Err("Adapter parameters must be primitive JSON types".into());
        }
        let valid_keys = [
            "type",
            "description",
            "enum",
            "minimum",
            "maximum",
            "minLength",
            "maxLength",
            "pattern",
        ];
        if prop.keys().any(|k| !valid_keys.contains(&k.as_str())) {
            return Err("Unsupported parameter schema keyword".into());
        }
        if let Some(pattern) = prop.get("pattern") {
            let raw = pattern.as_str().ok_or("pattern must be a string")?;
            if raw.len() > 256 {
                return Err("Pattern is too long".into());
            }
            Regex::new(raw).map_err(|_| "Invalid parameter pattern")?;
        }
        if prop.get("enum").is_some_and(|v| !v.is_array()) {
            return Err("enum must be an array".into());
        }
        for (key, value) in prop {
            if ["minLength", "maxLength"].contains(&key.as_str()) && value.as_u64().is_none() {
                return Err("Length bounds must be nonnegative integers".into());
            }
            if ["minimum", "maximum"].contains(&key.as_str()) && value.as_f64().is_none() {
                return Err("Numeric bounds must be numbers".into());
            }
        }
    }
    Ok(())
}

pub fn validate_arguments(schema: &Value, args: &Value) -> Result<(), ToolCallError> {
    let fail =
        |message: &str| ToolCallError::InvalidTool(format!("Invalid tool arguments: {message}"));
    if serde_json::to_vec(args).map_or(true, |encoded| encoded.len() > MAX_REQUEST_BYTES) {
        return Err(fail("request too large"));
    }
    let values = args.as_object().ok_or_else(|| fail("expected an object"))?;
    let properties = schema["properties"]
        .as_object()
        .ok_or_else(|| fail("invalid registered schema"))?;
    let required = schema["required"].as_array();
    if required.is_some_and(|items| {
        items
            .iter()
            .any(|item| item.as_str().is_some_and(|k| !values.contains_key(k)))
    }) {
        return Err(fail("missing required parameter"));
    }
    for (key, value) in values {
        let property = properties
            .get(key)
            .ok_or_else(|| fail("unknown parameter"))?;
        let typ = property["type"].as_str().unwrap_or("");
        let type_ok = match typ {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            _ => false,
        };
        if !type_ok {
            return Err(fail("parameter has wrong type"));
        }
        if property["enum"]
            .as_array()
            .is_some_and(|set| !set.contains(value))
        {
            return Err(fail("parameter not in enum"));
        }
        if let Some(text) = value.as_str() {
            if text.len() > 2048
                || property["minLength"]
                    .as_u64()
                    .is_some_and(|n| text.chars().count() < n as usize)
                || property["maxLength"]
                    .as_u64()
                    .is_some_and(|n| text.chars().count() > n as usize)
            {
                return Err(fail("string length invalid"));
            }
            if let Some(pattern) = property["pattern"].as_str() {
                if !Regex::new(pattern)
                    .map_err(|_| fail("invalid schema pattern"))?
                    .is_match(text)
                {
                    return Err(fail("string pattern mismatch"));
                }
            }
        }
        if let Some(number) = value.as_f64() {
            if property["minimum"].as_f64().is_some_and(|n| number < n)
                || property["maximum"].as_f64().is_some_and(|n| number > n)
            {
                return Err(fail("numeric bound exceeded"));
            }
        }
    }
    Ok(())
}

fn fixed_api_url(base: &str, path: &str, allow_local: bool) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|_| "Invalid API base URL")?;
    if url.cannot_be_a_base()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("API base URL must have a fixed host and no credentials/query/fragment".into());
    }
    if path.is_empty()
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.contains("..")
        || path.chars().any(|c| matches!(c, '?' | '#' | '%' | '\\'))
        || path.chars().any(char::is_control)
    {
        return Err("Invalid fixed API path".into());
    }
    if url.scheme() != "https"
        && !(allow_local && url.scheme() == "http" && is_local_host(url.host_str().unwrap_or("")))
    {
        return Err("External APIs require HTTPS; local HTTP requires an explicitly allowed local destination".into());
    }
    let joined = format!("{}{}", url.path().trim_end_matches('/'), path);
    url.set_path(&joined);
    Ok(url)
}

// Only loopback and private LAN/container addresses can be local HTTP targets.
// Reject cloud metadata, link-local, multicast, unspecified and CGNAT ranges.
fn allowed_local_http_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback() || mapped.is_private())
        }
    }
}

fn is_local_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("host.docker.internal")
        || host.ends_with(".internal")
        || host.parse::<IpAddr>().is_ok_and(allowed_local_http_ip)
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip == Ipv4Addr::BROADCAST
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 224
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 169 && ip.octets()[1] == 254)
                || (ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1]))
                || ip == Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|ipv4| is_private(IpAddr::V4(ipv4)))
                || ip == Ipv6Addr::LOCALHOST
        }
    }
}

fn primitive_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub async fn execute_api(
    definition: &AdapterDefinition,
    args: &Value,
    credential: Option<&str>,
) -> Result<Value, ToolCallError> {
    let AdapterKind::Api {
        base_url,
        path,
        method,
        headers,
        query_fields,
        body_fields,
        allow_local_http,
        timeout_ms,
        max_response_bytes,
        ..
    } = &definition.adapter
    else {
        return Err(ToolCallError::Execution("Incorrect API adapter".into()));
    };
    validate_arguments(&definition.input_schema, args)?;
    let mut url = fixed_api_url(base_url, path, *allow_local_http)
        .map_err(|_| ToolCallError::Execution("API destination rejected".into()))?;
    let host = url
        .host_str()
        .ok_or_else(|| ToolCallError::Execution("API destination rejected".into()))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| ToolCallError::Execution("Invalid API port".into()))?;
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| ToolCallError::Execution("API destination DNS lookup failed".into()))?
        .collect();
    if addresses.is_empty()
        || (url.scheme() == "https" && addresses.iter().any(|addr| is_private(addr.ip())))
        || (url.scheme() == "http"
            && addresses
                .iter()
                .any(|addr| !allowed_local_http_ip(addr.ip())))
    {
        return Err(ToolCallError::Execution(
            "API destination rejected (private or unresolved address)".into(),
        ));
    }
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &addresses)
        .timeout(Duration::from_millis(*timeout_ms))
        .build()
        .map_err(|_| ToolCallError::Execution("Unable to configure API client".into()))?;
    if let Some(obj) = args.as_object() {
        let mut query: Vec<(&str, String)> = Vec::new();
        for (key, name) in query_fields {
            if let Some(value) = obj.get(key) {
                query.push((
                    name.as_str(),
                    primitive_string(value).ok_or_else(|| {
                        ToolCallError::InvalidTool("Invalid mapped API argument".into())
                    })?,
                ));
            }
        }
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in &query {
                pairs.append_pair(name, value);
            }
        }
    }
    let http_method = Method::from_bytes(method.to_ascii_uppercase().as_bytes())
        .map_err(|_| ToolCallError::Execution("Invalid HTTP method".into()))?;
    let mut request = client.request(http_method, url);
    for (key, value) in headers {
        request = request.header(key.as_str(), value.as_str());
    }
    if let Some(secret) = credential {
        request = request.bearer_auth(secret);
    }
    if !body_fields.is_empty() {
        let mut body = Map::new();
        for field in body_fields {
            if let Some(value) = args.get(field) {
                body.insert(field.clone(), value.clone());
            }
        }
        request = request.json(&body);
    }
    let mut response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            ToolCallError::Timeout("API request timed out".into())
        } else {
            ToolCallError::Execution("API request failed".into())
        }
    })?;
    if !response.status().is_success() {
        return Err(ToolCallError::Execution(format!(
            "API returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    loop {
        let next = response.chunk().await.map_err(|error| {
            if error.is_timeout() {
                ToolCallError::Timeout("API response timed out".into())
            } else {
                ToolCallError::Execution("API response failed".into())
            }
        })?;
        let Some(chunk) = next else {
            break;
        };
        if chunk.len() > max_response_bytes.saturating_sub(bytes.len()) {
            return Err(ToolCallError::Execution(
                "API response exceeded the configured size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| ToolCallError::Execution("API response is not UTF-8".into()))?;
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

/// Construct typed arguments for a separately approved container-side executor.
/// No shell command string, native process, or inherited environment is created.
pub fn cli_bridge_arguments(
    definition: &AdapterDefinition,
    args: &Value,
) -> Result<Value, ToolCallError> {
    validate_arguments(&definition.input_schema, args)?;
    let AdapterKind::Cli {
        executable,
        working_directory,
        fixed_args,
        argument_fields,
        timeout_ms,
        max_output_bytes,
        approval_id,
        ..
    } = &definition.adapter
    else {
        return Err(ToolCallError::Execution("Incorrect CLI adapter".into()));
    };
    let mut argv = fixed_args.clone();
    for field in argument_fields {
        if let Some(value) = args.get(field) {
            let arg = primitive_string(value)
                .ok_or_else(|| ToolCallError::InvalidTool("Invalid CLI parameter".into()))?;
            if arg.starts_with('-') || arg.chars().any(char::is_control) {
                return Err(ToolCallError::InvalidTool(
                    "CLI parameter cannot be an option or contain control characters".into(),
                ));
            }
            argv.push(arg);
        }
    }
    Ok(json!({
        "executable": executable, "argv": argv, "cwd": working_directory,
        "timeoutMs": timeout_ms, "maxOutputBytes": max_output_bytes,
        "approvalId": approval_id
    }))
}

pub fn check_cli_output(result: Value, cap: usize) -> Result<Value, ToolCallError> {
    if serde_json::to_vec(&result).map_or(true, |bytes| bytes.len() > cap) {
        return Err(ToolCallError::Execution(
            "CLI output exceeded the configured size limit".into(),
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::db::adapter_repo::{AdapterDefinition, AdapterKind};
    use axum::{routing::post, Router};

    fn base_schema() -> Value {
        json!({"type":"object","properties":{"value":{"type":"string","minLength":2,"maxLength":12}},
            "required":["value"],"additionalProperties":false})
    }

    fn api(url: &str, method: &str) -> AdapterDefinition {
        AdapterDefinition {
            name: "Example API".into(),
            tool_name: "update_value".into(),
            description: "Send a value".into(),
            input_schema: base_schema(),
            risk_level: "write".into(),
            adapter: AdapterKind::Api {
                base_url: url.into(),
                path: "/update".into(),
                method: method.into(),
                headers: Default::default(),
                query_fields: Default::default(),
                body_fields: vec!["value".into()],
                credential_ref: None,
                allow_local_http: true,
                timeout_ms: 500,
                max_response_bytes: 4096,
            },
        }
    }

    #[test]
    fn rejects_invalid_arguments_and_unsupported_schemas() {
        let definition = api("http://127.0.0.1:9388", "POST");
        validate_definition(&definition).unwrap();
        for invalid in [
            json!({}),
            json!({"value":1}),
            json!({"value":"x"}),
            json!({"value":"acceptable", "url":"http://127.0.0.1"}),
        ] {
            assert!(validate_arguments(&definition.input_schema, &invalid).is_err());
        }
        assert!(validate_arguments(&definition.input_schema, &json!({"value":"works"})).is_ok());
        let mut invalid = definition.clone();
        invalid.input_schema["$ref"] = json!("https://example.com/schema");
        assert!(validate_definition(&invalid).is_err());
    }

    #[test]
    fn rejects_open_proxy_and_unapproved_cli_bridges() {
        for url in [
            "http://example.com",
            "http://169.254.169.254",
            "http://224.0.0.1",
            "http://0.0.0.0",
            "http://evil.com",
        ] {
            assert!(validate_definition(&api(url, "POST")).is_err(), "{url}");
        }
        let mut def = api("http://localhost:3000", "POST");
        if let AdapterKind::Api { path, .. } = &mut def.adapter {
            *path = "//169.254.169.254/latest".into();
        }
        assert!(validate_definition(&def).is_err());
        if let AdapterKind::Api { path, headers, .. } = &mut def.adapter {
            *path = "/update".into();
            headers.insert("X-Service-Secret".into(), "plaintext".into());
        }
        assert!(
            validate_definition(&def).is_err(),
            "secrets must use Keychain references"
        );
        if let AdapterKind::Api { headers, .. } = &mut def.adapter {
            headers.clear();
        }
        let mut cli = cli();
        assert!(validate_definition(&cli).is_ok());
        if let AdapterKind::Cli {
            bridge_tool_name, ..
        } = &mut cli.adapter
        {
            *bridge_tool_name = "run_command".into();
        }
        assert!(validate_definition(&cli).is_err());
    }

    fn cli() -> AdapterDefinition {
        AdapterDefinition {
            name: "Approved CLI".into(),
            tool_name: "status".into(),
            description: "Read status".into(),
            input_schema: base_schema(),
            risk_level: "safe".into(),
            adapter: AdapterKind::Cli {
                bridge_server_id: "approved-bridge".into(),
                bridge_tool_name: "execute_approved_cli".into(),
                executable: "/usr/local/bin/status".into(),
                working_directory: "/workplace/project".into(),
                fixed_args: vec!["--json".into()],
                argument_fields: vec!["value".into()],
                approval_id: "owner-approved-status".into(),
                timeout_ms: 1000,
                max_output_bytes: 4096,
            },
        }
    }

    #[test]
    fn cli_constructs_argv_without_shell_interpolation() {
        let def = cli();
        let args = cli_bridge_arguments(&def, &json!({"value":"a; echo unsafe"})).unwrap_err();
        assert!(
            matches!(args, ToolCallError::InvalidTool(_)),
            "overlong text must fail schema first"
        );
        let call = cli_bridge_arguments(&def, &json!({"value":"a;b"})).unwrap();
        assert_eq!(call["argv"], json!(["--json", "a;b"]));
        assert_eq!(call["cwd"], "/workplace/project");
        assert_eq!(call["approvalId"], "owner-approved-status");
    }

    #[tokio::test]
    async fn http_api_maps_json_and_bounds_errors_timeouts_and_response() {
        let app = Router::new().route(
            "/update",
            post(|Json(body): axum::Json<Value>| async move {
                if body["value"] == "slow" {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                if body["value"] == "fail" {
                    return (
                        axum::http::StatusCode::BAD_GATEWAY,
                        Json(json!({"token":"secret"})),
                    );
                }
                (axum::http::StatusCode::OK, Json(body))
            }),
        );
        use axum::Json;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut def = api(&format!("http://{addr}"), "POST");
        let result = execute_api(&def, &json!({"value":"works"}), None)
            .await
            .unwrap();
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("works"));
        let error = execute_api(&def, &json!({"value":"fail"}), None)
            .await
            .unwrap_err();
        assert!(matches!(error, ToolCallError::Execution(_)));
        assert!(!error.to_string().contains("secret"));
        if let AdapterKind::Api { timeout_ms, .. } = &mut def.adapter {
            *timeout_ms = 100;
        }
        assert!(matches!(
            execute_api(&def, &json!({"value":"slow"}), None).await,
            Err(ToolCallError::Timeout(_))
        ));
        if let AdapterKind::Api {
            max_response_bytes,
            timeout_ms,
            ..
        } = &mut def.adapter
        {
            *max_response_bytes = 3;
            *timeout_ms = 500;
        }
        assert!(execute_api(&def, &json!({"value":"works"}), None)
            .await
            .is_err());
        server.abort();
    }
}
