# Measurement contract v1

Status: accepted for Second Observer v1 implementation on 2026-08-18.

## Claim boundary

Second Observer describes activity retained by approved local sources during declared windows.
It does not establish complete history, productivity, correctness, time saved, cost savings,
population behavior, audit-grade completeness, or a causal Second effect.

Allowed comparison language:

> During two declared windows on this device, collector version X observed metric M change from A to
> B across the supported sources that passed the recorded comparability gates.

## Outputs

Every collection produces two independent profiles:

1. `retained_history`: all records still available to each approved adapter, with earliest and latest
   observed timestamps and explicit coverage limits.
2. `baseline_28d`: the 28 complete local calendar days ending before collection day.

Retained history never enters a matched-window comparison. A later Second-use collection compares its
28-day profile only with a compatible 28-day baseline.

## Evidence classes

| Class | Meaning |
|---|---|
| `observed_counter` | Explicit counter or event recorded by the source. |
| `deterministic_derived` | Deterministic reduction over observed timestamps or counters. |
| `local_content_heuristic` | Consent-gated deterministic feature derived locally from content. |
| `estimated` | Labeled estimate with a named basis; never a receipt. |
| `unknown` | Source cannot supply the value. |
| `not_applicable` | The metric has no meaning for that source. |

## Coverage

Each adapter reports exactly one state: `observed`, `detected_unmeasured`, `unsupported_version`,
`permission_denied`, `missing`, or `disabled`.

Missing, unsupported, disabled, and permission-denied values remain typed missing values. Reducers may
not convert them to zero. Installed software, retained source data, and measured activity remain
separate facts.

## Metrics

The normative metric registry is `registry/metrics-v1.json`. Each emitted value names its metric,
adapter, window, evidence class, unit, value, eligible count, observed count, missing count, and
`source_definition_version`. The definition version identifies the source-specific parsing and
reduction semantics independently of the collector release.

Provider-specific token accounting remains separated by adapter. The collector does not emit a
combined Claude-plus-Codex token total. Cost estimates identify their pricing basis and never present
as billing receipts.

Second-native metrics may explain the post condition but do not enter a pre/post delta when the
baseline source lacks the same definition.

## Comparability

The reducer emits one disposition:

- `COMPARABLE_DESCRIPTIVE`: window length, metric definition, adapter definition, approved fields,
  source coverage, and collection configuration match.
- `PARTIAL`: some metrics match, while named metrics fail a gate.
- `INCOMPARABLE`: no declared metric set passes the exact-match gates.
- `COLLECTION_FAILED`: the collection did not produce a valid export.

A changed adapter version does not automatically invalidate a comparison. The adapter registry must
declare whether the change preserves its measurement definition. Permission, source, consent, metric,
or window changes invalidate every affected metric.

Relative change is `null` when the baseline value or denominator is zero. Reports preserve failed,
abandoned, and zero-output eligible observations in denominators when the source records them.

## Integrity

Exports use `second-observer.study-export/v1`. Object keys serialize in lexicographic order without
insignificant whitespace. Finite integral metric values serialize as JSON integers; fractional values
use the serializer's shortest finite decimal representation.
`integrity.payload_sha256` is SHA-256 over canonical JSON after replacing that field with 64 lowercase
zeroes. The SHA-256 sidecar and upload negotiation digest cover the exact finalized canonical bytes.
Verification checks both digests, validates the schema, checks consent and forbidden-field assertions,
and rejects any modified payload.

Generated timestamps do not participate in fixture byte-replay tests unless the clock is pinned.
