---
name: "Second Observer"
type: program-map
status: active
generated: 2026-08-18
turn_prefix: "SO"
scope: "Design, implement, validate, and prepare public release of the deterministic Second Observer collector and isolated uploader in apps/second-observer. Private intake, participant data, hosted reports, and case-study authority remain in apps/second."
---

# Second Observer

## Program goal markers

<!-- Define the terminal markers and the evidence that proves each marker. -->

| Marker | End condition | Test |
|---|---|---|
| `G1` | The public measurement and privacy contract is frozen. | Versioned schemas define consent, coverage, evidence classes, metrics, missingness, comparability, forbidden fields, integrity, and nonclaims. |
| `G2` | The local collector is deterministic and network-isolated. | Cross-platform fixtures pass; filesystem reads match the approved adapter plan; collector dependency and runtime checks find no network path. |
| `G3` | Supported adapters produce safe retained-history and matched-window aggregates. | Claude Code, Codex, Cursor, Git/worktree, shell, and Second fixtures pass; Zed, VS Code/Copilot, and Warp report measured or explicit detected-unmeasured states. |
| `G4` | Export and upload have an inspectable consent boundary. | Preview is byte-equivalent to the export payload; the uploader reads only a finalized export; digest, tamper, revocation, and receipt tests pass. |
| `G5` | A public participant workflow is release-ready. | Signed release artifacts, SBOM, checksums, agent prompts, threat model, and an Aaron calibration receipt pass without participant data entering Git. |

## Operating model

<!-- Name the director, executor boundary, single-writer rule, and turn-close authority. -->

- Director: one Second Observer director owns this map, accepts the measurement contract, selects
  implementation slots, adjudicates returns, and alone closes or supersedes slots.
- Executor: bounded Rust, adapter, privacy, release, or verification roles implement declared slots,
  return facts, and never edit this map.
- Parent: the Second IDE Platform Master records only the accepted SIPM-03 discovery-instrument
  disposition and integration dependency. It does not duplicate this map's detailed state.
- External service: `apps/second` owns Cloudflare intake, private study data, hosted participant
  reports, and case-study publication decisions.

## Session bootstrap

<!-- List the read order a fresh director follows before acting. -->

1. This map: status board and decision register first.
2. `AGENTS.md` and `docs/design/MEASUREMENT_CONTRACT.md`.
3. `docs/design/PRIVACY_AND_THREAT_MODEL.md`.
4. The latest slot checkpoint or verification report named by the status board.

## Turn slots

<!-- Give every planned turn one heading, objective, dependencies, saved prompt path, and acceptance gate. -->

### SO-00 — Freeze measurement, privacy, and repository contracts

- Objective: translate the accepted implementation plan into versioned schemas, metric definitions,
  adapter rules, privacy boundaries, and explicit nonclaims before collector code begins.
- Depends: none.
- Prompt: integrated director work from the user-accepted plan; no executor prompt.
- Acceptance: G1 passes; public/private ownership is explicit; no schema permits raw content or stable
  machine identity; the director records the contract accepted.

### SO-01 — Build the Rust collector kernel and CLI

- Objective: implement discovery, consent, collection, preview, export, comparison, and verification
  with deterministic schemas and no network dependency.
- Depends: SO-00.
- Prompt: integrated implementation under the frozen contract.
- Acceptance: formatting, Clippy, unit, property, deterministic replay, permissions, and collector
  network-boundary tests pass on the development host.

### SO-02 — Implement and validate source adapters

- Objective: implement fixed-location adapters and coverage reporting for named coding agents, IDEs,
  shells, Git/worktrees, and Second without executing detected tools or recursively scanning.
- Depends: SO-00 and SO-01 kernel interfaces.
- Prompt: integrated implementation under the adapter registry.
- Acceptance: every adapter passes positive, missing, malformed, permission-denied, and unsupported
  fixtures or remains explicitly `detected_unmeasured`; missing values never become zero.

### SO-03 — Prove privacy, content, and integrity boundaries

- Objective: implement consent-gated local content features, forbidden-field checks, canary fixtures,
  deterministic archives, digests, and tamper verification.
- Depends: SO-01 and SO-02.
- Prompt: integrated implementation plus independent adversarial verification.
- Acceptance: canary, path, identifier, content, symlink, malformed-input, permissions, and byte-replay
  tests pass; exported artifacts contain only approved aggregate fields.

### SO-04 — Build the isolated uploader and intake protocol client

- Objective: implement a separate network-capable binary that reads only finalized exports and speaks
  the versioned Second intake protocol.
- Depends: SO-00 and the SO-01 export format.
- Prompt: integrated implementation under the intake API contract.
- Acceptance: dependency analysis excludes adapters from the uploader; mocked negotiate, direct PUT,
  finalize, status, and revoke flows pass; digest mismatch fails closed.

### SO-05 — Prepare public participant and release surfaces

- Objective: create cross-client agent prompts, bootstrap/install guidance, license, security policy,
  release matrix, checksums, SBOM, and source-build instructions.
- Depends: SO-01 through SO-04.
- Prompt: integrated documentation and release engineering.
- Acceptance: Claude Code, Codex, Cursor, and Zed prompts invoke one identical deterministic workflow;
  a participant without Rust can use a pinned binary; no prompt grants consent or opens raw history.

### SO-06 — Calibrate with Aaron and admit the Doug pilot

- Objective: run the collector against Aaron's retained local sources, reconcile only declared metrics
  against legacy reports, and freeze a pilot-ready release and study workflow.
- Depends: SO-03, SO-04, SO-05, and a validated private intake in `apps/second`.
- Prompt: to-write after prior slots close.
- Acceptance: Aaron's preview and receipt pass; legacy differences are explained; no raw input enters
  Git or Slack; the director explicitly admits or rejects the Doug pilot.

## Dependency order

<!-- Encode execution order without calendar promises. -->

```text
SO-00 -> SO-01
SO-01 -> SO-02 -> SO-03
SO-01 -> SO-04
SO-03 + SO-04 -> SO-05
SO-05 + private intake validation -> SO-06
```

## Decision register

<!-- Record decisions, owner, ruling, status, and the date of each ruling. -->

| ID | Decision | Owner | Status |
|---|---|---|---|
| `D-SO-01` | Create a separate public repository at `apps/second-observer`; private intake and study data remain in `apps/second`. | Aaron | accepted 2026-08-18 |
| `D-SO-02` | Build one Rust collector with no HTTP dependency and one separate Rust uploader with network capability. | Aaron | accepted 2026-08-18 |
| `D-SO-03` | Support macOS, Linux, and Windows through signed prebuilt binaries; Rust is not a participant prerequisite. | Aaron | accepted 2026-08-18 |
| `D-SO-04` | Collection is metadata-first; content-derived analysis requires explicit opt-in and exports no raw content. | Aaron | accepted 2026-08-18 |
| `D-SO-05` | Every collection emits a retained-history profile and a frozen 28-complete-day baseline. | Aaron | accepted 2026-08-18 |
| `D-SO-06` | Upload is a separate explicit command; participants see the exact local payload and receive a private hosted result. | Aaron | accepted 2026-08-18 |
| `D-SO-07` | Pre/post results are descriptive matched-window observations, not causal Second-effect or productivity verdicts. | Aaron | accepted 2026-08-18 |
| `D-SO-08` | Use Cloudflare Worker, private R2, Queue, and D1 for intake; Containers are excluded until a measured runtime need appears. | Aaron | accepted 2026-08-18 |
| `D-SO-09` | Implement and publish the named public `ontigon/second-observer` repository, deploy the bounded private intake at `intake.second.ontigon.ai`, and send Doug one pilot enrollment after the signed release and live protocol pass. This authority excludes participant data publication and Second application-runtime implementation. | Aaron | accepted 2026-08-18 |

## Kill / stop conditions

<!-- Pair each falsifier or stop condition with its mandatory response. -->

| Condition | Response |
|---|---|
| Collector code gains an HTTP, socket, update, crash-report, or telemetry dependency. | Stop SO-01/SO-03, remove the dependency, and rerun the network-boundary audit. |
| A detector executes an installed application or recursively scans a home/workspace tree. | Stop the adapter, mark it unsupported, and replace it with allowlisted metadata inspection. |
| Raw content, paths, identifiers, repository names, remotes, commands, prompts, or URLs enter an export, log, exception, or fixture. | Stop release work, remove the material, rotate affected study credentials, and rerun every canary test. |
| Missing or unsupported data becomes zero, or incomparable provider counters are summed. | Reject the report and repair the metric/comparability reducer. |
| A report emits productivity, correctness, cost-saving, population, audit-grade, or causal-effect language. | Reject the report and restore the descriptive nonclaim boundary. |
| Participant data enters Git, a public release, or an agent prompt. | Stop, remove the material from the working tree/index, assess history exposure, and report the custody breach. |
| The uploader can import an adapter or read outside the finalized export path. | Stop SO-04 and restore the binary/dependency boundary. |
| Another director holds this map's writer role. | Do not edit or dispatch; reconcile ownership first. |

## Non-goals

<!-- Bound work that this program does not own. -->

- Background surveillance, continuous telemetry, process spying, arbitrary extension/plugin scanning,
  and automatic upload.
- Raw transcript, prompt, command, code, tool-output, clipboard, environment, credential, or repository
  collection.
- A composite Second score, public leaderboard, adversarial-tamper claim, or causal product claim.
- Private Cloudflare implementation, participant study data, hosted-result authorization, or case-study
  publication; those belong to `apps/second`.
- Second native application runtime, terminal, session, program-graph, or context implementation.

## Status board

<!-- Record actual dated state; fresh sessions trust this table over memory. -->

| Item | State | Record |
|---|---|---|
| SO-00 | closed 2026-08-18 | contract and schemas accepted; `docs/design/MEASUREMENT_CONTRACT.md`; `docs/design/PRIVACY_AND_THREAT_MODEL.md`; `schemas/`; `registry/` |
| SO-01 | closed 2026-08-18 | Rust kernel, phase identity, deterministic export, comparison, consent, bounds, and local verification pass `scripts/check.sh` |
| SO-02 | closed 2026-08-18 | allowlisted Claude/Codex/Git/shell adapters pass bounded fixtures; Cursor, Zed, VS Code/Copilot, Warp, and Second preserve explicit detection or missingness rather than fabricate measures |
| SO-03 | closed 2026-08-18 | registry, consent, canonical-byte, tamper, content-coupling, forbidden-field, link, size, and JSON-safe-count gates pass; public/private fixture SHA-256 is `aedbe6642ee82102e63aec7373e5f324628f15b0c384fc7ff95c79b712aeee05` |
| SO-04 | closed 2026-08-18 | isolated uploader and live Cloudflare negotiate/upload/finalize/queue/result/revoke flow passed; receipt `5b27d1b9-a11a-4404-81aa-9f77ee084abf` ended `revoked` with zero derived rows |
| SO-05 | closed 2026-08-19 | `v0.1.2` published 28 assets for five targets; aggregate checksums, six Sigstore identities, archive contents, signed-manifest bindings, non-empty SBOM graphs, and path-free dependency records passed independent download verification; defective `v0.1.0` remains immutable and marked withdrawn |
| SO-06 | blocked 2026-08-19 | release and private-intake gates are closed; Aaron collection requires explicit preview/consent, and Doug has not returned a baseline receipt |

## Turn log

<!-- Append closed turns as: turn-id | verdict | one-line result | record path. -->

```text
SO-00 | closed/director-integrated | Measurement, privacy, schema, adapter, and metric contracts frozen before collector implementation | docs/design/MEASUREMENT_CONTRACT.md
SO-01..SO-04 | closed/director-integrated | Collector, adapters, privacy boundary, isolated uploader, and live private intake protocol passed their declared local and live gates | scripts/check.sh
SO-05 | closed/director-integrated | v0.1.2 cross-platform release and independent public-asset verification passed; v0.1.0 preserved and withdrawn after checksum/SBOM red-team failures | https://github.com/ontigon/second-observer/releases/tag/v0.1.2
```
