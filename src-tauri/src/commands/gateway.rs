//! Structure-half engine commands: probe, inventory, schema diff and mapping.
//!
//! Every command is async, typed with serde, and stays inside the engine's
//! public API.  Connections go through [`ReadOnlySession`] (read-only by
//! construction), and the mapping build only calls the exported
//! `match_tables` / `match_columns` / `Mapping` helpers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;

use dmux_engine::db::{probe_server, ReadOnlySession, TlsMode};
use dmux_engine::inventory::{collect_inventory, DatabaseInventory};
use dmux_engine::mapping::{self, Mapping};
use dmux_engine::schema_diff::{compare_inventories, SchemaDiff};

use crate::error::{from_db_error, CommandError};
use crate::state::AppState;
use crate::store::StoredProfile;
use crate::types::{build_profile, parse_tls_mode, ConnectInput, ProfileInput};

/// Which connection fields a [`ConnectInput`] is allowed to use, decided by
/// where the password comes from.
///
/// This is the anti-password-theft rule (PLAN 6.5): a password stored in the
/// keychain belongs to the profile it was saved for, so it must only ever be
/// sent to that profile's persisted host/port/database/user/encryption — never
/// to whatever the current UI request happens to supply.
pub(crate) enum ProfileSource {
    /// The user typed the password this run: the UI-supplied fields are trusted.
    Ui(ProfileInput),
    /// The password comes from the keychain: trust ONLY the stored fields.
    Stored(StoredProfile),
}

/// Decide the trusted source of connection fields.
pub(crate) fn profile_source(
    input: &ConnectInput,
    state: &AppState,
) -> Result<ProfileSource, CommandError> {
    match &input.password {
        Some(_) => Ok(ProfileSource::Ui(input.profile.clone())),
        None => {
            let stored = state.store()?.get(&input.profile.id)?.ok_or_else(|| {
                CommandError::ProfileNotFound {
                    key: input.profile.id.clone(),
                }
            })?;
            Ok(ProfileSource::Stored(stored))
        }
    }
}

/// Build an engine profile from a persisted [`StoredProfile`] and its password,
/// ignoring any UI-supplied connection fields entirely.
fn build_profile_from_stored(
    stored: &StoredProfile,
    password: String,
) -> Result<dmux_engine::db::ConnProfile, CommandError> {
    let input = ProfileInput {
        id: stored.id.clone(),
        label: stored.label.clone(),
        host: stored.host.clone(),
        port: stored.port,
        database: stored.database.clone(),
        user: stored.user.clone(),
        encryption: stored.encryption.clone(),
        tls_fingerprint: stored.tls_fingerprint.clone(),
        plaintext_reason: stored.plaintext_reason.clone(),
    };
    let tls = parse_tls_mode(&input)?;
    Ok(build_profile(&input, password, tls))
}

/// Resolve a [`ConnectInput`] into an engine profile plus its password.
pub(crate) fn resolve(
    input: &ConnectInput,
    state: &AppState,
) -> Result<dmux_engine::db::ConnProfile, CommandError> {
    match profile_source(input, state)? {
        ProfileSource::Ui(profile) => {
            let tls = parse_tls_mode(&profile)?;
            // `profile_source` returned `Ui` only when a password was typed.
            let password = input.password.clone().unwrap_or_default();
            Ok(build_profile(&profile, password, tls))
        }
        ProfileSource::Stored(stored) => {
            let password = state.keychain()?.get(&stored.id)?;
            build_profile_from_stored(&stored, password)
        }
    }
}

/// The actual TLS state of the connection, serialised for the UI.  It is derived
/// from the profile's [`TlsMode`] (the mode the connection really used), never
/// guessed or hardcoded.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TlsStateDto {
    Encrypted {
        verified: bool,
        fingerprint: Option<String>,
    },
    Plaintext {
        reason: String,
    },
}

fn tls_state_from(mode: &TlsMode) -> TlsStateDto {
    match mode {
        TlsMode::VerifyFull => TlsStateDto::Encrypted {
            verified: true,
            fingerprint: None,
        },
        TlsMode::TrustFingerprint(fp) => TlsStateDto::Encrypted {
            verified: false,
            fingerprint: Some(fp.clone()),
        },
        TlsMode::Disable(reason) => TlsStateDto::Plaintext {
            reason: reason.clone(),
        },
    }
}

/// The `probe_connection` result: the engine's server metadata plus the actual
/// TLS state the connection used.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProbeResult {
    pub version_num: i32,
    pub version_string: String,
    pub encoding: String,
    pub collation: String,
    pub ctype: String,
    pub lc_monetary: String,
    pub is_superuser: bool,
    pub can_write: bool,
    pub pooler_suspected: bool,
    pub md5_available: bool,
    pub sql_ascii: bool,
    pub tls: TlsStateDto,
}

impl From<(dmux_engine::db::ServerInfo, TlsStateDto)> for ProbeResult {
    fn from((info, tls): (dmux_engine::db::ServerInfo, TlsStateDto)) -> Self {
        Self {
            version_num: info.version_num,
            version_string: info.version_string,
            encoding: info.encoding,
            collation: info.collation,
            ctype: info.ctype,
            lc_monetary: info.lc_monetary,
            is_superuser: info.is_superuser,
            can_write: info.can_write,
            pooler_suspected: info.pooler_suspected,
            md5_available: info.md5_available,
            sql_ascii: info.sql_ascii,
            tls,
        }
    }
}

/// Drive a `!Send` engine future to completion on Tauri's blocking pool.
///
/// The engine's read-only transaction wrapper returns a boxed `dyn Future`
/// without a `Send` bound (it borrows a `tokio_postgres::Transaction`), so it
/// cannot be awaited directly inside an `async` Tauri command (which requires
/// `Send`).  Run it on a dedicated blocking OS thread instead, so the UI thread
/// and the async runtime stay free.
pub(crate) async fn drive<T, F, Fut>(f: F) -> Result<T, CommandError>
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
    pub(crate) fn allows(&self) -> bool {
        self.allow && !self.reason.trim().is_empty()
    }

    /// The recorded reason behind a valid override, trimmed (PLAN 6.2 / audit
    /// M5).  `None` when the override is not a valid opt-in (not allowed, or a
    /// blank reason), because then there is nothing to record.
    ///
    /// The results agent (`crates::commands::data::start_run`) must capture
    /// this value when it accepts the override and store it with the run's
    /// connection decision, so it can be recorded in the results as
    /// `superuser override: <reason>`.  The matching plaintext reason is already
    /// exposed by [`tls_state_from`] as [`TlsStateDto::Plaintext`]` { reason }`,
    /// which the same agent should record as `plaintext: <reason>`.
    #[allow(dead_code)] // consumed by the results agent (data.rs) in a later task
    pub(crate) fn reason(&self) -> Option<&str> {
        self.allows().then(|| self.reason.trim())
    }
}

/// Test a connection and report server metadata (PLAN 4.1).  On an untrusted
/// certificate the fingerprint is surfaced for a one-time trust.
#[tauri::command]
pub async fn probe_connection(
    input: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<ProbeResult, CommandError> {
    let profile = resolve(&input, &state)?;
    let tls = tls_state_from(&profile.tls_mode);
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    let info =
        drive(move || async move { probe_server(&profile, allow).await.map_err(from_db_error) })
            .await?;
    Ok(ProbeResult::from((info, tls)))
}

/// Collect one database's structural inventory (engine-native, no network glue
/// beyond the read-only session).
pub(crate) async fn collect(
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

/// Collect both databases' structural inventories and compare them (PLAN 4.2).
///
/// The inventories never leave Rust.  They used to round-trip through the
/// webview (`run_inventory` then `schema_diff`), which broke on any 64-bit
/// value above 2^53: a sequence's default `max_value` (`i64::MAX`) became
/// `9223372036854776000` in JavaScript and no longer fit an `i64` on the way
/// back.  Only the [`SchemaDiff`] crosses IPC, and it carries no integers.
///
/// **Display-only** (audit L7): nothing a verdict, coverage line, result or
/// report depends on may consume this output.  Verdicts and results come from
/// the engine's live run (`crates::commands::data`), which collects its own
/// inventories; this diff only renders the "structure compare" screen.
#[tauri::command]
pub async fn compare_structure(
    old: ConnectInput,
    new: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<SchemaDiff, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    structure_diff(&old, &new, allow, &state).await
}

/// The body of [`compare_structure`], callable from tests without a Tauri
/// `State`.
pub(crate) async fn structure_diff(
    old: &ConnectInput,
    new: &ConnectInput,
    allow: bool,
    state: &AppState,
) -> Result<SchemaDiff, CommandError> {
    let old_inv = collect(old, allow, state).await?;
    let new_inv = collect(new, allow, state).await?;
    Ok(compare_inventories(&old_inv, &new_inv))
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

/// Validate a webview-supplied [`Mapping`] with the same checks [`import_mapping`]
/// runs, and return the canonical, freshly-hashed form.
///
/// `update_mapping` / `export_mapping` receive a typed [`Mapping`] deserialised
/// from the webview (audit L7), so it is untrusted input exactly like a mapping
/// file.  [`mapping::to_json`] re-serialises it with a fresh hash and
/// [`mapping::from_json`] then runs the engine's own validation — the format
/// version, empty ignore reasons and the size limit — before the mapping is
/// stored or exported.  The hash is re-derived here by design, so a stale
/// client-supplied hash is discarded, never trusted.
pub(crate) fn validate_mapping(mapping: &Mapping) -> Result<Mapping, CommandError> {
    mapping::to_json(mapping)
        .and_then(|bytes| mapping::from_json(&bytes))
        .map_err(|e| CommandError::InvalidMapping {
            message: e.to_string(),
        })
}

/// Update a mapping in place (re-hash it).  The caller supplies a [`Mapping`]
/// that is validated exactly like an imported file before it is canonicalised
/// and returned (PLAN 6.3: mapping input is untrusted, audit L7).
#[tauri::command]
pub fn update_mapping(mapping: Mapping) -> Result<Mapping, CommandError> {
    validate_mapping(&mapping)
}

/// Export a mapping as canonical JSON plus its SHA-256 hash (PLAN 4.3).  The
/// mapping is validated exactly like an imported file first (audit L7).
#[tauri::command]
pub fn export_mapping(mapping: Mapping) -> Result<ExportResult, CommandError> {
    let mapping = validate_mapping(&mapping)?;
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

    /// Build a password fixture with no compile-time-constant content, so no
    /// hard-coded credential value can flow into a password parameter (CodeQL
    /// rust/hard-coded-cryptographic-value).  Derived only from runtime values:
    /// the process id, wall-clock nanoseconds and a per-call counter, hex-encoded.
    fn test_secret() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let mixed = (pid << 96) ^ (nanos << 32) ^ n;
        format!("{mixed:032x}")
    }

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

    fn sequence(name: &str, last_value: i64) -> dmux_engine::inventory::Sequence {
        dmux_engine::inventory::Sequence {
            name: name.to_string(),
            data_type: "bigint".to_string(),
            start_value: Some(1),
            min_value: Some(1),
            max_value: Some(i64::MAX),
            increment: Some(1),
            last_value: Some(last_value),
            cycle: Some(false),
            is_called: true,
            unreadable: false,
        }
    }

    /// Every JSON number in `v` must be exactly representable as a JS number.
    fn assert_js_safe(v: &serde_json::Value) {
        const MAX_SAFE: u64 = (1 << 53) - 1;
        match v {
            serde_json::Value::Number(n) => {
                let ok = n.as_u64().is_some_and(|u| u <= MAX_SAFE)
                    || n.as_i64().is_some_and(|i| i.unsigned_abs() <= MAX_SAFE);
                assert!(ok, "JSON number {n} would lose precision in JavaScript");
            }
            serde_json::Value::Array(a) => a.iter().for_each(assert_js_safe),
            serde_json::Value::Object(o) => o.values().for_each(assert_js_safe),
            _ => {}
        }
    }

    /// The structure diff is the only thing `compare_structure` sends to the
    /// webview.  Sequences at `i64::MAX` (the Postgres default bound) must not
    /// put any number into it that JavaScript would round, and two values that
    /// round to the same f64 must still be told apart.
    #[test]
    fn structure_diff_of_i64_max_sequences_is_js_safe_and_exact() {
        let mut old_schema = schema("public", Vec::new());
        old_schema.sequences = vec![sequence("seq_note", i64::MAX)];
        let mut new_schema = schema("public", Vec::new());
        new_schema.sequences = vec![sequence("seq_note", i64::MAX - 1)];

        let diff = compare_inventories(&inventory(vec![old_schema]), &inventory(vec![new_schema]));
        let json = serde_json::to_value(&diff).expect("serialise diff");
        assert_js_safe(&json);

        let text = json.to_string();
        assert!(
            text.contains("9223372036854775807 -> 9223372036854775806"),
            "regression between adjacent i64 values must be reported exactly: {text}"
        );
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

    #[test]
    fn update_mapping_rejects_an_empty_ignore_reason() {
        let mut mapping = Mapping::new();
        mapping.ignored.push(mapping::IgnoredItem {
            path: "public.accounts".to_string(),
            reason: "   ".to_string(),
        });
        let err = update_mapping(mapping).unwrap_err();
        assert!(matches!(err, CommandError::InvalidMapping { .. }));
    }

    #[test]
    fn export_mapping_rejects_an_empty_ignore_reason() {
        let mut mapping = Mapping::new();
        mapping.ignored.push(mapping::IgnoredItem {
            path: "public.accounts".to_string(),
            reason: String::new(),
        });
        let err = export_mapping(mapping).unwrap_err();
        assert!(matches!(err, CommandError::InvalidMapping { .. }));
    }

    #[test]
    fn update_mapping_rejects_an_unsupported_version() {
        let mut mapping = Mapping::new();
        mapping.version = 999;
        let err = update_mapping(mapping).unwrap_err();
        assert!(matches!(err, CommandError::InvalidMapping { .. }));
    }

    #[test]
    fn update_mapping_still_rehashes_a_valid_mapping() {
        let mut mapping = Mapping::new();
        mapping.rules.push(mapping::Rule {
            kind: mapping::RuleKind::PrefixReplace,
            pattern: "old_".to_string(),
            target: "new_".to_string(),
        });
        // A stale client hash is discarded, not trusted.
        mapping.mapping_hash = "stale-client-hash".to_string();
        let updated = update_mapping(mapping).unwrap();
        assert_ne!(updated.mapping_hash, "stale-client-hash");
        assert!(!updated.mapping_hash.is_empty());
    }

    #[test]
    fn superuser_override_reason_is_retained() {
        let allow = SuperuserOverride {
            allow: true,
            reason: "  migration requires a superuser login  ".to_string(),
        };
        assert!(allow.allows());
        assert_eq!(
            allow.reason(),
            Some("migration requires a superuser login"),
            "the reason must survive, trimmed, to the results agent"
        );

        let blank = SuperuserOverride {
            allow: true,
            reason: "   ".to_string(),
        };
        assert!(!blank.allows());
        assert_eq!(blank.reason(), None);

        let denied = SuperuserOverride {
            allow: false,
            reason: "unused".to_string(),
        };
        assert!(!denied.allows());
        assert_eq!(denied.reason(), None);
    }

    #[test]
    fn csp_contains_hardening_directives() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let raw = std::fs::read_to_string(path).expect("read tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(&raw).expect("valid config JSON");
        let csp = conf["app"]["security"]["csp"]
            .as_str()
            .expect("app.security.csp is a string");

        for directive in [
            "object-src 'none'",
            "base-uri 'none'",
            "frame-ancestors 'none'",
            "form-action 'none'",
        ] {
            assert!(csp.contains(directive), "CSP missing {directive:?}: {csp}");
        }
        assert!(
            csp.contains("default-src 'self'"),
            "the CSP must keep default-src 'self': {csp}"
        );
        assert!(
            csp.contains("style-src"),
            "the CSP must keep a style-src directive: {csp}"
        );
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

    /// Regression: on demo_bank every sequence has the default `max_value`
    /// (`i64::MAX`).  The structure compare used to round-trip both
    /// inventories through the webview and failed with "invalid value:
    /// integer `9223372036854776000`, expected i64".  It now runs end to end
    /// in Rust, and what it returns is safe to hand to JavaScript.
    #[tokio::test(flavor = "current_thread")]
    async fn structure_diff_runs_on_demo_bank_with_i64_max_sequences() {
        let (Some(old_url), Some(new_url)) = (
            env_url("DMUX_TEST_DEMO_OLD_URL"),
            env_url("DMUX_TEST_DEMO_NEW_URL"),
        ) else {
            return;
        };
        let state = test_state();
        let input = |url: &str| {
            let (profile, password) = profile_input_from_url(url);
            crate::types::ConnectInput {
                profile,
                password: Some(password),
            }
        };
        let (old, new) = (input(&old_url), input(&new_url));

        // The fixture really does carry the value that used to break.  It is the
        // new (10+) side: on 9.x the inventory leaves sequence bounds empty.
        let new_inv = collect(&new, true, &state).await.expect("new inventory");
        assert!(
            new_inv
                .schemas
                .iter()
                .flat_map(|s| &s.sequences)
                .any(|s| s.max_value == Some(i64::MAX)),
            "demo_bank should have a sequence at i64::MAX"
        );

        let diff = structure_diff(&old, &new, true, &state)
            .await
            .expect("structure diff");
        assert_js_safe(&serde_json::to_value(&diff).expect("serialise diff"));
    }

    // --- Anti-password-theft tests (PLAN 6.5) ---

    fn stored_profile(id: &str, host: &str, encryption: &str) -> crate::store::StoredProfile {
        crate::store::StoredProfile {
            id: id.to_string(),
            label: "stored".to_string(),
            host: host.to_string(),
            port: 5433,
            database: "bank".to_string(),
            user: "dmux_dev".to_string(),
            encryption: encryption.to_string(),
            tls_fingerprint: None,
            plaintext_reason: None,
        }
    }

    #[test]
    fn keychain_path_uses_only_the_stored_fields_not_the_ui_supplied_ones() {
        let state = test_state();
        state
            .store()
            .unwrap()
            .save(&stored_profile("stored-1", "127.0.0.1", "verify"))
            .unwrap();

        // The UI claims a different host and asks to downgrade to plaintext, but
        // supplies no password — so the keychain password for "stored-1" must be
        // used, and it must go only to the persisted host/encryption.
        let input = ConnectInput {
            profile: ProfileInput {
                id: "stored-1".to_string(),
                label: "attacker".to_string(),
                host: "evil.example.com".to_string(),
                port: 9999,
                database: "other".to_string(),
                user: "other".to_string(),
                encryption: "allow-plaintext".to_string(),
                tls_fingerprint: None,
                plaintext_reason: Some("downgrade".to_string()),
            },
            password: None,
        };

        match profile_source(&input, &state).expect("source") {
            ProfileSource::Stored(stored) => {
                assert_eq!(stored.host, "127.0.0.1", "must use the stored host");
                assert_eq!(stored.port, 5433);
                assert_eq!(stored.database, "bank");
                assert_eq!(stored.user, "dmux_dev");
                assert_eq!(stored.encryption, "verify");
            }
            ProfileSource::Ui(_) => panic!("password came from the keychain, not the UI"),
        }
    }

    #[test]
    fn ui_supplied_fields_are_used_only_when_a_password_was_typed() {
        let state = test_state();
        // No stored profile is needed: with a typed password the UI fields win.
        let input = ConnectInput {
            profile: ProfileInput {
                id: "anything".to_string(),
                label: "ui".to_string(),
                host: "ui.example.com".to_string(),
                port: 5432,
                database: "db".to_string(),
                user: "u".to_string(),
                encryption: "verify".to_string(),
                tls_fingerprint: None,
                plaintext_reason: None,
            },
            password: Some(test_secret()),
        };
        match profile_source(&input, &state).expect("source") {
            ProfileSource::Ui(profile) => assert_eq!(profile.host, "ui.example.com"),
            ProfileSource::Stored(_) => panic!("typed password must use the UI fields"),
        }
    }

    #[test]
    fn stored_profile_cannot_be_downgraded_to_plaintext() {
        let stored = stored_profile("stored-1", "127.0.0.1", "verify");
        let profile = build_profile_from_stored(&stored, test_secret()).expect("profile");
        assert!(
            matches!(profile.tls_mode, dmux_engine::db::TlsMode::VerifyFull),
            "a stored `verify` profile must stay encrypted, never downgrade"
        );
    }

    #[test]
    fn resolve_missing_profile_is_a_typed_error_not_a_panic() {
        let state = test_state();
        let input = ConnectInput {
            profile: ProfileInput {
                id: "no-such-profile".to_string(),
                label: String::new(),
                host: "h".to_string(),
                port: 5432,
                database: "d".to_string(),
                user: "u".to_string(),
                encryption: "verify".to_string(),
                tls_fingerprint: None,
                plaintext_reason: None,
            },
            password: None,
        };
        let err = resolve(&input, &state).unwrap_err();
        assert!(matches!(err, CommandError::ProfileNotFound { .. }));
    }

    #[test]
    fn tls_state_reflects_the_real_mode() {
        assert!(matches!(
            tls_state_from(&dmux_engine::db::TlsMode::VerifyFull),
            TlsStateDto::Encrypted { verified: true, .. }
        ));
        assert!(matches!(
            tls_state_from(&dmux_engine::db::TlsMode::TrustFingerprint("aabb".to_string())),
            TlsStateDto::Encrypted {
                verified: false,
                fingerprint: Some(fp)
            } if fp == "aabb"
        ));
        assert!(matches!(
            tls_state_from(&dmux_engine::db::TlsMode::Disable("test net".to_string())),
            TlsStateDto::Plaintext { reason } if reason == "test net"
        ));
    }

    // --- save-then-connect safety (the password must never follow a changed target) ---

    #[test]
    fn changed_host_without_a_new_password_never_resolves_with_the_old_password() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = crate::state::AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .expect("state");

        // Seed a stored profile with a stored password.
        let seed = ProfileInput {
            id: "prod".to_string(),
            label: "Production".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5432,
            database: "bank".to_string(),
            user: "dmux_dev".to_string(),
            encryption: "verify".to_string(),
            tls_fingerprint: None,
            plaintext_reason: None,
        };
        crate::commands::profiles::do_save_profile(seed.clone(), Some(test_secret()), &state)
            .expect("seed");

        // A compromised UI overwrites the host without supplying a password.
        let mut evil = seed;
        evil.host = "evil.example.com".to_string();
        let res =
            crate::commands::profiles::do_save_profile(evil.clone(), None, &state).expect("save");
        assert!(res
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("password removed")));

        // Connecting to the stored (now-evil) profile by keychain must fail with
        // a missing-password error, never send the old password to the attacker.
        let input = ConnectInput {
            profile: evil,
            password: None,
        };
        match resolve(&input, &state) {
            Err(CommandError::PasswordMissing { .. }) => {}
            Ok(_) => panic!("must not reuse the old password against the changed host"),
            Err(e) => panic!("expected a missing-password error, got {e:?}"),
        }
    }
}
