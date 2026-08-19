# Participant workflow

## Before collection

1. Download a signed release asset for your platform and verify its SHA-256 checksum and Sigstore bundle.
2. Obtain a study code from the study coordinator if you intend to upload.
3. Choose one supported agent prompt. The prompt operates the deterministic collector; it does not inspect your history itself.

## Local collection

Run one phase at a time. Choose `baseline` for the first window or `post` for the later matched
window. Each consent manifest authorizes exactly one phase.

`--baseline N` sets the window length in days (1-365, default 28). It is a comparability unit, not a
fixed constant: **baseline and post must use the same N**. `compare` derives both lengths from the
recorded window bounds and returns `phase_window_mismatch` when they differ, so an unequal pair is
refused rather than compared. A shorter N gives a faster round trip; a longer N gives a larger window,
bounded by what each tool still retains.

Run `discover` first and approve only the adapters it reports as `observed`. Comparison requires the
same adapter coverage in both phases with every approved adapter observed, so approving a tool you do
not use makes every later comparison `INCOMPARABLE` on `adapter_coverage_mismatch`.

```text
second-observer discover --home <participant-home>
second-observer consent init --phase <baseline|post> --adapter <id> [--adapter <id> ...]
second-observer collect --profile retained-history --baseline <days> --phase <baseline|post> --home <participant-home> --timezone <iana-timezone>
second-observer preview
second-observer export
second-observer verify <export>
```

`discover` reports known tool availability. It does not execute tools and does not establish that a tool has usable retained history.

Review the consent manifest before `collect`. Do not continue if the approved adapters, source classes, field classes, or local analyzers differ from your intent.

Add `--content-analysis` to `consent init` only to enable local relay, routing, and correction heuristics. The collector retains no message or command text and exports only aggregate values. Keep the generated `.second-observer/study-identity.json` between baseline and post collections, and run both phases with the same `--baseline` value and the same approved adapters; it stores random participant/device IDs owner-only while each collection receives a new run ID.

`preview` is the review boundary. It shows the exact aggregate payload and excluded-field assertions. Stop if any content is unexpected.

`verify` checks the local archive before it leaves your machine.

## Optional upload

Upload only after you have reviewed the preview and explicitly decide to send it:

```text
second-observer-upload send <export> --study-code <code> --confirm
```

The upload client submits exactly one finalized export. It cannot inspect local telemetry sources. Save the returned receipt and use it for status or revocation:

```text
second-observer-upload status <receipt> --study-code <code>
second-observer-upload revoke <receipt> --study-code <code> --confirm
```

Do not provide a study code to an agent unless you have already approved the specific upload command.

After collecting and exporting both phase-specific archives, compare them locally:

```text
second-observer compare <baseline-export> <post-export>
```

## Interpretation

The report compares descriptive measures only. It displays coverage limitations and preserves missing values. It does not establish productivity, correctness, causation, or cost savings.
