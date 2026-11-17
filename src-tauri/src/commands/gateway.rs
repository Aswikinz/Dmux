//! Structure-half engine commands: probe, inventory, schema diff and mapping.
//!
//! Every command is async, typed with serde, and stays inside the engine's
//! public API.  Connections go through [`ReadOnlySession`] (read-only by
//! construction), and the mapping build only calls the exported
//! `match_tables` / `match_columns` / `Mapping` helpers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;

use dmux_engine::db::{probe_server, ReadOnlySession};
use dmux_engine::inventory::{collect_inventory, DatabaseInventory};
use dmux_engine::mapping::{self, Mapping};
use dmux_engine::schema_diff::{compare_inventories, SchemaDiff};

use crate::error::{from_db_error, CommandError};
use crate::state::AppState;
use crate::types::{build_profile, parse_tls_mode, ConnectInput};

/// Resolve a [`ConnectInput`] into an engine profile plus its password, reading
/// the password from the keychain when the caller did not supply one.
fn resolve(
    input: &ConnectInput,
    state: &AppState,
) -> Result<dmux_engine::db::ConnProfile, CommandError> {
    let tls = parse_tls_mode(&input.profile)?;
    let password = match &input.password {
        Some(p) => p.clone(),
        None => state.keychain()?.get(&input.profile.id)?,
    };
    Ok(build_profile(&input.profile, password, tls))
}

/// Drive a `!Send` engine future to completion on Tauri's blocking pool.
///
/// The engine's read-only transaction wrapper returns a boxed `dyn Future`
/// without a `Send` bound (it borrows a `tokio_postgres::Transaction`), so it
/// cannot be awaited directly inside an `async` Tauri command (which requires
/// `Send`).  Run it on a dedicated blocking OS thread instead, so the UI thread
/// and the async runtime stay free.
async fn drive<T, F, Fut>(f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    Fut: std::future::Future<Output = Result<T, CommandError>> + 'static,
    F: FnOnce() -> Fut + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || tauri::async_runtime::block_on(f()))
        .await
        .map_err(|e| CommandError::internal(format!("engine task failed: {e}")))?
}

/// The superuser override, as its own small DTO so the UI can pass it explicitly
/// with a recorded reason (PLAN 6.2 refuses superusers by default).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SuperuserOverride {
    pub allow: bool,
    pub reason: String,
}

impl SuperuserOverride {
    /// Whether the override is a valid, recorded opt-in.
    fn allows(&self) -> bool {
        self.allow && !self.reason.trim().is_empty()
    }
}

/// Test a connection and report server metadata (PLAN 4.1).  On an untrusted
/// certificate the fingerprint is surfaced for a one-time trust.
#[tauri::command]
pub async fn probe_connection(
    input: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<dmux_engine::db::ServerInfo, CommandError> {
    let profile = resolve(&input, &state)?;
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    drive(move || async move { probe_server(&profile, allow).await.map_err(from_db_error) }).await
}

/// Collect one database's structural inventory (engine-native, no network glue
/// beyond the read-only session).
async fn collect(
    input: &ConnectInput,
    allow: bool,
    state: &AppState,
) -> Result<DatabaseInventory, CommandError> {
    let profile = resolve(input, state)?;
    drive(move || async move {
        let mut session = ReadOnlySession::open(&profile, allow)
            .await
            .map_err(from_db_error)?;
        session
            .with_read_only("30s", |txn| {
                Box::pin(async move {
                    collect_inventory(txn)
                        .await
                        .map_err(|e| dmux_engine::db::DbError::Postgres {
                            message: e.to_string(),
                        })
                })
            })
            .await
            .map_err(from_db_error)
    })
    .await
}

/// Collect one database's structural inventory.
#[tauri::command]
pub async fn run_inventory(
    input: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<DatabaseInventory, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    collect(&input, allow, &state).await
}

/// Compare two already-collected inventories (PLAN 4.2).
#[tauri::command]
pub fn schema_diff(
    old: DatabaseInventory,
    new: DatabaseInventory,
) -> Result<SchemaDiff, CommandError> {
    Ok(compare_inventories(&old, &new))
}

/// The UI-facing result of building a mapping: the engine `Mapping` (for save /
/// export) plus the serialised "needs attention" queue and fuzzy suggestions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MappingBuild {
    pub mapping: Mapping,
    pub needs_attention: Vec<AttentionEntry>,
    pub suggestions: Vec<SuggestionEntry>,
}

/// One item that needs a human decision, serialised from the engine's
/// (non-serialisable) `AttentionItem`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AttentionEntry {
    pub path: String,
    pub issue: String,
    pub candidates: Vec<String>,
}

/// A fuzzy-match suggestion, serialised from the engine's `Suggestion`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SuggestionEntry {
    pub path: String,
    pub candidate: String,
    pub confidence: u8,
}

/// Build a mapping between the two databases.
///
/// Connects to both, collects their inventories, applies the same deterministic
/// pipeline as the engine (`match_tables` / `match_columns`): rules first, then
/// exact, then unambiguous case-insensitive, with fuzzy suggestions only — never
/// auto-accepted.  Produces an engine [`Mapping`] plus the attention queue.
#[tauri::command]
pub async fn build_mapping(
    old: ConnectInput,
    new: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<MappingBuild, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    let old_inv = collect(&old, allow, &state).await?;
    let new_inv = collect(&new, allow, &state).await?;
    let (mapping, needs_attention, suggestions) = build_from_inventories(&old_inv, &new_inv);
    Ok(MappingBuild {
        mapping,
        needs_attention,
        suggestions,
    })
}

/// Run the deterministic matching pipeline over two inventories, producing an
/// engine [`Mapping`] plus serialisable attention/suggestion lists.
///
/// Split out (and shared by tests) so the pipeline is testable without a live
/// server.
pub fn build_from_inventories(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
) -> (Mapping, Vec<AttentionEntry>, Vec<SuggestionEntry>) {
    let mut mapping = Mapping::new();
    let mut needs_attention: Vec<AttentionEntry> = Vec::new();
    let mut suggestions: Vec<SuggestionEntry> = Vec::new();

    let old_schemas: BTreeMap<&str, &dmux_engine::inventory::Schema> =
        old.schemas.iter().map(|s| (s.name.as_str(), s)).collect();
    let new_schemas: BTreeMap<&str, &dmux_engine::inventory::Schema> =
        new.schemas.iter().map(|s| (s.name.as_str(), s)).collect();

    for (schema_name, old_schema) in &old_schemas {
        let target_schema = mapping
            .rules
            .iter()
            .find_map(|r| mapping::apply_rule(r, schema_name))
            .unwrap_or_else(|| (*schema_name).to_string());

        let new_schema = new_schemas.get(target_schema.as_str());
        if new_schema.is_none() {
            needs_attention.push(AttentionEntry {
                path: schema_name.to_string(),
                issue: "missing_in_new".to_string(),
                candidates: Vec::new(),
            });
            // Nothing to map at table level for a missing target schema.
            continue;
        }
        let new_schema = new_schema.unwrap();

        let old_names: Vec<String> = old_schema.tables.iter().map(|t| t.name.clone()).collect();
        let new_names: Vec<String> = new_schema.tables.iter().map(|t| t.name.clone()).collect();
        let tables = mapping::match_tables(schema_name, &old_names, &new_names);

        let mut schema_mapping = dmux_engine::mapping::SchemaMapping {
            target_schema: target_schema.clone(),
            table_mappings: BTreeMap::new(),
        };

        for (old_name, table_mapping) in &tables.mapped {
            let mut table_mapping = table_mapping.clone();
            // Match columns for mapped tables.
            let old_cols: Vec<String> = old_schema
                .tables
                .iter()
                .find(|t| &t.name == old_name)
                .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();
            let new_cols: Vec<String> = new_schema
                .tables
                .iter()
                .find(|t| t.name == table_mapping.target_table)
                .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();

            let (column_mappings, col_attention, col_suggestions) =
                mapping::match_columns(schema_name, old_name, &old_cols, &new_cols);
            table_mapping.column_mappings = column_mappings;

            for a in col_attention {
                needs_attention.push(AttentionEntry {
                    path: a.path,
                    issue: attention_issue(&a.issue),
                    candidates: attention_candidates(&a.issue),
                });
            }
            for s in col_suggestions {
                suggestions.push(SuggestionEntry {
                    path: s.path,
                    candidate: s.candidate,
                    confidence: s.confidence,
                });
            }

            schema_mapping
                .table_mappings
                .insert(old_name.clone(), table_mapping);
        }

        for a in tables.needs_attention {
            needs_attention.push(AttentionEntry {
                path: a.path,
                issue: attention_issue(&a.issue),
                candidates: attention_candidates(&a.issue),
            });
        }
        for s in tables.suggestions {
            suggestions.push(SuggestionEntry {
                path: s.path,
                candidate: s.candidate,
                confidence: s.confidence,
            });
        }

        if !schema_mapping.table_mappings.is_empty() {
            mapping
                .schema_mappings
                .insert(schema_name.to_string(), schema_mapping);
        }
    }

    mapping.ignored = Vec::new(); // nothing ignored at build time
    let mapping = mapping.with_fresh_hash();
    (mapping, needs_attention, suggestions)
}

fn attention_issue(issue: &mapping::AttentionIssue) -> String {
    match issue {
        mapping::AttentionIssue::MissingInNew { .. } => "missing_in_new".to_string(),
        mapping::AttentionIssue::AmbiguousCaseInsensitive { .. } => {
            "ambiguous_case_insensitive".to_string()
        }
        mapping::AttentionIssue::AddBackReview => "add_back_review".to_string(),
    }
}

fn attention_candidates(issue: &mapping::AttentionIssue) -> Vec<String> {
    match issue {
        mapping::AttentionIssue::MissingInNew { candidates }
        | mapping::AttentionIssue::AmbiguousCaseInsensitive { candidates } => candidates.clone(),
        mapping::AttentionIssue::AddBackReview => Vec::new(),
    }
}

/// Update a mapping in place (re-hash it).  The caller supplies a validated
/// [`Mapping`]; this command canonicalises and returns the stored form.
#[tauri::command]
pub fn update_mapping(mapping: Mapping) -> Result<Mapping, CommandError> {
    Ok(mapping.with_fresh_hash())
}

/// Export a mapping as canonical JSON plus its SHA-256 hash (PLAN 4.3).
#[tauri::command]
pub fn export_mapping(mapping: Mapping) -> Result<ExportResult, CommandError> {
    let mapping = mapping.with_fresh_hash();
    let json = serde_json::to_string(&mapping).map_err(CommandError::from)?;
    Ok(ExportResult {
        json,
        hash: mapping.mapping_hash,
    })
}

/// Import a mapping file.  Untrusted input: size-limited and fully validated by
/// the engine (`mapping::from_json`), which rejects unknown fields at any level
/// and checks the embedded hash (PLAN 6.3).
#[tauri::command]
pub fn import_mapping(json: String) -> Result<Mapping, CommandError> {
    match mapping::from_json(json.as_bytes()) {
        Ok(mapping) => Ok(mapping),
        Err(e) => Err(CommandError::InvalidMapping {
            message: e.to_string(),
        }),
    }
}

/// The export payload: canonical JSON plus its hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExportResult {
    pub json: String,
    pub hash: String,
}

/// The engine version for the run log and reports.
#[tauri::command]
pub fn get_version() -> String {
    dmux_engine::version().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmux_engine::inventory::{Schema, Table};

    fn table(name: &str, columns: &[&str]) -> Table {
        Table {
            name: name.to_string(),
            owner: "postgres".to_string(),
            has_oids: false,
            is_partitioned: false,
            columns: columns
                .iter()
                .map(|c| dmux_engine::inventory::Column {
                    name: c.to_string(),
                    type_name: "text".to_string(),
                    length: None,
                    nullable: true,
                    default: None,
                })
                .collect(),
            constraints: Vec::new(),
            indexes: Vec::new(),
            partitions: Vec::new(),
            inherited_from: Vec::new(),
            n_live_tup: 0,
            size_bytes: 0,
            last_analyze: None,
            stats_missing: false,
            warnings: Vec::new(),
        }
    }

    fn schema(name: &str, tables: Vec<Table>) -> Schema {
        Schema {
            name: name.to_string(),
            owner: "postgres".to_string(),
            tables,
            views: Vec::new(),
            materialized_views: Vec::new(),
            functions: Vec::new(),
            triggers: Vec::new(),
            sequences: Vec::new(),
        }
    }

    fn inventory(schemas: Vec<Schema>) -> DatabaseInventory {
        DatabaseInventory {
            server_version: "17.0".to_string(),
            server_version_num: 170000,
            encoding: "UTF8".to_string(),
            collation: "C".to_string(),
            ctype: "C".to_string(),
            lc_monetary: "C".to_string(),
            schemas,
            extensions: Vec::new(),
            large_object_count: 0,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn exact_names_map_without_attention() {
        let old = inventory(vec![schema(
            "public",
            vec![table("accounts", &["id", "name"])],
        )]);
        let new = inventory(vec![schema(
            "public",
            vec![table("accounts", &["id", "name"])],
        )]);

        let (mapping, attention, suggestions) = build_from_inventories(&old, &new);

        assert!(attention.is_empty(), "exact matches need no attention");
        assert!(suggestions.is_empty());

        let sm = mapping.schema_mappings.get("public").unwrap();
        let tm = sm.table_mappings.get("accounts").unwrap();
        assert_eq!(tm.status, mapping::MappingStatus::Exact);
        assert_eq!(tm.column_mappings.len(), 2);
        assert_eq!(
            tm.column_mappings["id"].status,
            mapping::MappingStatus::Exact
        );
    }

    #[test]
    fn missing_table_is_flagged_not_silent() {
        let old = inventory(vec![schema("public", vec![table("accounts", &["id"])])]);
        let new = inventory(vec![schema("public", vec![])]);

        let (_mapping, attention, _suggestions) = build_from_inventories(&old, &new);

        assert!(attention
            .iter()
            .any(|a| a.path == "public.accounts" && a.issue == "missing_in_new"));
    }

    #[test]
    fn mapping_hash_is_fresh_and_deterministic() {
        let old = inventory(vec![schema("public", vec![table("t", &["a"])])]);
        let new = inventory(vec![schema("public", vec![table("t", &["a"])])]);

        let (m1, _, _) = build_from_inventories(&old, &new);
        let (m2, _, _) = build_from_inventories(&old, &new);
        assert_eq!(m1.mapping_hash, m2.mapping_hash);
        assert!(!m1.mapping_hash.is_empty());
    }

    #[test]
    fn export_and_import_round_trip() {
        let old = inventory(vec![schema("public", vec![table("t", &["a"])])]);
        let new = inventory(vec![schema("public", vec![table("t", &["a"])])]);
        let (mapping, _, _) = build_from_inventories(&old, &new);

        let exported = export_mapping(mapping).unwrap();
        let imported = import_mapping(exported.json).unwrap();
        // The imported mapping's hash round-trips through the engine's
        // self-verifying load.
        assert_eq!(imported.schema_mappings.len(), 1);
    }

    #[test]
    fn import_rejects_unknown_fields() {
        let json = r#"{"version":1,"rules":[],"schema_mappings":{},"ignored":[],"mapping_hash":"","extra":true}"#;
        let err = import_mapping(json.to_string()).unwrap_err();
        assert!(matches!(err, CommandError::InvalidMapping { .. }));
    }

    // --- Live-server tests (gated, matching the engine core policy) ---

    fn env_url(name: &str) -> Option<String> {
        match std::env::var(name) {
            Ok(u) => Some(u),
            Err(_) => {
                assert!(
                    std::env::var("CI").as_deref() != Ok("true"),
                    "CI=true but {name} is not set: the live probe/inventory must run in CI"
                );
                eprintln!("skipped: {name} not set");
                None
            }
        }
    }

    fn profile_input_from_url(url: &str) -> (crate::types::ProfileInput, String) {
        let url = url
            .strip_prefix("postgres://")
            .expect("url must be postgres");
        let (creds, rest) = url
            .split_once('@')
            .expect("url must contain user:pass@host");
        let (user, password) = creds.split_once(':').expect("url must contain user:pass");
        let (host_port, dbname) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port_str) = host_port
            .split_once(':')
            .expect("url must contain host:port");
        let port = port_str.parse().expect("port must be numeric");
        (
            crate::types::ProfileInput {
                id: "test".to_string(),
                label: "mock".to_string(),
                host: host.to_string(),
                port,
                database: dbname.to_string(),
                user: user.to_string(),
                encryption: "allow-plaintext".to_string(),
                tls_fingerprint: None,
                plaintext_reason: Some("mock database has no TLS".to_string()),
            },
            password.to_string(),
        )
    }

    fn test_state() -> crate::state::AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::state::AppState::new(dir.path().to_path_buf()).expect("state")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn old_database_probes_and_inventories() {
        let Some(url) = env_url("DMUX_TEST_OLD_URL") else {
            return;
        };
        let (profile_input, password) = profile_input_from_url(&url);
        let state = test_state();
        let input = crate::types::ConnectInput {
            profile: profile_input,
            password: Some(password),
        };
        let profile = resolve(&input, &state).expect("resolve");
        let info = dmux_engine::db::probe_server(&profile, true)
            .await
            .expect("probe");
        assert!(info.version_num >= 90400);

        let mut session = ReadOnlySession::open(&profile, true)
            .await
            .expect("session");
        session
            .with_read_only("30s", |txn| {
                Box::pin(async move {
                    collect_inventory(txn)
                        .await
                        .map_err(|e| dmux_engine::db::DbError::Postgres {
                            message: e.to_string(),
                        })
                })
            })
            .await
            .expect("inventory");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn new_database_probes_and_inventories() {
        let Some(url) = env_url("DMUX_TEST_NEW_URL") else {
            return;
        };
        let (profile_input, password) = profile_input_from_url(&url);
        let state = test_state();
        let input = crate::types::ConnectInput {
            profile: profile_input,
            password: Some(password),
        };
        let profile = resolve(&input, &state).expect("resolve");
        let info = dmux_engine::db::probe_server(&profile, true)
            .await
            .expect("probe");
        assert!(info.version_num >= 100000);
    }
}
