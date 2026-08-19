use chrono::{Duration, TimeZone, Utc};
use observer_core::{
    atomic_write_owner_only, collect, compare, random_study, Adapter, RegistryPlaceholderAdapter,
};
use observer_domain::{
    ComparabilityDisposition, ConsentManifest, CoverageStatus, EvidenceClass, MetricValue,
};

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 18, 15, 30, 0).single().expect("valid timestamp")
}

fn fixed_consent(now: chrono::DateTime<Utc>) -> ConsentManifest {
    ConsentManifest::metadata_first(
        now - Duration::days(1),
        now + Duration::days(30),
        vec!["codex".to_owned()],
    )
}

fn observed_metric(id: &str) -> MetricValue {
    MetricValue {
        metric_id: id.to_owned(),
        adapter_id: "codex".to_owned(),
        window_id: "baseline-28d".to_owned(),
        source_definition_version: "adapter-v1".to_owned(),
        evidence_class: EvidenceClass::ObservedCounter,
        unit: "turns".to_owned(),
        value: Some(1.0),
        eligible_count: 1,
        observed_count: 1,
        missing_count: 0,
    }
}

#[test]
fn missing_status_is_not_an_observed_zero() {
    let adapter = RegistryPlaceholderAdapter::new(observer_core::adapter_registry().expect("registry").adapters[0].clone());
    let result = adapter.collect(true);
    assert_eq!(result.coverage.status, CoverageStatus::Missing);
    assert_eq!(result.coverage.observed_records, 0);
    let missing_metric = MetricValue {
        metric_id: "human_turns".to_owned(),
        adapter_id: "codex".to_owned(),
        window_id: "baseline-28d".to_owned(),
        source_definition_version: "adapter-v1".to_owned(),
        evidence_class: EvidenceClass::Unknown,
        unit: "turns".to_owned(),
        value: None,
        eligible_count: 1,
        observed_count: 0,
        missing_count: 1,
    };
    assert_ne!(missing_metric.value, Some(0.0));
}

#[test]
fn pinned_clock_export_is_byte_deterministic() {
    let now = fixed_time();
    let consent = fixed_consent(now);
    let study = random_study();
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let first = collect(&consent, now, "UTC", "test", digest, study.clone()).expect("collect");
    let second = collect(&consent, now, "UTC", "test", digest, study).expect("collect");
    assert_eq!(first.canonical_bytes().expect("bytes"), second.canonical_bytes().expect("bytes"));
}

#[test]
fn tampered_canonical_export_fails_verification() {
    let now = fixed_time();
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let export = collect(&fixed_consent(now), now, "UTC", "test", digest, random_study()).expect("collect");
    let bytes = export.canonical_bytes().expect("bytes");
    let tampered = String::from_utf8(bytes)
        .expect("utf8")
        .replace("not productivity", "not productivitx");
    assert!(observer_domain::parse_and_verify_export(tampered.as_bytes()).is_err());
}

#[test]
fn comparison_returns_every_contract_disposition() {
    let now = fixed_time();
    let consent = fixed_consent(now);
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut baseline = collect(&consent, now, "UTC", "test", digest, random_study()).expect("collect");
    let mut post = collect(&consent, now, "UTC", "test", digest, random_study()).expect("collect");
    assert_eq!(compare(&baseline, &post).disposition, ComparabilityDisposition::Incomparable);

    post.study.participant_id = baseline.study.participant_id.clone();
    post.study.device_id = baseline.study.device_id.clone();
    for coverage in baseline.coverage.iter_mut().chain(post.coverage.iter_mut()) {
        coverage.status = CoverageStatus::Observed;
    }
    let phase = post.windows.iter_mut().find(|window| window.id == "baseline-28d").expect("phase window");
    phase.id = "post-28d".to_owned();
    phase.kind = observer_domain::WindowKind::Post28d;

    baseline.metrics.push(observed_metric("human_turns"));
    post.metrics.push(observed_metric("human_turns"));
    assert_eq!(compare(&baseline, &post).disposition, ComparabilityDisposition::ComparableDescriptive);

    post.metrics.push(observed_metric("agent_turns"));
    assert_eq!(compare(&baseline, &post).disposition, ComparabilityDisposition::Partial);

    post.comparability.disposition = ComparabilityDisposition::CollectionFailed;
    assert_eq!(compare(&baseline, &post).disposition, ComparabilityDisposition::CollectionFailed);
}

#[cfg(unix)]
#[test]
fn atomic_output_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir().expect("tempdir");
    let output = directory.path().join("export.study-export");
    atomic_write_owner_only(&output, b"{}").expect("write");
    assert_eq!(std::fs::metadata(output).expect("metadata").permissions().mode() & 0o077, 0);
}
