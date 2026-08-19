#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Fixed-location, content-free adapters for supported local tools.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead as _, BufReader},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use observer_core::{
    Adapter, AdapterDefinition, AdapterMeasurement, AdapterResult, ObservationEvent, SessionSpan,
};
use observer_domain::{
    ADAPTER_REGISTRY_VERSION, ConsentManifest, Coverage, CoverageStatus, FieldClass,
};
use serde_json::Value;

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 4_096;
const MAX_JSONL_LINE_BYTES: usize = 256 * 1024;
const MAX_JSONL_DEPTH: usize = 4;
const MAX_SHELL_HISTORY_BYTES: u64 = 2 * 1024 * 1024;

#[must_use]
pub fn source_definition_version(adapter_id: &str) -> &'static str {
    match adapter_id {
        "claude-code" => "claude-code-project-jsonl/v2",
        "codex" => "codex-rollout-jsonl/v2",
        "cursor" => "cursor-detected-unmeasured/v2",
        "shell-history" => "shell-history-opt-in/v1",
        "git-worktrees" => "git-worktree-metadata/v1",
        "second" | "zed" | "vscode-copilot" | "warp" => "detection-only/v1",
        _ => "unknown-adapter/v1",
    }
}

/// All locations derive only from the participant-supplied `--home` value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterLocations {
    pub claude_code_root: PathBuf,
    pub codex_root: PathBuf,
    pub cursor_roots: Vec<PathBuf>,
    pub shell_history_paths: Vec<PathBuf>,
    pub git_metadata_roots: Vec<PathBuf>,
    pub detection_paths: BTreeMap<String, Vec<PathBuf>>,
}

impl AdapterLocations {
    #[must_use]
    pub fn from_home(home: impl AsRef<Path>) -> Self {
        let home = home.as_ref();
        let mut detection_paths = BTreeMap::new();
        detection_paths.insert(
            "zed".to_owned(),
            vec![
                home.join("Library/Application Support/Zed"),
                home.join(".local/share/zed"),
                home.join("AppData/Roaming/Zed"),
            ],
        );
        detection_paths.insert(
            "vscode-copilot".to_owned(),
            vec![
                home.join("Library/Application Support/Code"),
                home.join(".config/Code"),
                home.join("AppData/Roaming/Code"),
            ],
        );
        detection_paths.insert(
            "warp".to_owned(),
            vec![
                home.join("Library/Application Support/warp"),
                home.join(".local/share/warp"),
                home.join("AppData/Roaming/warp"),
            ],
        );
        detection_paths.insert(
            "second".to_owned(),
            vec![
                home.join("Library/Application Support/Second"),
                home.join(".local/share/second"),
                home.join("AppData/Roaming/Second"),
            ],
        );
        Self {
            claude_code_root: home.join(".claude/projects"),
            codex_root: home.join(".codex/sessions"),
            cursor_roots: vec![
                home.join("Library/Application Support/Cursor/User/globalStorage"),
                home.join(".config/Cursor/User/globalStorage"),
                home.join("AppData/Roaming/Cursor/User/globalStorage"),
            ],
            shell_history_paths: vec![home.join(".zsh_history"), home.join(".bash_history")],
            git_metadata_roots: Vec::new(),
            detection_paths,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectionPermission {
    pub adapter_approved: bool,
    pub content_analysis: bool,
    pub approved_field_classes: BTreeSet<FieldClass>,
}

impl CollectionPermission {
    #[must_use]
    pub fn metadata_only(adapter_approved: bool) -> Self {
        Self {
            adapter_approved,
            content_analysis: false,
            approved_field_classes: [
                FieldClass::FilesystemMetadata,
                FieldClass::Timestamps,
                FieldClass::EventTypes,
                FieldClass::Counters,
            ]
            .into_iter()
            .collect(),
        }
    }
}

impl CollectionPermission {
    #[must_use]
    pub fn from_consent(consent: &ConsentManifest, adapter_approved: bool) -> Self {
        Self {
            adapter_approved,
            content_analysis: consent.content_analysis,
            approved_field_classes: consent.approved_field_classes.iter().cloned().collect(),
        }
    }

    fn allows_metadata(&self) -> bool {
        [
            FieldClass::FilesystemMetadata,
            FieldClass::Timestamps,
            FieldClass::EventTypes,
            FieldClass::Counters,
        ]
        .iter()
        .all(|field| self.approved_field_classes.contains(field))
    }

    fn allows_message_text(&self) -> bool {
        self.content_analysis && self.approved_field_classes.contains(&FieldClass::MessageText)
    }
    fn allows_command_text(&self) -> bool {
        self.content_analysis && self.approved_field_classes.contains(&FieldClass::CommandText)
    }
}

pub trait LocalAdapter: Adapter {
    fn observe(&self, permission: CollectionPermission) -> AdapterMeasurement;
}

macro_rules! jsonl_adapter {
    ($name:ident, $parser:ident) => {
        #[derive(Clone, Debug)]
        pub struct $name {
            definition: AdapterDefinition,
            root: PathBuf,
        }
        impl $name {
            #[must_use]
            pub fn new(definition: AdapterDefinition, root: PathBuf) -> Self {
                Self { definition, root }
            }
        }
        impl Adapter for $name {
            fn definition(&self) -> &AdapterDefinition {
                &self.definition
            }
            fn collect(&self, consented: bool) -> AdapterResult {
                AdapterResult {
                    coverage: self.observe(CollectionPermission::metadata_only(consented)).coverage,
                }
            }
        }
        impl LocalAdapter for $name {
            fn observe(&self, permission: CollectionPermission) -> AdapterMeasurement {
                if !permission.adapter_approved {
                    return disabled(&self.definition);
                }
                if !permission.allows_metadata() {
                    return field_disabled(&self.definition, "metadata_field_classes_not_approved");
                }
                $parser(&self.definition, &self.root, permission.allows_message_text())
            }
        }
    };
}

jsonl_adapter!(ClaudeCodeAdapter, observe_claude);
jsonl_adapter!(CodexAdapter, observe_codex);

#[derive(Clone, Debug)]
pub struct CursorAdapter {
    definition: AdapterDefinition,
    roots: Vec<PathBuf>,
}
impl CursorAdapter {
    #[must_use]
    pub fn new(definition: AdapterDefinition, roots: Vec<PathBuf>) -> Self {
        Self { definition, roots }
    }
}
impl Adapter for CursorAdapter {
    fn definition(&self) -> &AdapterDefinition {
        &self.definition
    }
    fn collect(&self, consented: bool) -> AdapterResult {
        AdapterResult {
            coverage: self.observe(CollectionPermission::metadata_only(consented)).coverage,
        }
    }
}
impl LocalAdapter for CursorAdapter {
    fn observe(&self, permission: CollectionPermission) -> AdapterMeasurement {
        if !permission.adapter_approved {
            return disabled(&self.definition);
        }
        if !permission.approved_field_classes.contains(&FieldClass::FilesystemMetadata) {
            return field_disabled(&self.definition, "filesystem_metadata_not_approved");
        }
        let detected = self.roots.iter().any(|path| path_exists_without_following(path));
        measurement(
            coverage(
                &self.definition,
                if detected { CoverageStatus::DetectedUnmeasured } else { CoverageStatus::Missing },
                0,
                vec![if detected {
                    "cursor_schema_not_verified"
                } else {
                    "fixed_location_not_found"
                }],
            ),
            Vec::new(),
            Vec::new(),
            0,
            BTreeSet::new(),
        )
    }
}

#[derive(Clone, Debug)]
pub struct ShellHistoryAdapter {
    definition: AdapterDefinition,
    paths: Vec<PathBuf>,
}
impl ShellHistoryAdapter {
    #[must_use]
    pub fn new(definition: AdapterDefinition, paths: Vec<PathBuf>) -> Self {
        Self { definition, paths }
    }
}
impl Adapter for ShellHistoryAdapter {
    fn definition(&self) -> &AdapterDefinition {
        &self.definition
    }
    fn collect(&self, consented: bool) -> AdapterResult {
        AdapterResult {
            coverage: self.observe(CollectionPermission::metadata_only(consented)).coverage,
        }
    }
}
impl LocalAdapter for ShellHistoryAdapter {
    fn observe(&self, permission: CollectionPermission) -> AdapterMeasurement {
        if !permission.adapter_approved {
            return disabled(&self.definition);
        }
        observe_shell_history(&self.definition, &self.paths, permission.allows_command_text())
    }
}

#[derive(Clone, Debug)]
pub struct GitWorktreeAdapter {
    definition: AdapterDefinition,
    roots: Vec<PathBuf>,
}
impl GitWorktreeAdapter {
    #[must_use]
    pub fn new(definition: AdapterDefinition, roots: Vec<PathBuf>) -> Self {
        Self { definition, roots }
    }
}
impl Adapter for GitWorktreeAdapter {
    fn definition(&self) -> &AdapterDefinition {
        &self.definition
    }
    fn collect(&self, consented: bool) -> AdapterResult {
        AdapterResult {
            coverage: self.observe(CollectionPermission::metadata_only(consented)).coverage,
        }
    }
}
impl LocalAdapter for GitWorktreeAdapter {
    fn observe(&self, permission: CollectionPermission) -> AdapterMeasurement {
        if !permission.adapter_approved {
            return disabled(&self.definition);
        }
        if !permission.approved_field_classes.contains(&FieldClass::FilesystemMetadata) {
            return field_disabled(&self.definition, "filesystem_metadata_not_approved");
        }
        observe_worktrees(&self.definition, &self.roots)
    }
}

#[derive(Clone, Debug)]
pub struct DetectionOnlyAdapter {
    definition: AdapterDefinition,
    paths: Vec<PathBuf>,
}
impl DetectionOnlyAdapter {
    #[must_use]
    pub fn new(definition: AdapterDefinition, paths: Vec<PathBuf>) -> Self {
        Self { definition, paths }
    }
}
impl Adapter for DetectionOnlyAdapter {
    fn definition(&self) -> &AdapterDefinition {
        &self.definition
    }
    fn collect(&self, consented: bool) -> AdapterResult {
        AdapterResult {
            coverage: self.observe(CollectionPermission::metadata_only(consented)).coverage,
        }
    }
}
impl LocalAdapter for DetectionOnlyAdapter {
    fn observe(&self, permission: CollectionPermission) -> AdapterMeasurement {
        if !permission.adapter_approved {
            return disabled(&self.definition);
        }
        if !permission.approved_field_classes.contains(&FieldClass::FilesystemMetadata) {
            return field_disabled(&self.definition, "filesystem_metadata_not_approved");
        }
        let detected = self.paths.iter().any(|path| path_exists_without_following(path));
        measurement(
            coverage(
                &self.definition,
                if detected { CoverageStatus::DetectedUnmeasured } else { CoverageStatus::Missing },
                0,
                vec![if detected { "detection_only_adapter" } else { "fixed_location_not_found" }],
            ),
            Vec::new(),
            Vec::new(),
            0,
            BTreeSet::new(),
        )
    }
}

/// Creates adapters without process execution or ambient-home lookup.
pub fn adapters_for_locations(
    definitions: &[AdapterDefinition],
    locations: &AdapterLocations,
) -> Result<Vec<Box<dyn LocalAdapter>>, String> {
    definitions
        .iter()
        .map(|definition| match definition.id.as_str() {
            "claude-code" => Ok(Box::new(ClaudeCodeAdapter::new(
                definition.clone(),
                locations.claude_code_root.clone(),
            )) as Box<dyn LocalAdapter>),
            "codex" => {
                Ok(Box::new(CodexAdapter::new(definition.clone(), locations.codex_root.clone()))
                    as Box<dyn LocalAdapter>)
            }
            "cursor" => {
                Ok(Box::new(CursorAdapter::new(definition.clone(), locations.cursor_roots.clone()))
                    as Box<dyn LocalAdapter>)
            }
            "shell-history" => Ok(Box::new(ShellHistoryAdapter::new(
                definition.clone(),
                locations.shell_history_paths.clone(),
            )) as Box<dyn LocalAdapter>),
            "git-worktrees" => Ok(Box::new(GitWorktreeAdapter::new(
                definition.clone(),
                locations.git_metadata_roots.clone(),
            )) as Box<dyn LocalAdapter>),
            "second" | "zed" | "vscode-copilot" | "warp" => Ok(Box::new(DetectionOnlyAdapter::new(
                definition.clone(),
                locations.detection_paths.get(&definition.id).cloned().unwrap_or_default(),
            ))
                as Box<dyn LocalAdapter>),
            _ => Err("registry contains an unknown adapter identifier".to_owned()),
        })
        .collect()
}

fn observe_claude(
    definition: &AdapterDefinition,
    root: &Path,
    content_analysis: bool,
) -> AdapterMeasurement {
    observe_jsonl_source(definition, root, content_analysis, parse_claude_row)
}

fn observe_codex(
    definition: &AdapterDefinition,
    root: &Path,
    content_analysis: bool,
) -> AdapterMeasurement {
    observe_jsonl_source(definition, root, content_analysis, parse_codex_row)
}

type RowParser = fn(&Value, bool, &mut FileTally) -> bool;

fn observe_jsonl_source(
    definition: &AdapterDefinition,
    root: &Path,
    content_analysis: bool,
    parser: RowParser,
) -> AdapterMeasurement {
    let listed = bounded_files(root, MAX_JSONL_DEPTH, MAX_FILES, |name| {
        Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
    });
    let mut total = FileTally::default();
    let mut permission_denied = false;
    let mut malformed = false;
    let mut unsafe_input = false;
    for path in listed.files {
        match read_jsonl(&path, content_analysis, parser) {
            Ok(tally) => total.merge(tally),
            Err(ReadClass::PermissionDenied) => permission_denied = true,
            Err(ReadClass::Malformed) => malformed = true,
            Err(ReadClass::Unsafe) => unsafe_input = true,
        }
    }
    let observed = total.recognized_records > 0;
    let status = if observed {
        CoverageStatus::Observed
    } else if permission_denied {
        CoverageStatus::PermissionDenied
    } else if listed.any_candidate || malformed || unsafe_input {
        CoverageStatus::UnsupportedVersion
    } else {
        CoverageStatus::Missing
    };
    let mut reasons = Vec::new();
    if listed.truncated {
        reasons.push("bounded_file_limit_reached");
    }
    if malformed {
        reasons.push("malformed_jsonl_record");
    }
    if unsafe_input {
        reasons.push("unsafe_or_oversized_jsonl_input");
    }
    if permission_denied {
        reasons.push("source_permission_denied");
    }
    if reasons.is_empty() && status == CoverageStatus::Missing {
        reasons.push("fixed_location_not_found");
    }
    let mut supported = standard_metrics();
    if !total.saw_tokens {
        supported.retain(|metric| !metric.ends_with("tokens"));
    }
    if content_analysis {
        supported.extend(
            ["manual_relay_ratio", "routing_message_ratio", "correction_signals"]
                .into_iter()
                .map(str::to_owned),
        );
    } else {
        supported.remove("correction_signals");
    }
    measurement(
        coverage(definition, status, total.recognized_records, reasons),
        total.events,
        total.spans,
        total.untimestamped_records,
        supported,
    )
}

#[derive(Default)]
struct FileTally {
    recognized_records: u64,
    untimestamped_records: u64,
    saw_tokens: bool,
    events: Vec<ObservationEvent>,
    spans: Vec<SessionSpan>,
    first_timestamp: Option<DateTime<Utc>>,
    last_timestamp: Option<DateTime<Utc>>,
    token_total_marker: u64,
}

impl FileTally {
    fn event(&mut self, timestamp: Option<DateTime<Utc>>, counters: BTreeMap<String, u64>) {
        if counters.is_empty() {
            return;
        }
        if let Some(timestamp) = timestamp {
            self.first_timestamp =
                Some(self.first_timestamp.map_or(timestamp, |old| old.min(timestamp)));
            self.last_timestamp =
                Some(self.last_timestamp.map_or(timestamp, |old| old.max(timestamp)));
        } else {
            self.untimestamped_records += 1;
        }
        self.events.push(ObservationEvent { timestamp, counters });
    }
    fn merge(&mut self, mut other: Self) {
        self.recognized_records += other.recognized_records;
        self.untimestamped_records += other.untimestamped_records;
        self.saw_tokens |= other.saw_tokens;
        self.events.append(&mut other.events);
        self.spans.append(&mut other.spans);
    }
    fn finish_span(&mut self) {
        if let (Some(start), Some(end)) = (self.first_timestamp, self.last_timestamp) {
            self.spans.push(SessionSpan { start, end });
        }
    }
}

enum ReadClass {
    PermissionDenied,
    Malformed,
    Unsafe,
}

fn read_jsonl(
    path: &Path,
    content_analysis: bool,
    parser: RowParser,
) -> Result<FileTally, ReadClass> {
    let metadata = safe_regular_file(path).ok_or(ReadClass::Unsafe)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(ReadClass::Unsafe);
    }
    let file = fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            ReadClass::PermissionDenied
        } else {
            ReadClass::Malformed
        }
    })?;
    let mut tally = FileTally::default();
    for line in BufReader::new(file).split(b'\n') {
        let line = line.map_err(|_| ReadClass::Malformed)?;
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_JSONL_LINE_BYTES {
            return Err(ReadClass::Unsafe);
        }
        let row: Value = serde_json::from_slice(&line).map_err(|_| ReadClass::Malformed)?;
        if !parser(&row, content_analysis, &mut tally) {
            return Err(ReadClass::Malformed);
        }
        tally.recognized_records += 1;
    }
    tally.finish_span();
    Ok(tally)
}

fn parse_claude_row(row: &Value, content_analysis: bool, tally: &mut FileTally) -> bool {
    let kind = row.get("type").and_then(Value::as_str);
    let timestamp = parse_timestamp(row.get("timestamp"));
    let mut counters = BTreeMap::new();
    match kind {
        Some("user") => {
            let content = row.pointer("/message/content").or_else(|| row.get("content"));
            let (text_bearing, tool_errors) = user_content_counts(content);
            if !row.get("isMeta").and_then(Value::as_bool).unwrap_or(false) && text_bearing > 0 {
                counters.insert("human_turns".to_owned(), 1);
            }
            add(&mut counters, "tool_error_signals", tool_errors);
        }
        Some("assistant") => {
            counters.insert("agent_turns".to_owned(), 1);
            let content = row.pointer("/message/content").or_else(|| row.get("content"));
            let (tools, errors) = content_block_counts(content);
            add(&mut counters, "tool_operations", tools);
            add(&mut counters, "tool_error_signals", errors);
            let usage = row.pointer("/message/usage").or_else(|| row.get("usage"));
            add_claude_usage(&mut counters, usage, tally);
        }
        Some("system" | "summary" | "file-history-snapshot") => {}
        _ => return false,
    }
    if content_analysis && counters.contains_key("human_turns") {
        add_content_patterns(&mut counters, row);
    }
    tally.event(timestamp, counters);
    true
}

fn parse_codex_row(row: &Value, content_analysis: bool, tally: &mut FileTally) -> bool {
    let row_kind = row.get("type").and_then(Value::as_str);
    let timestamp = parse_timestamp(row.get("timestamp"));
    let payload = row.get("payload").unwrap_or(&Value::Null);
    let kind = if row_kind == Some("event_msg") {
        payload.get("type").and_then(Value::as_str)
    } else {
        row_kind
    };
    let mut counters = BTreeMap::new();
    match kind {
        Some("turn_aborted") => {
            counters.insert("interrupt_signals".to_owned(), 1);
        }
        Some("token_count") => add_codex_usage(&mut counters, payload.pointer("/info"), tally),
        Some("turn_context" | "session_meta") => {}
        Some("agent_message") => {
            counters.insert("agent_turns".to_owned(), 1);
        }
        Some("response_item") => match payload.get("type").and_then(Value::as_str) {
            Some("message")
                if payload.get("role").and_then(Value::as_str) == Some("user")
                    && codex_user_text(payload) =>
            {
                counters.insert("human_turns".to_owned(), 1);
            }
            Some("agent_message") => {
                counters.insert("agent_turns".to_owned(), 1);
            }
            Some(value) if value.ends_with("call") && !value.ends_with("call_output") => {
                counters.insert("tool_operations".to_owned(), 1);
                if matches!(
                    payload.get("status").and_then(Value::as_str),
                    Some("failed" | "error" | "incomplete")
                ) {
                    counters.insert("tool_error_signals".to_owned(), 1);
                }
            }
            Some(value) if value.ends_with("call_output") => {
                if matches!(
                    payload.get("status").and_then(Value::as_str),
                    Some("failed" | "error" | "incomplete")
                ) {
                    counters.insert("tool_error_signals".to_owned(), 1);
                }
            }
            _ => return false,
        },
        _ => return false,
    }
    if content_analysis && counters.contains_key("human_turns") {
        add_content_patterns(&mut counters, payload);
    }
    tally.event(timestamp, counters);
    true
}

fn add_claude_usage(
    counters: &mut BTreeMap<String, u64>,
    usage: Option<&Value>,
    tally: &mut FileTally,
) {
    let Some(usage) = usage else {
        return;
    };
    let mut seen = false;
    for (source, destination) in [
        ("input_tokens", "input_tokens"),
        ("output_tokens", "output_tokens"),
        ("cache_read_input_tokens", "cached_input_tokens"),
        ("cache_creation_input_tokens", "cache_write_input_tokens"),
    ] {
        if let Some(value) = usage.get(source).and_then(Value::as_u64) {
            add(counters, destination, value);
            seen = true;
        }
    }
    if seen {
        tally.saw_tokens = true;
    }
}

fn add_codex_usage(
    counters: &mut BTreeMap<String, u64>,
    info: Option<&Value>,
    tally: &mut FileTally,
) {
    let Some(info) = info else {
        return;
    };
    let total =
        info.pointer("/total_token_usage/total_tokens").and_then(Value::as_u64).unwrap_or_default();
    if total <= tally.token_total_marker {
        return;
    }
    tally.token_total_marker = total;
    let Some(last) = info.get("last_token_usage") else {
        return;
    };
    let mut seen = false;
    for (source, destination) in [
        ("input_tokens", "input_tokens"),
        ("cached_input_tokens", "cached_input_tokens"),
        ("cache_write_input_tokens", "cache_write_input_tokens"),
        ("output_tokens", "output_tokens"),
        ("reasoning_output_tokens", "reasoning_output_tokens"),
        ("total_tokens", "total_tokens"),
    ] {
        if let Some(value) = last.get(source).and_then(Value::as_u64) {
            add(counters, destination, value);
            seen = true;
        }
    }
    if seen {
        tally.saw_tokens = true;
    }
}

fn observe_shell_history(
    definition: &AdapterDefinition,
    paths: &[PathBuf],
    content_analysis: bool,
) -> AdapterMeasurement {
    if !content_analysis {
        return measurement(
            coverage(
                definition,
                CoverageStatus::Disabled,
                0,
                vec!["content_analysis_not_approved"],
            ),
            Vec::new(),
            Vec::new(),
            0,
            BTreeSet::new(),
        );
    }
    let mut commands = 0_u64;
    let mut found = false;
    let mut permission_denied = false;
    let mut unsafe_input = false;
    for path in paths {
        if !path_exists_without_following(path) {
            continue;
        }
        found = true;
        let Some(metadata) = safe_regular_file(path) else {
            unsafe_input = true;
            continue;
        };
        if metadata.len() > MAX_SHELL_HISTORY_BYTES {
            unsafe_input = true;
            continue;
        }
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                permission_denied = true;
                continue;
            }
            Err(_) => {
                unsafe_input = true;
                continue;
            }
        };
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                unsafe_input = true;
                continue;
            };
            if !line.trim().is_empty()
                && !line.contains('\u{1b}')
                && line.len() <= MAX_JSONL_LINE_BYTES
            {
                commands += 1;
            }
        }
    }
    let status = if commands > 0 {
        CoverageStatus::Observed
    } else if permission_denied {
        CoverageStatus::PermissionDenied
    } else if found || unsafe_input {
        CoverageStatus::UnsupportedVersion
    } else {
        CoverageStatus::Missing
    };
    let mut reasons = Vec::new();
    if unsafe_input {
        reasons.push("unsafe_or_malformed_history_input");
    }
    if permission_denied {
        reasons.push("source_permission_denied");
    }
    if reasons.is_empty() && status == CoverageStatus::Missing {
        reasons.push("fixed_location_not_found");
    }
    measurement(
        coverage(definition, status, commands, reasons),
        Vec::new(),
        Vec::new(),
        commands,
        BTreeSet::new(),
    )
}

fn observe_worktrees(definition: &AdapterDefinition, roots: &[PathBuf]) -> AdapterMeasurement {
    let mut count = 0_u64;
    let mut found = false;
    let mut denied = false;
    for root in roots {
        match fs::read_dir(root.join("worktrees")) {
            Ok(entries) => {
                found = true;
                count += entries
                    .flatten()
                    .take(MAX_FILES)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .count() as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => denied = true,
            Err(_) => {}
        }
    }
    let status = if found {
        CoverageStatus::Observed
    } else if denied {
        CoverageStatus::PermissionDenied
    } else {
        CoverageStatus::Missing
    };
    measurement(
        coverage(
            definition,
            status,
            count,
            vec![if found {
                "git_metadata_only"
            } else if denied {
                "source_permission_denied"
            } else {
                "fixed_location_not_found"
            }],
        ),
        Vec::new(),
        Vec::new(),
        0,
        BTreeSet::new(),
    )
}

fn standard_metrics() -> BTreeSet<String> {
    [
        "active_days",
        "retained_sessions",
        "human_turns",
        "agent_turns",
        "tool_operations",
        "tool_error_signals",
        "interrupt_signals",
        "peak_overlapping_sessions",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn measurement(
    coverage: Coverage,
    events: Vec<ObservationEvent>,
    spans: Vec<SessionSpan>,
    untimestamped_records: u64,
    supported_metrics: BTreeSet<String>,
) -> AdapterMeasurement {
    AdapterMeasurement {
        source_definition_version: source_definition_version(&coverage.adapter_id).to_owned(),
        coverage,
        supported_metrics,
        events,
        session_spans: spans,
        untimestamped_records,
    }
}

fn disabled(definition: &AdapterDefinition) -> AdapterMeasurement {
    measurement(
        coverage(definition, CoverageStatus::Disabled, 0, vec!["adapter_not_approved_by_consent"]),
        Vec::new(),
        Vec::new(),
        0,
        BTreeSet::new(),
    )
}

fn field_disabled(definition: &AdapterDefinition, reason: &str) -> AdapterMeasurement {
    measurement(
        coverage(definition, CoverageStatus::Disabled, 0, vec![reason]),
        Vec::new(),
        Vec::new(),
        0,
        BTreeSet::new(),
    )
}

fn coverage(
    definition: &AdapterDefinition,
    status: CoverageStatus,
    observed_records: u64,
    reasons: Vec<&str>,
) -> Coverage {
    Coverage {
        adapter_id: definition.id.clone(),
        adapter_version: ADAPTER_REGISTRY_VERSION.to_owned(),
        status,
        observed_records,
        missingness_reasons: reasons.into_iter().map(str::to_owned).collect(),
    }
}

fn add(counters: &mut BTreeMap<String, u64>, key: &str, value: u64) {
    if value > 0 {
        *counters.entry(key.to_owned()).or_default() += value;
    }
}

fn parse_timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn content_block_counts(content: Option<&Value>) -> (u64, u64) {
    let Some(blocks) = content.and_then(Value::as_array) else {
        return (0, 0);
    };
    let tools = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .count() as u64;
    let errors = blocks
        .iter()
        .filter(|block| block.get("is_error").and_then(Value::as_bool) == Some(true))
        .count() as u64;
    (tools, errors)
}

fn user_content_counts(content: Option<&Value>) -> (u64, u64) {
    match content {
        Some(Value::String(text)) if !text.is_empty() => (1, 0),
        Some(Value::Array(blocks)) => {
            let text = blocks
                .iter()
                .filter(|block| {
                    matches!(block.get("type").and_then(Value::as_str), Some("text" | "input_text"))
                        && block
                            .get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|text| !text.is_empty())
                })
                .count() as u64;
            let errors = blocks
                .iter()
                .filter(|block| {
                    block.get("type").and_then(Value::as_str) == Some("tool_result")
                        && block.get("is_error").and_then(Value::as_bool) == Some(true)
                })
                .count() as u64;
            (text, errors)
        }
        _ => (0, 0),
    }
}

fn codex_user_text(payload: &Value) -> bool {
    payload.get("content").and_then(Value::as_array).is_some_and(|blocks| {
        blocks.iter().any(|block| {
            matches!(block.get("type").and_then(Value::as_str), Some("input_text" | "text"))
                && block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
        })
    })
}

fn add_content_patterns(counters: &mut BTreeMap<String, u64>, value: &Value) {
    let text = content_text(value);
    if text.is_empty() {
        return;
    }
    add(counters, "content_user_messages", 1);
    let folded = text.trim_start().to_ascii_lowercase();
    if ["no ", "wrong", "stop", "fix", "you missed", "you ignored"]
        .iter()
        .any(|prefix| folded.starts_with(prefix))
    {
        add(counters, "correction_signals", 1);
    }
    if text.contains("```") {
        add(counters, "manual_relay_messages", 1);
    }
    if ["/turn", "/goal", "director", "executor", "subagent"]
        .iter()
        .any(|pattern| folded.contains(pattern))
    {
        add(counters, "routing_messages", 1);
    }
}

fn content_text(value: &Value) -> String {
    let content = value.pointer("/message/content").or_else(|| value.get("content"));
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

struct BoundedFiles {
    files: Vec<PathBuf>,
    any_candidate: bool,
    truncated: bool,
}
fn bounded_files(
    root: &Path,
    max_depth: usize,
    max_files: usize,
    predicate: impl Fn(&str) -> bool,
) -> BoundedFiles {
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut files = Vec::new();
    let mut any_candidate = false;
    let mut truncated = false;
    let mut total_bytes = 0_u64;
    while let Some((directory, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() && depth < max_depth {
                pending.push((entry.path(), depth + 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !predicate(&name) {
                continue;
            }
            any_candidate = true;
            let size = entry.metadata().map_or(u64::MAX, |metadata| metadata.len());
            if files.len() == max_files || total_bytes.saturating_add(size) > MAX_TOTAL_BYTES {
                truncated = true;
            } else {
                total_bytes += size;
                files.push(entry.path());
            }
        }
    }
    files.sort();
    BoundedFiles { files, any_candidate, truncated }
}

fn safe_regular_file(path: &Path) -> Option<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_file() && !is_hard_link(&metadata) { Some(metadata) } else { None }
}
#[cfg(unix)]
fn is_hard_link(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    metadata.nlink() != 1
}
#[cfg(not(unix))]
fn is_hard_link(_: &fs::Metadata) -> bool {
    false
}
fn path_exists_without_following(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.file_type().is_symlink())
}

#[cfg(test)]
mod tests {
    use super::*;
    use observer_core::adapter_registry;
    use tempfile::tempdir;
    fn definition(id: &str) -> AdapterDefinition {
        adapter_registry()
            .unwrap()
            .adapters
            .into_iter()
            .find(|definition| definition.id == id)
            .unwrap()
    }
    #[test]
    fn claude_parses_actual_shape_without_exporting_content() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("projects/project");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("session.jsonl"),
            include_str!("../../../fixtures/adapters/claude-code/positive.jsonl"),
        )
        .unwrap();
        let observation =
            ClaudeCodeAdapter::new(definition("claude-code"), directory.path().join("projects"))
                .observe(CollectionPermission::metadata_only(true));
        assert_eq!(observation.coverage.status, CoverageStatus::Observed);
        assert_eq!(observation.events.len(), 3);
        assert!(
            observation.events.iter().any(|event| event.counters.get("input_tokens") == Some(&7))
        );
        assert_eq!(
            observation
                .events
                .iter()
                .filter_map(|event| event.counters.get("human_turns"))
                .sum::<u64>(),
            1
        );
        assert_eq!(
            observation
                .events
                .iter()
                .filter_map(|event| event.counters.get("tool_error_signals"))
                .sum::<u64>(),
            1
        );
        assert!(!observation.supported_metrics.contains("correction_signals"));
    }

    #[test]
    fn content_heuristics_require_explicit_message_text_grant() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("projects/project");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("session.jsonl"),
            include_str!("../../../fixtures/adapters/claude-code/positive.jsonl"),
        )
        .unwrap();
        let adapter =
            ClaudeCodeAdapter::new(definition("claude-code"), directory.path().join("projects"));
        let denied = adapter.observe(CollectionPermission::metadata_only(true));
        assert!(!denied.supported_metrics.contains("manual_relay_ratio"));
        let granted = adapter.observe(CollectionPermission {
            adapter_approved: true,
            content_analysis: true,
            approved_field_classes: [
                FieldClass::FilesystemMetadata,
                FieldClass::Timestamps,
                FieldClass::EventTypes,
                FieldClass::Counters,
                FieldClass::MessageText,
            ]
            .into_iter()
            .collect(),
        });
        assert!(granted.supported_metrics.contains("manual_relay_ratio"));
        assert_eq!(
            granted
                .events
                .iter()
                .filter_map(|event| event.counters.get("manual_relay_messages"))
                .sum::<u64>(),
            1
        );
    }
    #[test]
    fn codex_deduplicates_cumulative_token_receipts() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("sessions/2026/08/18");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("rollout.jsonl"),
            include_str!("../../../fixtures/adapters/codex/positive.jsonl"),
        )
        .unwrap();
        let observation = CodexAdapter::new(definition("codex"), directory.path().join("sessions"))
            .observe(CollectionPermission::metadata_only(true));
        let input = observation
            .events
            .iter()
            .filter_map(|event| event.counters.get("input_tokens"))
            .sum::<u64>();
        assert_eq!(input, 11);
    }
    #[test]
    fn cursor_stays_detected_unmeasured() {
        let directory = tempdir().unwrap();
        let root = directory.path().join("cursor");
        fs::create_dir(&root).unwrap();
        let observation = CursorAdapter::new(definition("cursor"), vec![root])
            .observe(CollectionPermission::metadata_only(true));
        assert_eq!(observation.coverage.status, CoverageStatus::DetectedUnmeasured);
    }
    #[test]
    fn absent_root_is_missing() {
        let directory = tempdir().unwrap();
        let observation = CodexAdapter::new(definition("codex"), directory.path().join("absent"))
            .observe(CollectionPermission::metadata_only(true));
        assert_eq!(observation.coverage.status, CoverageStatus::Missing);
    }
}
