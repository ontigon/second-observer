#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write as _,
    path::Path,
};

use chrono::{DateTime, Days, LocalResult, NaiveTime, TimeZone as _, Utc};
use chrono_tz::Tz;
use observer_domain::{
    ADAPTER_REGISTRY_VERSION, Collector, Comparability, ComparabilityDisposition, ConsentManifest,
    Coverage, CoverageStatus, DomainError, EXPORT_CONTRACT_VERSION, ExportConsent, Integrity,
    METRIC_REGISTRY_VERSION, MetricValue, NONCLAIMS, Privacy, Study, StudyExport, WindowKind,
    ZERO_SHA256, parse_and_verify_export, random_scoped_id, sha256_hex,
};
use serde::Deserialize;
use thiserror::Error;

const ADAPTER_REGISTRY_JSON: &str = include_str!("../../../registry/adapters-v1.json");

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("adapter registry is invalid: {0}")]
    Registry(#[from] serde_json::Error),
    #[error("unknown adapter in consent manifest: {0}")]
    UnknownAdapter(String),
    #[error("collection result file is absent: {0}")]
    MissingCollection(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct AdapterRegistry {
    pub registry_version: String,
    pub adapters: Vec<AdapterDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct AdapterDefinition {
    pub id: String,
    pub measurement: String,
    pub initial_status: String,
    pub content_capable: bool,
}

pub fn adapter_registry() -> Result<AdapterRegistry, CoreError> {
    let registry: AdapterRegistry = serde_json::from_str(ADAPTER_REGISTRY_JSON)?;
    if registry.registry_version != ADAPTER_REGISTRY_VERSION {
        return Err(CoreError::Domain(DomainError::InvalidContract(
            "embedded adapter registry version differs from export contract".to_owned(),
        )));
    }
    Ok(registry)
}

pub trait Adapter {
    fn definition(&self) -> &AdapterDefinition;
    fn collect(&self, consented: bool) -> AdapterResult;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterResult {
    pub coverage: Coverage,
}

/// A timestamped, content-free counter record emitted by a local adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationEvent {
    pub timestamp: Option<DateTime<Utc>>,
    pub counters: BTreeMap<String, u64>,
}

/// A local session span. The identifier used to construct a span never leaves the adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSpan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// An adapter reduction containing only schema-approved aggregates and timestamps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterMeasurement {
    pub coverage: Coverage,
    pub source_definition_version: String,
    pub supported_metrics: BTreeSet<String>,
    pub events: Vec<ObservationEvent>,
    pub session_spans: Vec<SessionSpan>,
    pub untimestamped_records: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct StudyIdentity {
    pub participant_id: String,
    pub device_id: String,
}

/// Placeholder for SO-02. It deliberately reads no source location and therefore never reports
/// detected software as observed activity.
#[derive(Clone, Debug)]
pub struct RegistryPlaceholderAdapter {
    definition: AdapterDefinition,
}

impl RegistryPlaceholderAdapter {
    #[must_use]
    pub fn new(definition: AdapterDefinition) -> Self {
        Self { definition }
    }
}

impl Adapter for RegistryPlaceholderAdapter {
    fn definition(&self) -> &AdapterDefinition {
        &self.definition
    }

    fn collect(&self, consented: bool) -> AdapterResult {
        let (status, missingness_reasons) = if consented {
            (CoverageStatus::Missing, vec!["adapter_source_parser_not_implemented".to_owned()])
        } else {
            (CoverageStatus::Disabled, vec!["adapter_not_approved_by_consent".to_owned()])
        };
        AdapterResult {
            coverage: Coverage {
                adapter_id: self.definition.id.clone(),
                adapter_version: ADAPTER_REGISTRY_VERSION.to_owned(),
                status,
                observed_records: 0,
                missingness_reasons,
            },
        }
    }
}

pub fn discover() -> Result<Vec<Coverage>, CoreError> {
    let registry = adapter_registry()?;
    Ok(registry
        .adapters
        .into_iter()
        .map(|definition| RegistryPlaceholderAdapter::new(definition).collect(true).coverage)
        .collect())
}

pub fn collect(
    consent: &ConsentManifest,
    now: DateTime<Utc>,
    timezone: &str,
    collector_version: &str,
    binary_sha256: &str,
    study: Study,
) -> Result<StudyExport, CoreError> {
    consent.validate(now)?;
    if timezone.trim().is_empty() {
        return Err(CoreError::Domain(DomainError::InvalidContract(
            "timezone must not be empty".to_owned(),
        )));
    }
    let registry = adapter_registry()?;
    let registered =
        registry.adapters.iter().map(|adapter| adapter.id.as_str()).collect::<BTreeSet<_>>();
    for adapter in &consent.approved_adapters {
        if !registered.contains(adapter.as_str()) {
            return Err(CoreError::UnknownAdapter(adapter.clone()));
        }
    }

    let approved = consent.approved_adapters.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let coverage = registry
        .adapters
        .into_iter()
        .map(|definition| {
            let consented = approved.contains(definition.id.as_str());
            RegistryPlaceholderAdapter::new(definition).collect(consented).coverage
        })
        .collect::<Vec<_>>();
    let measurements = coverage
        .into_iter()
        .map(|coverage| AdapterMeasurement {
            coverage,
            source_definition_version: "placeholder/v1".to_owned(),
            supported_metrics: BTreeSet::new(),
            events: Vec::new(),
            session_spans: Vec::new(),
            untimestamped_records: 0,
        })
        .collect::<Vec<_>>();
    collect_measurements(
        consent,
        now,
        timezone,
        collector_version,
        binary_sha256,
        study,
        &measurements,
    )
}

/// Builds an export from adapter-owned content-free observations.
pub fn collect_measurements(
    consent: &ConsentManifest,
    now: DateTime<Utc>,
    timezone: &str,
    collector_version: &str,
    binary_sha256: &str,
    study: Study,
    measurements: &[AdapterMeasurement],
) -> Result<StudyExport, CoreError> {
    collect_measurements_phase(
        consent,
        now,
        timezone,
        collector_version,
        binary_sha256,
        study,
        measurements,
        WindowKind::Baseline28d,
    )
}

/// Builds an export for the consented baseline or post phase.
#[allow(clippy::too_many_arguments)]
pub fn collect_measurements_phase(
    consent: &ConsentManifest,
    now: DateTime<Utc>,
    timezone: &str,
    collector_version: &str,
    binary_sha256: &str,
    study: Study,
    measurements: &[AdapterMeasurement],
    phase: WindowKind,
) -> Result<StudyExport, CoreError> {
    consent.validate(now)?;
    if timezone.trim().is_empty() {
        return Err(CoreError::Domain(DomainError::InvalidContract(
            "timezone must not be empty".to_owned(),
        )));
    }
    let registry = adapter_registry()?;
    let registered =
        registry.adapters.iter().map(|adapter| adapter.id.as_str()).collect::<BTreeSet<_>>();
    for adapter in &consent.approved_adapters {
        if !registered.contains(adapter.as_str()) {
            return Err(CoreError::UnknownAdapter(adapter.clone()));
        }
    }
    if !matches!(phase, WindowKind::Baseline28d | WindowKind::Post28d)
        || !consent.windows.contains(&phase)
    {
        return Err(CoreError::Domain(DomainError::InvalidContract(
            "collection phase is not approved by consent".to_owned(),
        )));
    }
    let windows = collection_windows(now, timezone, phase)?;
    let coverage = measurements
        .iter()
        .filter(|measurement| consent.approved_adapters.contains(&measurement.coverage.adapter_id))
        .map(|measurement| measurement.coverage.clone())
        .collect();
    let metrics = reduce_measurements(measurements, &windows, timezone)?;
    let comparability = if metrics.iter().any(|metric| {
        metric.window_id == "baseline-28d" && metric.value.is_some() && metric.missing_count == 0
    }) {
        Comparability {
            disposition: ComparabilityDisposition::ComparableDescriptive,
            blocking_mismatches: Vec::new(),
        }
    } else {
        Comparability {
            disposition: ComparabilityDisposition::Incomparable,
            blocking_mismatches: vec!["no_complete_baseline_metric".to_owned()],
        }
    };
    let configuration_sha256 = consent.digest()?;
    let mut export = StudyExport {
        contract_version: EXPORT_CONTRACT_VERSION.to_owned(),
        collector: Collector {
            version: collector_version.to_owned(),
            binary_sha256: binary_sha256.to_owned(),
            configuration_sha256,
            metric_registry_version: METRIC_REGISTRY_VERSION.to_owned(),
            adapter_registry_version: ADAPTER_REGISTRY_VERSION.to_owned(),
        },
        study,
        consent: ExportConsent {
            manifest_sha256: consent.digest()?,
            approved_adapters: consent.approved_adapters.clone(),
            approved_field_classes: consent.approved_field_classes.clone(),
            content_analysis: consent.content_analysis,
            collection_approved_at: now,
        },
        windows,
        coverage,
        metrics,
        comparability,
        privacy: Privacy {
            forbidden_fields_absent: true,
            content_persisted: false,
            content_exported: false,
        },
        integrity: Integrity { payload_sha256: ZERO_SHA256.to_owned() },
        nonclaims: NONCLAIMS.iter().map(|value| (*value).to_owned()).collect(),
    };
    export.finalize()?;
    Ok(export)
}

#[allow(clippy::too_many_lines)]
fn reduce_measurements(
    measurements: &[AdapterMeasurement],
    windows: &[observer_domain::CollectionWindow],
    timezone: &str,
) -> Result<Vec<MetricValue>, CoreError> {
    let timezone: Tz = timezone.parse().map_err(|_| {
        CoreError::Domain(DomainError::InvalidContract(
            "timezone must be an IANA timezone name".to_owned(),
        ))
    })?;
    let mut output = Vec::new();
    for measurement in measurements {
        if measurement.coverage.status != CoverageStatus::Observed {
            continue;
        }
        for window in windows {
            let included = measurement
                .events
                .iter()
                .filter(|event| {
                    window.start.is_none_or(|start| {
                        event
                            .timestamp
                            .is_some_and(|timestamp| timestamp >= start && timestamp < window.end)
                    })
                })
                .collect::<Vec<_>>();
            let timestamp_missing = if window.kind == WindowKind::RetainedHistory {
                0
            } else {
                measurement.untimestamped_records
            };
            for metric_id in &measurement.supported_metrics {
                let (value, observed_count, eligible_count) = if metric_id == "manual_relay_ratio"
                    || metric_id == "routing_message_ratio"
                {
                    let numerator_key = if metric_id == "manual_relay_ratio" {
                        "manual_relay_messages"
                    } else {
                        "routing_messages"
                    };
                    let denominator = included
                        .iter()
                        .filter_map(|event| event.counters.get("content_user_messages"))
                        .sum::<u64>();
                    let numerator = included
                        .iter()
                        .filter_map(|event| event.counters.get(numerator_key))
                        .sum::<u64>();
                    #[allow(clippy::cast_precision_loss)]
                    let value = (denominator > 0).then_some(numerator as f64 / denominator as f64);
                    (value, denominator, denominator + timestamp_missing)
                } else if metric_id == "retained_sessions" {
                    let spans = measurement
                        .session_spans
                        .iter()
                        .filter(|span| {
                            window
                                .start
                                .is_none_or(|start| span.end >= start && span.start < window.end)
                        })
                        .count() as u64;
                    (counter_to_f64(spans), spans, spans + timestamp_missing)
                } else if metric_id == "active_days" {
                    let dates = included
                        .iter()
                        .filter_map(|event| event.timestamp)
                        .map(|timestamp| timestamp.with_timezone(&timezone).date_naive())
                        .collect::<BTreeSet<_>>();
                    (
                        counter_to_f64(dates.len() as u64),
                        dates.len() as u64,
                        included.len() as u64 + timestamp_missing,
                    )
                } else if metric_id == "peak_overlapping_sessions" {
                    let spans = measurement
                        .session_spans
                        .iter()
                        .filter(|span| {
                            window
                                .start
                                .is_none_or(|start| span.end >= start && span.start < window.end)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    (
                        counter_to_f64(peak_overlaps(&spans, window.start, window.end)),
                        spans.len() as u64,
                        spans.len() as u64,
                    )
                } else {
                    let observed_count = included
                        .iter()
                        .filter(|event| event.counters.contains_key(metric_id))
                        .count() as u64;
                    let total = included
                        .iter()
                        .filter_map(|event| event.counters.get(metric_id))
                        .sum::<u64>();
                    (
                        counter_to_f64(total),
                        observed_count,
                        included.len() as u64 + timestamp_missing,
                    )
                };
                let overflow_missing = u64::from(
                    value.is_none()
                        && observed_count > 0
                        && !matches!(
                            metric_id.as_str(),
                            "manual_relay_ratio" | "routing_message_ratio"
                        ),
                );
                let partial_missing = u64::from(
                    measurement
                        .coverage
                        .missingness_reasons
                        .iter()
                        .any(|reason| reason == "bounded_file_limit_reached"),
                );
                output.push(MetricValue {
                    metric_id: metric_id.clone(),
                    adapter_id: measurement.coverage.adapter_id.clone(),
                    window_id: window.id.clone(),
                    source_definition_version: measurement.source_definition_version.clone(),
                    evidence_class: if matches!(
                        metric_id.as_str(),
                        "active_days" | "peak_overlapping_sessions"
                    ) {
                        observer_domain::EvidenceClass::DeterministicDerived
                    } else if matches!(
                        metric_id.as_str(),
                        "manual_relay_ratio" | "routing_message_ratio" | "correction_signals"
                    ) {
                        observer_domain::EvidenceClass::LocalContentHeuristic
                    } else {
                        observer_domain::EvidenceClass::ObservedCounter
                    },
                    unit: metric_unit(metric_id).to_owned(),
                    value: if (timestamp_missing > 0 && window.kind != WindowKind::RetainedHistory)
                        || overflow_missing > 0
                        || partial_missing > 0
                    {
                        None
                    } else {
                        value
                    },
                    eligible_count,
                    observed_count,
                    missing_count: timestamp_missing + overflow_missing + partial_missing,
                });
            }
        }
    }
    output.sort_by(|left, right| {
        (&left.adapter_id, &left.window_id, &left.metric_id).cmp(&(
            &right.adapter_id,
            &right.window_id,
            &right.metric_id,
        ))
    });
    Ok(output)
}

#[allow(clippy::cast_precision_loss)]
fn counter_to_f64(value: u64) -> Option<f64> {
    const JSON_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
    (value <= JSON_SAFE_INTEGER).then_some(value as f64)
}

fn metric_unit(metric_id: &str) -> &'static str {
    match metric_id {
        "active_days" => "days",
        "retained_sessions" | "peak_overlapping_sessions" => "sessions",
        "human_turns" | "agent_turns" => "turns",
        "tool_operations" => "operations",
        "tool_error_signals" | "interrupt_signals" | "correction_signals" => "signals",
        "input_tokens"
        | "cached_input_tokens"
        | "cache_write_input_tokens"
        | "output_tokens"
        | "reasoning_output_tokens"
        | "total_tokens" => "tokens",
        "manual_relay_ratio" | "routing_message_ratio" => "ratio",
        _ => "count",
    }
}

fn peak_overlaps(spans: &[SessionSpan], start: Option<DateTime<Utc>>, end: DateTime<Utc>) -> u64 {
    let mut boundaries = Vec::with_capacity(spans.len() * 2);
    for span in spans {
        let begin = start.map_or(span.start, |window_start| span.start.max(window_start));
        let finish = span.end.min(end);
        if finish >= begin {
            boundaries.push((begin, 1_i64));
            boundaries.push((finish, -1_i64));
        }
    }
    boundaries.sort_by_key(|boundary| (boundary.0, boundary.1));
    let mut active = 0_i64;
    let mut peak = 0_i64;
    for (_, delta) in boundaries {
        active += delta;
        peak = peak.max(active);
    }
    u64::try_from(peak).unwrap_or_default()
}

#[must_use]
pub fn random_study() -> Study {
    Study {
        participant_id: random_scoped_id(),
        device_id: random_scoped_id(),
        run_id: random_scoped_id(),
    }
}

/// Loads a stable local study/device identity or creates one owner-only on first collection.
pub fn load_or_create_study(path: &Path) -> Result<Study, CoreError> {
    let identity = if path.exists() {
        let bytes = fs::read(path)?;
        let identity: StudyIdentity = serde_json::from_slice(&bytes)?;
        if !is_scoped_identity(&identity.participant_id) || !is_scoped_identity(&identity.device_id)
        {
            return Err(CoreError::Domain(DomainError::InvalidContract(
                "invalid local study identity".to_owned(),
            )));
        }
        identity
    } else {
        let identity =
            StudyIdentity { participant_id: random_scoped_id(), device_id: random_scoped_id() };
        atomic_write_owner_only(path, &observer_domain::canonical_json(&identity)?)?;
        identity
    };
    Ok(Study {
        participant_id: identity.participant_id,
        device_id: identity.device_id,
        run_id: random_scoped_id(),
    })
}

fn is_scoped_identity(value: &str) -> bool {
    (8..=80).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn collection_windows(
    now: DateTime<Utc>,
    timezone: &str,
    phase: WindowKind,
) -> Result<Vec<observer_domain::CollectionWindow>, CoreError> {
    let timezone: Tz = timezone.parse().map_err(|_| {
        CoreError::Domain(DomainError::InvalidContract(
            "timezone must be an IANA timezone name".to_owned(),
        ))
    })?;
    let local_today = now.with_timezone(&timezone).date_naive();
    let baseline_start_day = local_today.checked_sub_days(Days::new(28)).ok_or_else(|| {
        CoreError::Domain(DomainError::InvalidContract("baseline window underflow".to_owned()))
    })?;
    let baseline_start = local_midnight(timezone, baseline_start_day)?;
    let baseline_end = local_midnight(timezone, local_today)?;
    Ok(vec![
        observer_domain::CollectionWindow {
            id: "retained-history".to_owned(),
            kind: WindowKind::RetainedHistory,
            start: None,
            end: now,
            timezone: timezone.to_string(),
        },
        observer_domain::CollectionWindow {
            id: match phase {
                WindowKind::Baseline28d => "baseline-28d",
                WindowKind::Post28d => "post-28d",
                WindowKind::RetainedHistory => unreachable!(),
            }
            .to_owned(),
            kind: phase,
            start: Some(baseline_start),
            end: baseline_end,
            timezone: timezone.to_string(),
        },
    ])
}

fn local_midnight(timezone: Tz, day: chrono::NaiveDate) -> Result<DateTime<Utc>, CoreError> {
    match timezone.from_local_datetime(&day.and_time(NaiveTime::MIN)) {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(earliest, _) => Ok(earliest.with_timezone(&Utc)),
        LocalResult::None => Err(CoreError::Domain(DomainError::InvalidContract(
            "timezone has no valid local midnight for baseline boundary".to_owned(),
        ))),
    }
}

pub fn write_consent(path: &Path, consent: &ConsentManifest) -> Result<(), CoreError> {
    atomic_write_owner_only(path, &consent.canonical_bytes()?)
}

pub fn load_consent(path: &Path, now: DateTime<Utc>) -> Result<ConsentManifest, CoreError> {
    let bytes = fs::read(path)?;
    let consent: ConsentManifest = serde_json::from_slice(&bytes)?;
    if consent.canonical_bytes()? != bytes {
        return Err(CoreError::Domain(DomainError::NonCanonicalJson));
    }
    consent.validate(now)?;
    Ok(consent)
}

pub fn write_pending(path: &Path, export: &StudyExport) -> Result<(), CoreError> {
    atomic_write_owner_only(path, &export.canonical_bytes()?)
}

pub fn load_pending(path: &Path) -> Result<StudyExport, CoreError> {
    if !path.exists() {
        return Err(CoreError::MissingCollection(path.display().to_string()));
    }
    Ok(parse_and_verify_export(&fs::read(path)?)?)
}

pub fn export_finalized(pending: &Path, destination: &Path) -> Result<String, CoreError> {
    let export = load_pending(pending)?;
    let payload = export.canonical_bytes()?;
    atomic_write_owner_only(destination, &payload)?;
    let sidecar = destination.with_extension(format!(
        "{}sha256",
        destination.extension().and_then(|extension| extension.to_str()).unwrap_or_default()
    ));
    let digest = sha256_hex(&payload);
    atomic_write_owner_only(
        &sidecar,
        format!(
            "{digest}  {}\n",
            destination.file_name().and_then(|name| name.to_str()).unwrap_or("export")
        )
        .as_bytes(),
    )?;
    Ok(digest)
}

pub fn verify_file(path: &Path) -> Result<StudyExport, CoreError> {
    let bytes = fs::read(path)?;
    let export = parse_and_verify_export(&bytes)?;
    let sidecar = path.with_extension(format!(
        "{}sha256",
        path.extension().and_then(|extension| extension.to_str()).unwrap_or_default()
    ));
    if sidecar.exists() {
        let expected = sha256_hex(&fs::read(path)?);
        let found = fs::read_to_string(sidecar)?;
        if found.split_whitespace().next() != Some(expected.as_str()) {
            return Err(CoreError::Domain(DomainError::DigestMismatch));
        }
    }
    Ok(export)
}

/// Determines per-metric comparability. The outcome is descriptive only.
#[must_use]
pub fn compare(baseline: &StudyExport, post: &StudyExport) -> Comparability {
    if baseline.comparability.disposition == ComparabilityDisposition::CollectionFailed
        || post.comparability.disposition == ComparabilityDisposition::CollectionFailed
    {
        return Comparability {
            disposition: ComparabilityDisposition::CollectionFailed,
            blocking_mismatches: vec!["collection_failed".to_owned()],
        };
    }
    let mut gates = Vec::new();
    if baseline.study.participant_id != post.study.participant_id
        || baseline.study.device_id != post.study.device_id
    {
        gates.push("study_identity_changed".to_owned());
    }
    let before_window =
        baseline.windows.iter().find(|window| window.kind == WindowKind::Baseline28d);
    let after_window = post.windows.iter().find(|window| window.kind == WindowKind::Post28d);
    if !matching_phase_windows(before_window, after_window) {
        gates.push("phase_window_mismatch".to_owned());
    }
    if baseline.contract_version != post.contract_version
        || baseline.collector.metric_registry_version != post.collector.metric_registry_version
        || baseline.collector.adapter_registry_version != post.collector.adapter_registry_version
        || baseline.consent.content_analysis != post.consent.content_analysis
        || baseline.consent.approved_adapters != post.consent.approved_adapters
        || baseline.consent.approved_field_classes != post.consent.approved_field_classes
    {
        gates.push("contract_or_consent_changed".to_owned());
    }
    let before_coverage = baseline
        .coverage
        .iter()
        .filter(|coverage| baseline.consent.approved_adapters.contains(&coverage.adapter_id))
        .map(|coverage| (coverage.adapter_id.as_str(), &coverage.status))
        .collect::<BTreeMap<_, _>>();
    let after_coverage = post
        .coverage
        .iter()
        .filter(|coverage| post.consent.approved_adapters.contains(&coverage.adapter_id))
        .map(|coverage| (coverage.adapter_id.as_str(), &coverage.status))
        .collect::<BTreeMap<_, _>>();
    if before_coverage != after_coverage
        || before_coverage.values().any(|status| **status != CoverageStatus::Observed)
    {
        gates.push("adapter_coverage_mismatch".to_owned());
    }
    if !gates.is_empty() {
        return Comparability {
            disposition: ComparabilityDisposition::Incomparable,
            blocking_mismatches: gates,
        };
    }

    let baseline_metrics = baseline
        .metrics
        .iter()
        .map(|metric| ((metric.metric_id.as_str(), metric.adapter_id.as_str()), metric))
        .collect::<BTreeMap<_, _>>();
    let post_metrics = post
        .metrics
        .iter()
        .map(|metric| ((metric.metric_id.as_str(), metric.adapter_id.as_str()), metric))
        .collect::<BTreeMap<_, _>>();
    let keys = baseline_metrics.keys().chain(post_metrics.keys()).copied().collect::<BTreeSet<_>>();
    let mut comparable = 0_u64;
    let mut mismatches = Vec::new();
    for key in keys {
        match (baseline_metrics.get(&key), post_metrics.get(&key)) {
            (Some(before), Some(after))
                if before.unit == after.unit
                    && before.evidence_class == after.evidence_class
                    && before.source_definition_version == after.source_definition_version
                    && before.value.is_some()
                    && after.value.is_some()
                    && before.missing_count == 0
                    && after.missing_count == 0 =>
            {
                comparable += 1;
            }
            _ => mismatches.push(format!("metric_unmatched:{}:{}", key.0, key.1)),
        }
    }
    let disposition = match (comparable, mismatches.is_empty()) {
        (0, _) => ComparabilityDisposition::Incomparable,
        (_, true) => ComparabilityDisposition::ComparableDescriptive,
        _ => ComparabilityDisposition::Partial,
    };
    Comparability { disposition, blocking_mismatches: mismatches }
}

fn matching_phase_windows(
    baseline: Option<&observer_domain::CollectionWindow>,
    post: Option<&observer_domain::CollectionWindow>,
) -> bool {
    let (Some(before), Some(after)) = (baseline, post) else {
        return false;
    };
    let (Some(before_start), Some(after_start)) = (before.start, after.start) else {
        return false;
    };
    before.timezone == after.timezone
        && before.end.signed_duration_since(before_start)
            == after.end.signed_duration_since(after_start)
}

pub fn atomic_write_owner_only(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let parent = path.parent().ok_or_else(|| {
        CoreError::Domain(DomainError::InvalidContract(
            "output path has no parent directory".to_owned(),
        ))
    })?;
    fs::create_dir_all(parent)?;
    set_owner_only_directory(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("export"),
        random_scoped_id()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    set_owner_only_file_options(&mut options);
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_owner_only_file_options(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_owner_only_directory(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_owner_only_directory(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_never_converts_missing_to_zero() {
        let mut registry = adapter_registry().expect("registry");
        let adapter = RegistryPlaceholderAdapter::new(registry.adapters.remove(0));
        let result = adapter.collect(true);
        assert_eq!(result.coverage.status, CoverageStatus::Missing);
        assert_eq!(result.coverage.observed_records, 0);
    }

    #[test]
    fn embedded_registry_is_contract_versioned() {
        assert_eq!(
            adapter_registry().expect("registry").registry_version,
            ADAPTER_REGISTRY_VERSION
        );
    }
}
