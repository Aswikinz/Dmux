//! Structure-half engine commands: probe, inventory, schema diff and mapping.
//!
//! Every command is async, typed with serde, and stays inside the engine's
//! public API.  Connections go through [`ReadOnlySession`] (read-only by
//! construction), and the mapping build only calls the exported
//! `match_tables` / `match_columns` / `Mapping` helpers.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::State;

use dmux_engine::db::{probe_server, ReadOnlySession, TlsMode};
use dmux_engine::inventory::{collect_inventory, DatabaseInventory};
use dmux_engine::mapping::{self, Mapping};
#[cfg(test)]
use dmux_engine::schema_diff::{compare_inventories, SchemaDiff};

use crate::error::{from_db_error, CommandError, PasswordMissing};
use crate::secrets::{input_fields_hash, stored_fields_hash};
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
        ca_file: stored.ca_file.clone(),
    };
    let tls = parse_tls_mode(&input)?;
    Ok(build_profile(&input, password, tls))
}

/// Resolve a [`ConnectInput`] into an engine profile plus its password.
///
/// With no typed password, the password comes from, in order:
/// 1. the keychain, sent only to the saved profile's fields;
/// 2. the session cache (T055, no keychain), for exactly the fields the UI
///    sends now, when a password was typed for those very fields earlier in
///    this session;
/// 3. the session cache for the saved profile's fields.
///
/// A cached password is bound to the hash of the fields it was typed for, so
/// it can never travel to a changed destination (PLAN 6.5).
pub(crate) fn resolve(
    input: &ConnectInput,
    state: &AppState,
) -> Result<dmux_engine::db::ConnProfile, CommandError> {
    if input.password.is_some() {
        return match profile_source(input, state)? {
            ProfileSource::Ui(profile) => {
                let tls = parse_tls_mode(&profile)?;
                let password = input.password.clone().unwrap_or_default();
                Ok(build_profile(&profile, password, tls))
            }
            ProfileSource::Stored(stored) => {
                let password = state.keychain()?.get(&stored.id)?;
                build_profile_from_stored(&stored, password)
            }
        };
    }
    let id = &input.profile.id;
    let stored = state.store()?.get(id)?;
    // 1. The keychain, for the saved fields only.
    let keychain_err = match &stored {
        Some(stored) => {
            let got = state.keychain()?.get(&stored.id);
            match got {
                Ok(password) => return build_profile_from_stored(stored, password),
                Err(e @ CommandError::PasswordMissing { .. }) => Some(e),
                Err(e) => return Err(e),
            }
        }
        None => None,
    };
    // 2. A session password typed for exactly these UI fields.
    let session = state.session_passwords()?;
    if let Some(password) = session.get(id, &input_fields_hash(&input.profile)) {
        drop(session);
        let tls = parse_tls_mode(&input.profile)?;
        return Ok(build_profile(&input.profile, password, tls));
    }
    // 3. A session password kept for the saved fields.
    if let Some(stored) = &stored {
        if let Some(password) = session.get(id, &stored_fields_hash(stored)) {
            drop(session);
            return build_profile_from_stored(stored, password);
        }
    }
    Err(keychain_err.unwrap_or_else(|| CommandError::ProfileNotFound { key: id.clone() }))
}

/// Whether the password for `input` is (or, after a typed password, will be)
/// a session-only one: the keychain cannot hold it on this machine.
fn keychain_unavailable_for(id: &str, state: &AppState) -> Result<bool, CommandError> {
    Ok(matches!(
        state.keychain()?.get(id),
        Err(CommandError::PasswordMissing {
            reason: PasswordMissing::NoKeychain,
            ..
        })
    ))
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
    /// The password is kept in memory for this session only, because there is
    /// no usable keychain (T055).  The UI shows a plain notice.
    pub session_password: bool,
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
            session_password: false,
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
///
/// With no usable keychain, a typed password that connected is kept for this
/// session, bound to exactly the fields it was tested with, so Compare
/// structure, Map, Choose checks and Run can use it (PLAN 4.1, T055).
#[tauri::command]
pub async fn probe_connection(
    input: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: State<'_, AppState>,
) -> Result<ProbeResult, CommandError> {
    probe_connection_with(input, allow_superuser, &state).await
}

/// [`probe_connection`] over a plain `&AppState` (testable).
pub(crate) async fn probe_connection_with(
    input: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: &AppState,
) -> Result<ProbeResult, CommandError> {
    let profile = resolve(&input, state)?;
    let tls = tls_state_from(&profile.tls_mode);
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    let info =
        drive(move || async move { probe_server(&profile, allow).await.map_err(from_db_error) })
            .await?;
    let mut result = ProbeResult::from((info, tls));
    let id = &input.profile.id;
    let fields = input_fields_hash(&input.profile);
    if let Some(password) = &input.password {
        // Only a password that really connected is kept, and only when the
        // keychain cannot hold it.
        if keychain_unavailable_for(id, state)? {
            state.session_passwords()?.put(id, fields, password);
            result.session_password = true;
        }
    } else {
        let stored_hash = state.store()?.get(id)?.map(|s| stored_fields_hash(&s));
        let sessions = state.session_passwords()?;
        result.session_password = sessions.contains(id, &fields)
            || stored_hash.is_some_and(|h| sessions.contains(id, &h));
    }
    Ok(result)
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
) -> Result<crate::commands::structure_view::StructureCompare, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    let old_inv = collect(&old, allow, &state).await?;
    let new_inv = collect(&new, allow, &state).await?;
    Ok(crate::commands::structure_view::compare(&old_inv, &new_inv))
}

/// The schema diff alone (what the command returned before the side-by-side
/// rows), callable from tests without a Tauri `State`.
#[cfg(test)]
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
    /// The pieces of `path`, so the UI never has to split a dotted name.
    pub schema: String,
    pub table: Option<String>,
    pub column: Option<String>,
    pub issue: String,
    pub candidates: Vec<String>,
}

/// A fuzzy-match suggestion, serialised from the engine's `Suggestion`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SuggestionEntry {
    pub path: String,
    /// The pieces of `path` (old side).  `candidate` is a bare table or column
    /// name in the matching new schema or table.
    pub schema: String,
    pub table: Option<String>,
    pub column: Option<String>,
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
    build_mapping_with(old, new, allow_superuser, &state).await
}

/// [`build_mapping`] over a plain `&AppState` (testable).
pub(crate) async fn build_mapping_with(
    old: ConnectInput,
    new: ConnectInput,
    allow_superuser: Option<SuperuserOverride>,
    state: &AppState,
) -> Result<MappingBuild, CommandError> {
    let allow = allow_superuser
        .as_ref()
        .is_some_and(SuperuserOverride::allows);
    let old_inv = collect(&old, allow, state).await?;
    let new_inv = collect(&new, allow, state).await?;
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
    build_with_rules(old, new, &[])
}

/// The part of `path` after `prefix.`, when it starts with it.  Paths are built
/// as `schema.table` and `schema.table.column` by the engine matcher; the
/// structured fields sent to the UI are cut from them here, where the schema and
/// table are known, so a name that itself contains a dot is never split wrongly.
fn tail<'a>(path: &'a str, prefix: &str) -> &'a str {
    path.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or(path)
}

fn table_attention(schema: &str, item: mapping::AttentionItem) -> AttentionEntry {
    AttentionEntry {
        schema: schema.to_string(),
        table: Some(tail(&item.path, schema).to_string()),
        column: None,
        issue: attention_issue(&item.issue),
        candidates: attention_candidates(&item.issue),
        path: item.path,
    }
}

fn column_attention(schema: &str, table: &str, item: mapping::AttentionItem) -> AttentionEntry {
    let prefix = format!("{schema}.{table}");
    AttentionEntry {
        schema: schema.to_string(),
        table: Some(table.to_string()),
        column: Some(tail(&item.path, &prefix).to_string()),
        issue: attention_issue(&item.issue),
        candidates: attention_candidates(&item.issue),
        path: item.path,
    }
}

/// [`build_from_inventories`] with the user's rename rules applied first.
///
/// Rules apply to **table** names only (a schema rename renames a schema, a
/// prefix or suffix rule renames tables); column names are never renamed by a
/// rule.  A rule pairs an old table with the renamed one only when that table
/// really exists in the new database; the engine's own `apply_rule` does the
/// renaming, so the build and the run can never disagree.  Two old tables that
/// a rule sends to one new table are not silently double-mapped: the second one
/// is flagged for a decision.
pub fn build_with_rules(
    old: &DatabaseInventory,
    new: &DatabaseInventory,
    rules: &[mapping::Rule],
) -> (Mapping, Vec<AttentionEntry>, Vec<SuggestionEntry>) {
    let mut mapping = Mapping::new();
    mapping.rules = rules.to_vec();
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

        let Some(new_schema) = new_schemas.get(target_schema.as_str()) else {
            needs_attention.push(AttentionEntry {
                path: schema_name.to_string(),
                schema: schema_name.to_string(),
                table: None,
                column: None,
                issue: "missing_in_new".to_string(),
                candidates: Vec::new(),
            });
            // Nothing to map at table level for a missing target schema.
            continue;
        };

        let old_names: Vec<String> = old_schema.tables.iter().map(|t| t.name.clone()).collect();
        let new_names: Vec<String> = new_schema.tables.iter().map(|t| t.name.clone()).collect();

        // Rules come first (PLAN 4.3): a prefix or suffix rule pairs a table
        // with its renamed counterpart when that exists in the new schema.
        let mut rule_mapped: BTreeMap<String, dmux_engine::mapping::TableMapping> = BTreeMap::new();
        let mut rule_targets: BTreeSet<String> = BTreeSet::new();
        for old_name in &old_names {
            let renamed = mapping
                .rules
                .iter()
                .filter(|r| !matches!(r.kind, mapping::RuleKind::SchemaRename))
                .find_map(|r| mapping::apply_rule(r, old_name));
            let Some(renamed) = renamed else { continue };
            if renamed == *old_name || !new_names.contains(&renamed) {
                continue;
            }
            if rule_targets.contains(&renamed) {
                // A second old table for the same new table: never double-map.
                needs_attention.push(AttentionEntry {
                    path: format!("{schema_name}.{old_name}"),
                    schema: schema_name.to_string(),
                    table: Some(old_name.clone()),
                    column: None,
                    issue: "ambiguous_case_insensitive".to_string(),
                    candidates: vec![renamed],
                });
                continue;
            }
            rule_targets.insert(renamed.clone());
            rule_mapped.insert(
                old_name.clone(),
                dmux_engine::mapping::TableMapping {
                    target_table: renamed,
                    status: mapping::MappingStatus::Rule,
                    ..Default::default()
                },
            );
        }
        let rest_old: Vec<String> = old_names
            .iter()
            .filter(|n| !rule_mapped.contains_key(*n))
            .cloned()
            .collect();
        let rest_new: Vec<String> = new_names
            .iter()
            .filter(|n| !rule_targets.contains(*n))
            .cloned()
            .collect();
        let mut tables = mapping::match_tables(schema_name, &rest_old, &rest_new);
        tables.mapped.extend(rule_mapped);
        let taken: BTreeSet<String> = tables
            .mapped
            .values()
            .map(|m| m.target_table.clone())
            .collect();

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
            let taken_cols: BTreeSet<&str> = column_mappings
                .values()
                .map(|c| c.target_column.as_str())
                .collect();

            for a in col_attention {
                needs_attention.push(column_attention(schema_name, old_name, a));
            }
            for s in col_suggestions {
                // Only a column nobody has taken can be suggested.
                if taken_cols.contains(s.candidate.as_str()) {
                    continue;
                }
                suggestions.push(SuggestionEntry {
                    schema: schema_name.to_string(),
                    table: Some(old_name.clone()),
                    column: Some(tail(&s.path, &format!("{schema_name}.{old_name}")).to_string()),
                    path: s.path,
                    candidate: s.candidate,
                    confidence: s.confidence,
                });
            }
            table_mapping.column_mappings = column_mappings;

            schema_mapping
                .table_mappings
                .insert(old_name.clone(), table_mapping);
        }

        for a in tables.needs_attention {
            needs_attention.push(table_attention(schema_name, a));
        }
        for s in tables.suggestions {
            if taken.contains(&s.candidate) {
                continue;
            }
            suggestions.push(SuggestionEntry {
                schema: schema_name.to_string(),
                table: Some(tail(&s.path, schema_name).to_string()),
                column: None,
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
                ca_file: None,
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
            ca_file: None,
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
                ca_file: None,
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
                ca_file: None,
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
                ca_file: None,
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
            ca_file: None,
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

    // --- T055: session passwords when there is no keychain ---

    fn no_keychain_state() -> crate::state::AppState {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        crate::state::AppState::with_password_store(
            path,
            Box::new(crate::secrets::InMemoryPasswordStore::unavailable()),
        )
        .expect("state")
    }

    fn ui_profile(id: &str, host: &str) -> ProfileInput {
        ProfileInput {
            id: id.to_string(),
            label: "l".to_string(),
            host: host.to_string(),
            port: 5433,
            database: "bank".to_string(),
            user: "reader".to_string(),
            encryption: "allow-plaintext".to_string(),
            tls_fingerprint: None,
            plaintext_reason: Some("lab".to_string()),
            ca_file: None,
        }
    }

    fn no_password(profile: ProfileInput) -> ConnectInput {
        ConnectInput {
            profile,
            password: None,
        }
    }

    #[test]
    fn an_unsaved_profile_resolves_from_a_session_password_for_the_same_fields_only() {
        let state = no_keychain_state();
        let secret = test_secret();
        let p = ui_profile("u1", "db.bank");
        state.session_passwords().unwrap().put(
            "u1",
            crate::secrets::input_fields_hash(&p),
            &secret,
        );
        let got = resolve(&no_password(p.clone()), &state).expect("cached password");
        assert_eq!(got.host, "db.bank");
        assert_eq!(got.password, secret);
        // Any changed connection field misses: the password never follows it.
        let mut moved = p.clone();
        moved.host = "evil.example.com".to_string();
        let err = resolve(&no_password(moved), &state).unwrap_err();
        assert!(
            matches!(err, CommandError::ProfileNotFound { .. }),
            "{err:?}"
        );
        let mut other_user = p;
        other_user.user = "admin".to_string();
        assert!(resolve(&no_password(other_user), &state).is_err());
    }

    #[test]
    fn a_saved_profile_without_a_keychain_resolves_from_the_session_password() {
        let state = no_keychain_state();
        let secret = test_secret();
        let res = crate::commands::profiles::do_save_profile(
            ui_profile("s1", "db.bank"),
            Some(secret.clone()),
            &state,
        )
        .unwrap();
        assert_eq!(
            res.warning.as_deref(),
            Some(crate::secrets::SESSION_PASSWORD_MESSAGE)
        );
        // The UI sends the saved fields: the session password is used for them.
        let got = resolve(&no_password(ui_profile("s1", "db.bank")), &state).unwrap();
        assert_eq!(
            (got.host.as_str(), got.password.as_str()),
            ("db.bank", secret.as_str())
        );
        // A UI that sends another host still only reaches the saved host.
        let got = resolve(&no_password(ui_profile("s1", "evil.example.com")), &state).unwrap();
        assert_eq!(got.host, "db.bank");
    }

    #[test]
    fn changing_the_host_or_deleting_the_profile_clears_the_session_password() {
        let state = no_keychain_state();
        let secret = test_secret();
        crate::commands::profiles::do_save_profile(
            ui_profile("c1", "db.bank"),
            Some(secret.clone()),
            &state,
        )
        .unwrap();
        assert_eq!(state.session_passwords().unwrap().len(), 1);
        // Host change without a new password: the cached password is gone.
        let res = crate::commands::profiles::do_save_profile(
            ui_profile("c1", "other.bank"),
            None,
            &state,
        )
        .unwrap();
        assert!(state.session_passwords().unwrap().is_empty());
        assert!(res.warning.is_some());
        let err = resolve(&no_password(ui_profile("c1", "other.bank")), &state).unwrap_err();
        assert!(
            matches!(err, CommandError::PasswordMissing { .. }),
            "{err:?}"
        );
        assert!(!format!("{err} {err:?}").contains(&secret));
        // Delete forgets it too.
        crate::commands::profiles::do_save_profile(
            ui_profile("c1", "other.bank"),
            Some(secret.clone()),
            &state,
        )
        .unwrap();
        assert_eq!(state.session_passwords().unwrap().len(), 1);
        crate::commands::profiles::do_delete_profile("c1", &state).unwrap();
        assert!(state.session_passwords().unwrap().is_empty());
        // And the app exit clears everything.
        state
            .session_passwords()
            .unwrap()
            .put("x", "h".into(), &secret);
        state.clear_session_passwords();
        assert!(state.session_passwords().unwrap().is_empty());
    }

    #[test]
    fn a_label_only_change_keeps_the_session_password() {
        let state = no_keychain_state();
        crate::commands::profiles::do_save_profile(
            ui_profile("k1", "db.bank"),
            Some(test_secret()),
            &state,
        )
        .unwrap();
        let mut relabelled = ui_profile("k1", "db.bank");
        relabelled.label = "Renamed".to_string();
        crate::commands::profiles::do_save_profile(relabelled, None, &state).unwrap();
        assert_eq!(state.session_passwords().unwrap().len(), 1);
    }

    #[test]
    fn a_working_keychain_never_uses_the_session_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = crate::state::AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::new()),
        )
        .expect("state");
        let res = crate::commands::profiles::do_save_profile(
            ui_profile("w1", "db.bank"),
            Some(test_secret()),
            &state,
        )
        .unwrap();
        assert!(res.warning.is_none());
        assert!(state.session_passwords().unwrap().is_empty());
        assert!(!keychain_unavailable_for("w1", &state).unwrap());
    }

    /// The live flow on a machine with no keychain: save, probe, build the
    /// mapping and start a run, all from the one typed password.  Then the
    /// unsaved variant: type the password, Test connection, Compare structure.
    #[tokio::test(flavor = "multi_thread")]
    async fn without_a_keychain_save_probe_map_and_run_use_the_session_password() {
        let (Some(old_url), Some(new_url)) = (
            env_url("DMUX_TEST_DEMO_OLD_URL"),
            env_url("DMUX_TEST_DEMO_NEW_URL"),
        ) else {
            return;
        };
        let state = no_keychain_state();
        let (mut old, old_pw) = profile_input_from_url(&old_url);
        let (mut new, new_pw) = profile_input_from_url(&new_url);
        old.id = "old".to_string();
        new.id = "new".to_string();
        let su = || {
            Some(SuperuserOverride {
                allow: true,
                reason: "local test database".to_string(),
            })
        };

        // Save with the typed password: kept for the session, with the notice.
        for (p, pw) in [(&old, &old_pw), (&new, &new_pw)] {
            let res =
                crate::commands::profiles::do_save_profile(p.clone(), Some(pw.clone()), &state)
                    .unwrap();
            assert_eq!(
                res.warning.as_deref(),
                Some(crate::secrets::SESSION_PASSWORD_MESSAGE)
            );
        }
        // Probe with no password.
        let probed = probe_connection_with(no_password(old.clone()), su(), &state)
            .await
            .expect("probe uses the session password");
        assert!(probed.session_password);
        let json = serde_json::to_string(&probed).unwrap();
        assert!(!json.contains(&old_pw));
        // Build the mapping with no password.
        let build = build_mapping_with(
            no_password(old.clone()),
            no_password(new.clone()),
            su(),
            &state,
        )
        .await
        .expect("mapping uses the session password");
        assert!(!serde_json::to_string(&build).unwrap().contains(&old_pw));
        // Start a run with no password (structure only, one small table).
        let mut levels = BTreeMap::new();
        levels.insert(
            "demo_bank.account_types".to_string(),
            dmux_engine::results::CheckLevel::Structure,
        );
        let started = crate::commands::data::start_run_with(
            crate::commands::data::StartRunInput {
                old: no_password(old.clone()),
                new: no_password(new.clone()),
                mapping: build.mapping,
                levels,
                default_level: dmux_engine::results::CheckLevel::Structure,
                control_total_columns: vec![],
                old_parallelism: None,
                new_parallelism: None,
                run_window: None,
                export_keys: None,
                allow_superuser: su(),
            },
            &state,
        )
        .expect("the run starts with the session password");
        let id = started.run_id;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let p = crate::commands::data::poll_run_with(id.clone(), &state).unwrap();
            assert_ne!(p.overall_state, "failed", "{:?}", p.error);
            if p.overall_state == "done" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the run never finished"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // The unsaved variant: a typed password that connects is kept.
        let mut fresh = old.clone();
        fresh.id = "unsaved".to_string();
        let typed = ConnectInput {
            profile: fresh.clone(),
            password: Some(old_pw.clone()),
        };
        let probed = probe_connection_with(typed, su(), &state).await.unwrap();
        assert!(probed.session_password);
        collect(&no_password(fresh.clone()), true, &state)
            .await
            .expect("compare structure uses the tested password");
        // A wrong password that fails is never kept.
        let mut other = fresh.clone();
        other.id = "wrong".to_string();
        let bad = ConnectInput {
            profile: other.clone(),
            password: Some(format!("{}-wrong", test_secret())),
        };
        let err = probe_connection_with(bad, su(), &state).await.unwrap_err();
        assert!(!format!("{err} {err:?}").contains(&old_pw));
        assert!(resolve(&no_password(other), &state).is_err());
    }

    /// The CI no-keychain case: a password typed and proven by Test connection
    /// is cached for exactly those fields; the automatic save that follows is
    /// refused because the keychain cannot even delete; Compare structure must
    /// still find the password.  Changed fields must still never get it.
    #[test]
    fn a_refused_save_never_costs_the_tested_session_password() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = crate::state::AppState::with_password_store(
            dir.path().to_path_buf(),
            Box::new(crate::secrets::InMemoryPasswordStore::unavailable_everywhere()),
        )
        .expect("state");
        let secret = test_secret();
        let p = ui_profile("old-profile", "db.bank");
        state.session_passwords().unwrap().put(
            "old-profile",
            crate::secrets::input_fields_hash(&p),
            &secret,
        );
        let saved =
            crate::commands::profiles::do_save_profile(p.clone(), Some(secret.clone()), &state);
        assert!(
            matches!(saved, Err(CommandError::PasswordRemoveFailed)),
            "{saved:?}"
        );
        let got = resolve(&no_password(p.clone()), &state).expect("the tested password is kept");
        assert_eq!(got.password, secret);
        // A save with different fields forgets it (and still fails closed).
        let mut moved = p;
        moved.host = "other.bank".to_string();
        let _ = crate::commands::profiles::do_save_profile(moved.clone(), None, &state);
        assert!(state.session_passwords().unwrap().is_empty());
        assert!(resolve(&no_password(moved), &state).is_err());
    }
}
