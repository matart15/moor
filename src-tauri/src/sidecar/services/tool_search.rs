//! 工具检索：策展模式下 `moor_search_tools` 的匹配、分页与审计。
//!
//! 协议层只负责参数校验与响应编码（见 CONTEXT.md 的 Server Runtime），
//! 检索逻辑与它产生的 Audit Log 归属都留在这里。

use crate::sidecar::db::profile_repo::ProfileRepository;
use crate::sidecar::db::Database;
use crate::sidecar::services::audit_recorder::{AuditRecorder, ToolCallRecord, ToolCallVia};
use crate::sidecar::services::server_manager::{ServerManager, ToolCallError};
use crate::sidecar::services::tool_catalog::ToolCatalogEntry;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 策展模式下注入的检索元工具。名字只有单个下划线，与 `{slug}__{tool}` 不冲突。
pub const SEARCH_TOOL_NAME: &str = "moor_search_tools";

const DEFAULT_LIMIT: usize = 10;
/// 元工具 schema 与运行时夹取上限共用的上限。
pub const MAX_SEARCH_LIMIT: usize = 50;

pub struct SearchOutcome {
    pub payload: Value,
}

/// 检索可调用工具，并记录一条来源为 `meta` 的审计。
///
/// Active Profile 只在这里读一次：检索池与审计行使用同一个归属，
/// 调用途中切 Profile 不会把这次检索记到新 Profile 上。
#[allow(dead_code)] // Legacy active-profile API.
pub async fn search(
    manager: &ServerManager,
    db: &Database,
    params: &Value,
    agent_info: Option<&str>,
) -> Result<SearchOutcome, ToolCallError> {
    search_for_profile(manager, db, params, agent_info, None).await
}

pub async fn search_for_profile(
    manager: &ServerManager,
    db: &Database,
    params: &Value,
    agent_info: Option<&str>,
    selected_profile: Option<&str>,
) -> Result<SearchOutcome, ToolCallError> {
    let started = std::time::Instant::now();
    let exposure = manager.tool_exposure_for_profile(selected_profile).await;
    let profile_id = exposure
        .as_ref()
        .ok()
        .and_then(|exposure| exposure.profile_id.as_deref());
    // Include only instructions from servers enabled in the selected profile.
    // Match by server ID, not display name (which need not be unique).
    let server_instructions: HashMap<String, String> = exposure
        .as_ref()
        .ok()
        .and_then(|exposure| exposure.profile_id.as_deref())
        .and_then(|id| ProfileRepository::new(db).find_profile_servers(id).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item.profile_server.enabled && !item.server.instructions.trim().is_empty())
        .map(|item| (item.server.id, item.server.instructions))
        .collect();
    let result = exposure
        .as_ref()
        .ok()
        .map(|exposure| search_catalog(&exposure.callable_tools(), params, &server_instructions));
    let recorded_result = result.as_ref().map(|(_, hits)| json!({ "hits": hits }));
    let error = exposure.as_ref().err().map(ToString::to_string);
    AuditRecorder::record(
        db,
        ToolCallRecord {
            profile_id,
            server_id: None,
            tool_name: SEARCH_TOOL_NAME,
            arguments: params,
            result: recorded_result.as_ref(),
            error: error.as_deref(),
            duration_ms: started.elapsed().as_millis() as i64,
            agent_info,
            via: ToolCallVia::Meta,
            refusal: None,
        },
    );
    exposure?;
    Ok(SearchOutcome {
        payload: result.expect("successful exposure has search results").0,
    })
}

/// Match natural-language queries against server names, tool names and descriptions.
/// Prefer complete matches; if the query uses unfamiliar words, fall back to
/// ranked partial matches rather than returning an empty catalog.
fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| part.chars().count() >= 2)
        .map(str::to_lowercase)
        .filter(|term| {
            !matches!(
                term.as_str(),
                "a" | "an"
                    | "the"
                    | "my"
                    | "me"
                    | "i"
                    | "please"
                    | "can"
                    | "you"
                    | "for"
                    | "to"
                    | "in"
                    | "on"
                    | "of"
                    | "and"
                    | "or"
                    | "with"
                    | "about"
                    | "using"
                    | "use"
                    | "do"
                    | "does"
                    | "what"
                    | "is"
            )
        })
        .collect()
}

fn matches_term(text: &str, term: &str) -> bool {
    let aliases: &[&str] = match term {
        "find" | "search" | "lookup" => &["search", "find", "lookup", "query"],
        "doc" | "docs" | "document" | "documents" => &["doc", "page", "file", "note"],
        "show" | "display" => &["list", "get", "read"],
        "check" => &["search", "find", "read", "get"],
        _ => &[],
    };
    text.contains(term) || aliases.iter().any(|alias| text.contains(alias))
}

fn relevance(entry: &ToolCatalogEntry, terms: &[String]) -> (usize, u32) {
    let name = entry.tool_name.to_lowercase();
    let server = entry.server_name.to_lowercase();
    let description = entry.description.as_deref().unwrap_or("").to_lowercase();
    terms.iter().fold((0, 0), |(matched, score), term| {
        let weight = if matches_term(&name, term) {
            6
        } else if matches_term(&server, term) {
            3
        } else if matches_term(&description, term) {
            1
        } else {
            0
        };
        (matched + usize::from(weight != 0), score + weight)
    })
}

/// `cursor` is the offset in the ranked result set; `limit` is capped at 50.
fn search_catalog(
    catalog: &[ToolCatalogEntry],
    params: &Value,
    server_instructions: &HashMap<String, String>,
) -> (Value, usize) {
    let terms = query_terms(params.get("query").and_then(Value::as_str).unwrap_or(""));
    let server_filter = params
        .get("server")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIMIT as u64)
        .clamp(1, MAX_SEARCH_LIMIT as u64) as usize;
    let offset = params
        .get("cursor")
        .and_then(Value::as_str)
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0);

    let mut ranked: Vec<_> = catalog
        .iter()
        .filter(|entry| {
            server_filter
                .as_ref()
                .is_none_or(|server| entry.server_name.to_lowercase().contains(server))
        })
        .map(|entry| {
            let (matched, score) = relevance(entry, &terms);
            (entry, matched, score)
        })
        .filter(|(_, matched, _)| terms.is_empty() || *matched > 0)
        .collect();

    // Keep strict results when they exist, but tolerate task-oriented prompts
    // with extra words that no MCP tool uses in its title or description.
    if !terms.is_empty() && ranked.iter().any(|(_, matched, _)| *matched == terms.len()) {
        ranked.retain(|(_, matched, _)| *matched == terms.len());
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
    let hits = ranked.len();

    let tools: Vec<Value> = ranked
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(entry, _, _)| {
            let mut tool = json!({
                "name": entry.exposed_name,
                "description": entry.description,
                "inputSchema": entry
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| json!({"type": "object"})),
                "serverName": entry.server_name,
                "riskLevel": entry.risk_level,
            });
            if let Some(instructions) = server_instructions.get(&entry.server_id) {
                tool["serverInstructions"] = json!(instructions.trim());
            }
            tool
        })
        .collect();

    let mut payload = json!({ "tools": tools });
    if hits > offset + limit {
        payload["nextCursor"] = json!((offset + limit).to_string());
    }
    (payload, hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool_name: &str, description: Option<&str>) -> ToolCatalogEntry {
        ToolCatalogEntry {
            server_id: "alpha-id".to_string(),
            server_name: "Alpha".to_string(),
            risk_level: "unknown".to_string(),
            tool_name: tool_name.to_string(),
            exposed_name: format!("alpha__{tool_name}"),
            description: description.map(|text| text.to_string()),
            input_schema: None,
        }
    }

    #[test]
    fn complete_matches_are_preferred_over_partial_matches() {
        let catalog = vec![
            entry("search_issues", Some("Find issues in a repository")),
            entry("delete_repo", None),
        ];

        let (payload, hits) = search_catalog(
            &catalog,
            &json!({ "query": "issues repo" }),
            &HashMap::new(),
        );
        assert_eq!(hits, 1);
        assert_eq!(payload["tools"][0]["name"], "alpha__search_issues");
    }

    #[test]
    fn caps_the_limit_and_offers_a_cursor() {
        let catalog: Vec<_> = (0..5)
            .map(|index| entry(&format!("tool{index}"), None))
            .collect();

        let (first, hits) = search_catalog(&catalog, &json!({ "limit": 2 }), &HashMap::new());
        assert_eq!(hits, 5);
        assert_eq!(first["tools"].as_array().expect("tools").len(), 2);
        assert_eq!(first["nextCursor"], "2");

        let cursor = first["nextCursor"].clone();
        let (second, _) = search_catalog(
            &catalog,
            &json!({ "limit": 2, "cursor": cursor }),
            &HashMap::new(),
        );
        assert_eq!(second["tools"][0]["name"], "alpha__tool2");
    }

    #[test]
    fn natural_language_search_finds_the_relevant_server_tool() {
        let catalog = vec![
            ToolCatalogEntry {
                server_id: "notion-id".into(),
                server_name: "Notion".into(),
                exposed_name: "notion__delete_page".into(),
                ..entry("delete_page", Some("Remove a page"))
            },
            ToolCatalogEntry {
                server_id: "notion-id".into(),
                server_name: "Notion".into(),
                exposed_name: "notion__search_pages".into(),
                ..entry("search_pages", Some("Search workspace pages"))
            },
            entry("search_issues", Some("Search GitHub issues")),
        ];
        let (result, hits) = search_catalog(
            &catalog,
            &json!({ "query": "Please find my Notion documents" }),
            &HashMap::new(),
        );
        assert_eq!(hits, 1);
        assert_eq!(result["tools"][0]["name"], "notion__search_pages");
    }

    #[test]
    fn irrelevant_extra_words_fall_back_to_ranked_partial_matches() {
        let catalog = vec![
            entry("delete_page", Some("Remove a page")),
            entry("search_pages", Some("Search pages")),
        ];
        let (result, hits) = search_catalog(
            &catalog,
            &json!({ "query": "find the documents about a nonexistent project" }),
            &HashMap::new(),
        );
        assert_eq!(hits, 2);
        assert_eq!(result["tools"][0]["name"], "alpha__search_pages");
    }

    #[test]
    fn server_instructions_are_scoped_by_id_not_display_name() {
        let catalog = vec![
            entry("search_pages", Some("Find pages")),
            ToolCatalogEntry {
                server_id: "other-id".into(),
                ..entry("search_notes", Some("Find notes"))
            },
        ];
        let instructions = HashMap::from([("alpha-id".to_string(), " Read only. ".to_string())]);
        let (result, hits) = search_catalog(&catalog, &json!({}), &instructions);
        assert_eq!(hits, 2);
        assert_eq!(result["tools"][0]["serverInstructions"], "Read only.");
        assert!(result["tools"][1].get("serverInstructions").is_none());
    }

    #[test]
    fn filters_by_server_name() {
        let catalog = vec![
            entry("search", None),
            ToolCatalogEntry {
                server_name: "Beta".to_string(),
                ..entry("search-beta", None)
            },
        ];

        let (payload, hits) =
            search_catalog(&catalog, &json!({ "server": "beta" }), &HashMap::new());
        assert_eq!(hits, 1);
        assert_eq!(payload["tools"][0]["serverName"], "Beta");
    }
}
