#![forbid(unsafe_code)]

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use rand::Rng as _;
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const CONSENT_CONTRACT_VERSION: &str = "second-observer.consent/v1";
pub const EXPORT_CONTRACT_VERSION: &str = "second-observer.study-export/v1";
pub const ADAPTER_REGISTRY_VERSION: &str = "second-observer.adapters/v1";
pub const METRIC_REGISTRY_VERSION: &str = "second-observer.metrics/v1";
pub const ZERO_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const REQUIRED_PROHIBITED_FIELDS: [&str; 9] = [
    "raw_prompts",
    "raw_commands",
    "transcripts",
    "tool_output",
    "paths",
    "repository_names",
    "remotes",
    "urls",
    "stable_machine_identifiers",
];

pub const NONCLAIMS: [&str; 6] = [
    "not complete history",
    "not productivity",
    "not correctness",
    "not causal effect",
    "not cost savings",
    "not provider billing receipt",
];

const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
const REGISTERED_ADAPTERS: [(&str, bool); 9] = [
    ("claude-code", true),
    ("codex", true),
    ("cursor", false),
    ("git-worktrees", false),
    ("shell-history", true),
    ("second", false),
    ("zed", false),
    ("vscode-copilot", false),
    ("warp", false),
];

const REGISTERED_METRICS: [(&str, &str, EvidenceClass); 19] = [
    ("active_days", "days", EvidenceClass::DeterministicDerived),
    ("retained_sessions", "sessions", EvidenceClass::ObservedCounter),
    ("human_turns", "turns", EvidenceClass::ObservedCounter),
    ("agent_turns", "turns", EvidenceClass::ObservedCounter),
    ("tool_operations", "operations", EvidenceClass::ObservedCounter),
    ("tool_error_signals", "signals", EvidenceClass::ObservedCounter),
    ("interrupt_signals", "signals", EvidenceClass::ObservedCounter),
    ("peak_overlapping_sessions", "sessions", EvidenceClass::DeterministicDerived),
    ("input_tokens", "tokens", EvidenceClass::ObservedCounter),
    ("cached_input_tokens", "tokens", EvidenceClass::ObservedCounter),
    ("cache_write_input_tokens", "tokens", EvidenceClass::ObservedCounter),
    ("output_tokens", "tokens", EvidenceClass::ObservedCounter),
    ("reasoning_output_tokens", "tokens", EvidenceClass::ObservedCounter),
    ("total_tokens", "tokens", EvidenceClass::ObservedCounter),
    ("manual_relay_ratio", "ratio", EvidenceClass::LocalContentHeuristic),
    ("routing_message_ratio", "ratio", EvidenceClass::LocalContentHeuristic),
    ("correction_signals", "signals", EvidenceClass::LocalContentHeuristic),
    ("git_commits", "commits", EvidenceClass::ObservedCounter),
    ("linked_worktrees", "worktrees", EvidenceClass::ObservedCounter),
];

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid contract: {0}")]
    InvalidContract(String),
    #[error("payload digest mismatch")]
    DigestMismatch,
    #[error("payload is not canonical JSON")]
    NonCanonicalJson,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentManifest {
    pub contract_version: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub approved_adapters: Vec<String>,
    pub approved_field_classes: Vec<FieldClass>,
    pub prohibited_field_classes: Vec<String>,
    pub content_analysis: bool,
    pub windows: Vec<WindowKind>,
}

impl ConsentManifest {
    #[must_use]
    pub fn metadata_first(
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        adapters: Vec<String>,
    ) -> Self {
        Self {
            contract_version: CONSENT_CONTRACT_VERSION.to_owned(),
            created_at: now,
            expires_at,
            approved_adapters: sorted_unique(adapters),
            approved_field_classes: vec![
                FieldClass::FilesystemMetadata,
                FieldClass::Timestamps,
                FieldClass::EventTypes,
                FieldClass::Counters,
            ],
            prohibited_field_classes: vec![
                "raw_prompts".to_owned(),
                "raw_commands".to_owned(),
                "transcripts".to_owned(),
                "tool_output".to_owned(),
                "paths".to_owned(),
                "repository_names".to_owned(),
                "remotes".to_owned(),
                "urls".to_owned(),
                "stable_machine_identifiers".to_owned(),
            ],
            content_analysis: false,
            windows: vec![WindowKind::RetainedHistory, WindowKind::Baseline],
        }
    }

    /// # Errors
    ///
    /// Returns an error when the manifest is expired or violates the consent contract.
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), DomainError> {
        if self.contract_version != CONSENT_CONTRACT_VERSION {
            return Err(DomainError::InvalidContract(
                "unsupported consent contract version".to_owned(),
            ));
        }
        if self.expires_at <= self.created_at || self.expires_at <= now {
            return Err(DomainError::InvalidContract("consent manifest is expired".to_owned()));
        }
        let phase_count = usize::from(self.windows.contains(&WindowKind::Baseline))
            + usize::from(self.windows.contains(&WindowKind::Post));
        if self.windows.len() != 2
            || !self.windows.contains(&WindowKind::RetainedHistory)
            || phase_count != 1
        {
            return Err(DomainError::InvalidContract(
                "consent must approve retained_history and exactly one measurement phase".to_owned(),
            ));
        }
        if self.approved_adapters.is_empty()
            || self.approved_field_classes.is_empty()
            || !is_unique(&self.approved_adapters)
            || !is_unique_values(&self.approved_field_classes)
            || !is_unique(&self.prohibited_field_classes)
            || self.prohibited_field_classes.is_empty()
        {
            return Err(DomainError::InvalidContract(
                "consent lists must be unique and nonempty".to_owned(),
            ));
        }
        if REQUIRED_PROHIBITED_FIELDS
            .iter()
            .any(|required| !self.prohibited_field_classes.iter().any(|field| field == required))
        {
            return Err(DomainError::InvalidContract(
                "consent does not retain all required prohibited field classes".to_owned(),
            ));
        }
        let content_fields = [FieldClass::MessageText, FieldClass::CommandText];
        if !self.content_analysis
            && self.approved_field_classes.iter().any(|field| content_fields.contains(field))
        {
            return Err(DomainError::InvalidContract(
                "content fields require content_analysis=true".to_owned(),
            ));
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when the manifest cannot be represented as canonical JSON.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, DomainError> {
        canonical_json(self)
    }

    /// # Errors
    ///
    /// Returns an error when the manifest cannot be represented as canonical JSON.
    pub fn digest(&self) -> Result<String, DomainError> {
        Ok(sha256_hex(&self.canonical_bytes()?))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldClass {
    FilesystemMetadata,
    Timestamps,
    EventTypes,
    Counters,
    MessageText,
    CommandText,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    RetainedHistory,
    Baseline,
    Post,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Observed,
    DetectedUnmeasured,
    UnsupportedVersion,
    PermissionDenied,
    Missing,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    ObservedCounter,
    DeterministicDerived,
    LocalContentHeuristic,
    Estimated,
    Unknown,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ComparabilityDisposition {
    #[serde(rename = "COMPARABLE_DESCRIPTIVE")]
    ComparableDescriptive,
    #[serde(rename = "PARTIAL")]
    Partial,
    #[serde(rename = "INCOMPARABLE")]
    Incomparable,
    #[serde(rename = "COLLECTION_FAILED")]
    CollectionFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Collector {
    pub version: String,
    pub binary_sha256: String,
    pub configuration_sha256: String,
    pub metric_registry_version: String,
    pub adapter_registry_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Study {
    pub participant_id: String,
    pub device_id: String,
    pub run_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportConsent {
    pub manifest_sha256: String,
    pub approved_adapters: Vec<String>,
    pub approved_field_classes: Vec<FieldClass>,
    pub content_analysis: bool,
    pub collection_approved_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionWindow {
    pub id: String,
    pub kind: WindowKind,
    pub start: Option<DateTime<Utc>>,
    pub end: DateTime<Utc>,
    pub timezone: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub adapter_id: String,
    pub adapter_version: String,
    pub status: CoverageStatus,
    pub observed_records: u64,
    pub missingness_reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricValue {
    pub metric_id: String,
    pub adapter_id: String,
    pub window_id: String,
    pub source_definition_version: String,
    pub evidence_class: EvidenceClass,
    pub unit: String,
    #[serde(serialize_with = "serialize_metric_value")]
    pub value: Option<f64>,
    pub eligible_count: u64,
    pub observed_count: u64,
    pub missing_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Comparability {
    pub disposition: ComparabilityDisposition,
    pub blocking_mismatches: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    pub forbidden_fields_absent: bool,
    pub content_persisted: bool,
    pub content_exported: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Integrity {
    pub payload_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StudyExport {
    pub contract_version: String,
    pub collector: Collector,
    pub study: Study,
    pub consent: ExportConsent,
    pub windows: Vec<CollectionWindow>,
    pub coverage: Vec<Coverage>,
    pub metrics: Vec<MetricValue>,
    pub comparability: Comparability,
    pub privacy: Privacy,
    pub integrity: Integrity,
    pub nonclaims: Vec<String>,
}

impl StudyExport {
    /// # Errors
    ///
    /// Returns an error when the export violates the contract or cannot be canonicalized.
    pub fn finalize(&mut self) -> Result<(), DomainError> {
        ZERO_SHA256.clone_into(&mut self.integrity.payload_sha256);
        self.integrity.payload_sha256 = sha256_hex(&canonical_json(self)?);
        self.validate_contract()?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when the export cannot be canonicalized.
    pub fn normalized_digest(&self) -> Result<String, DomainError> {
        let mut normalized = self.clone();
        ZERO_SHA256.clone_into(&mut normalized.integrity.payload_sha256);
        Ok(sha256_hex(&canonical_json(&normalized)?))
    }

    /// # Errors
    ///
    /// Returns an error when the export cannot be represented as canonical JSON.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, DomainError> {
        canonical_json(self)
    }

    /// # Errors
    ///
    /// Returns an error when a required export contract invariant does not hold.
    #[allow(clippy::too_many_lines)]
    pub fn validate_contract(&self) -> Result<(), DomainError> {
        if self.contract_version != EXPORT_CONTRACT_VERSION {
            return Err(DomainError::InvalidContract(
                "unsupported export contract version".to_owned(),
            ));
        }
        if self.collector.metric_registry_version != METRIC_REGISTRY_VERSION
            || self.collector.adapter_registry_version != ADAPTER_REGISTRY_VERSION
        {
            return Err(DomainError::InvalidContract("unexpected registry version".to_owned()));
        }
        for hash in [
            &self.collector.binary_sha256,
            &self.collector.configuration_sha256,
            &self.consent.manifest_sha256,
            &self.integrity.payload_sha256,
        ] {
            if !is_sha256(hash) {
                return Err(DomainError::InvalidContract("invalid SHA-256 value".to_owned()));
            }
        }
        if self.collector.binary_sha256 == ZERO_SHA256 {
            return Err(DomainError::InvalidContract(
                "collector binary digest must identify the executing binary".to_owned(),
            ));
        }
        if !is_scoped_id(&self.study.participant_id)
            || !is_scoped_id(&self.study.device_id)
            || !is_scoped_id(&self.study.run_id)
        {
            return Err(DomainError::InvalidContract("invalid scoped study identifier".to_owned()));
        }
        if self.consent.approved_adapters.is_empty()
            || !is_unique(&self.consent.approved_adapters)
            || self
                .consent
                .approved_adapters
                .iter()
                .any(|adapter| registered_adapter(adapter).is_none())
        {
            return Err(DomainError::InvalidContract(
                "consent contains an unknown or duplicate adapter".to_owned(),
            ));
        }
        if self.consent.approved_field_classes.is_empty()
            || !is_unique_values(&self.consent.approved_field_classes)
        {
            return Err(DomainError::InvalidContract(
                "consent contains an empty or duplicate field class list".to_owned(),
            ));
        }
        let permits_content = self
            .consent
            .approved_field_classes
            .iter()
            .any(|field| matches!(field, FieldClass::MessageText | FieldClass::CommandText));
        if permits_content != self.consent.content_analysis {
            return Err(DomainError::InvalidContract(
                "content analysis and approved content field classes disagree".to_owned(),
            ));
        }
        if self.windows.len() < 2
            || !self.windows.iter().any(|window| window.kind == WindowKind::RetainedHistory)
            || (usize::from(
                self.windows.iter().any(|window| window.kind == WindowKind::Baseline),
            ) + usize::from(
                self.windows.iter().any(|window| window.kind == WindowKind::Post),
            )) != 1
        {
            return Err(DomainError::InvalidContract(
                "required collection windows are absent".to_owned(),
            ));
        }
        if self.windows.len() != 2
            || !is_unique(&self.windows.iter().map(|window| window.id.clone()).collect::<Vec<_>>())
            || self.windows.iter().any(|window| {
                window.id.trim().is_empty()
                    || window.timezone.trim().is_empty()
                    || window.start.is_some_and(|start| start >= window.end)
            })
        {
            return Err(DomainError::InvalidContract(
                "collection windows are malformed".to_owned(),
            ));
        }
        if self.coverage.iter().any(|coverage| {
            !self.consent.approved_adapters.contains(&coverage.adapter_id)
                || coverage.adapter_version.trim().is_empty()
                || coverage.observed_records > MAX_SAFE_JSON_INTEGER
                || !is_unique(&coverage.missingness_reasons)
        }) {
            return Err(DomainError::InvalidContract(
                "coverage exceeds consent or JSON-safe limits".to_owned(),
            ));
        }
        if !self.privacy.forbidden_fields_absent
            || self.privacy.content_persisted
            || self.privacy.content_exported
        {
            return Err(DomainError::InvalidContract("privacy assertions failed".to_owned()));
        }
        if self.nonclaims.len() < 4 {
            return Err(DomainError::InvalidContract("required nonclaims are absent".to_owned()));
        }
        if self.metrics.iter().any(|metric| metric.value.is_some_and(|value| !value.is_finite())) {
            return Err(DomainError::InvalidContract(
                "metric value must be finite or null".to_owned(),
            ));
        }
        if self.metrics.iter().any(|metric| metric.source_definition_version.trim().is_empty()) {
            return Err(DomainError::InvalidContract(
                "metric source definition version must not be empty".to_owned(),
            ));
        }
        if self.metrics.iter().any(|metric| {
            let Some((unit, evidence)) = registered_metric(&metric.metric_id) else {
                return true;
            };
            unit != metric.unit
                || evidence != metric.evidence_class
                || !self.consent.approved_adapters.contains(&metric.adapter_id)
                || !self.windows.iter().any(|window| window.id == metric.window_id)
                || metric.eligible_count > MAX_SAFE_JSON_INTEGER
                || metric.observed_count > MAX_SAFE_JSON_INTEGER
                || metric.missing_count > MAX_SAFE_JSON_INTEGER
                || (metric.evidence_class == EvidenceClass::LocalContentHeuristic
                    && (!self.consent.content_analysis
                        || !registered_adapter(&metric.adapter_id)
                            .is_some_and(|(_, capable)| capable)))
        }) {
            return Err(DomainError::InvalidContract(
                "metric exceeds registry, consent, or JSON-safe limits".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Parses canonical JSON and verifies its normalized integrity digest.
///
/// # Errors
///
/// Returns an error when JSON is malformed, noncanonical, or violates the export contract.
pub fn parse_and_verify_export(bytes: &[u8]) -> Result<StudyExport, DomainError> {
    let export: StudyExport = serde_json::from_slice(bytes)?;
    if export.canonical_bytes()? != bytes {
        return Err(DomainError::NonCanonicalJson);
    }
    export.validate_contract()?;
    if export.normalized_digest()? != export.integrity.payload_sha256 {
        return Err(DomainError::DigestMismatch);
    }
    Ok(export)
}

/// Serializes a value using recursively lexicographic object keys.
///
/// # Errors
///
/// Returns an error when serialization to JSON fails.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, DomainError> {
    // serde_json::Map uses lexicographic keys without the preserve_order feature. Serializing a
    // Value therefore matches the Worker canonicalizer's recursively sorted object-key order.
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}

#[allow(clippy::ref_option)] // serde's `serialize_with` callback receives a field reference.
fn serialize_metric_value<S>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        None => serializer.serialize_none(),
        Some(value) if !value.is_finite() => {
            Err(serde::ser::Error::custom("metric value must be finite"))
        }
        Some(value) if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 => {
            let integer = value.to_string().parse::<i64>().map_err(|_| {
                serde::ser::Error::custom("safe integral metric value must fit i64")
            })?;
            serializer.serialize_i64(integer)
        }
        Some(value) => serializer.serialize_f64(*value),
    }
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[must_use]
pub fn random_scoped_id() -> String {
    let mut bytes = [0_u8; 18];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_scoped_id(value: &str) -> bool {
    (8..=80).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn registered_adapter(value: &str) -> Option<(&'static str, bool)> {
    REGISTERED_ADAPTERS.iter().copied().find(|(adapter, _)| *adapter == value)
}

fn registered_metric(value: &str) -> Option<(&'static str, EvidenceClass)> {
    REGISTERED_METRICS
        .iter()
        .find(|(metric, _, _)| *metric == value)
        .map(|(_, unit, evidence)| (*unit, evidence.clone()))
}

fn is_unique(values: &[String]) -> bool {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).all(|pair| pair[0] != pair[1])
}

fn is_unique_values<T: Ord + Clone>(values: &[T]) -> bool {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).all(|pair| pair[0] != pair[1])
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort_unstable();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_contract_fixture_is_canonical_and_integrity_valid() {
        let fixture = include_bytes!("../../../fixtures/contracts/study-export-v1.json");
        let export = parse_and_verify_export(fixture).expect("fixture must verify");
        assert_eq!(export.canonical_bytes().expect("fixture bytes"), fixture);
        assert_eq!(export.metrics[0].value, Some(20.0));
        assert_eq!(export.metrics[1].value, Some(0.125));
    }

    #[test]
    fn verification_rejects_unregistered_or_unconsented_contract_values() {
        let fixture = include_bytes!("../../../fixtures/contracts/study-export-v1.json");
        let export = parse_and_verify_export(fixture).expect("fixture must verify");

        let mut unknown_adapter = export.clone();
        unknown_adapter.consent.approved_adapters = vec!["unregistered-adapter".to_owned()];
        assert!(unknown_adapter.validate_contract().is_err());

        let mut content_without_consent = export.clone();
        content_without_consent.consent.approved_field_classes.push(FieldClass::MessageText);
        assert!(content_without_consent.validate_contract().is_err());

        let mut wrong_metric_definition = export;
        wrong_metric_definition.metrics[0].unit = "tokens".to_owned();
        assert!(wrong_metric_definition.validate_contract().is_err());
    }

    #[test]
    fn verification_rejects_unsafe_json_counts() {
        let fixture = include_bytes!("../../../fixtures/contracts/study-export-v1.json");
        let mut export = parse_and_verify_export(fixture).expect("fixture must verify");
        export.metrics[0].observed_count = MAX_SAFE_JSON_INTEGER + 1;
        assert!(export.validate_contract().is_err());
    }

    #[test]
    fn verification_registry_matches_the_public_registry_files() {
        let adapters: serde_json::Value =
            serde_json::from_str(include_str!("../../../registry/adapters-v1.json"))
                .expect("adapter registry JSON");
        let adapter_entries = adapters["adapters"].as_array().expect("adapter entries");
        assert_eq!(adapter_entries.len(), REGISTERED_ADAPTERS.len());
        for (id, content_capable) in REGISTERED_ADAPTERS {
            assert!(
                adapter_entries.iter().any(|entry| {
                    entry["id"] == id && entry["content_capable"] == content_capable
                })
            );
        }

        let metrics: serde_json::Value =
            serde_json::from_str(include_str!("../../../registry/metrics-v1.json"))
                .expect("metric registry JSON");
        let metric_entries = metrics["metrics"].as_array().expect("metric entries");
        assert_eq!(metric_entries.len(), REGISTERED_METRICS.len());
        for (id, unit, evidence) in REGISTERED_METRICS {
            assert!(metric_entries.iter().any(|entry| {
                entry["id"] == id
                    && entry["unit"] == unit
                    && entry["evidence"] == serde_json::to_value(&evidence).expect("evidence")
            }));
        }
    }
}
