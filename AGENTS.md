# Second Observer — agent instructions

Canonical leaf instructions for Claude Code and Codex. `CLAUDE.md` symlinks here.

## Scope

Second Observer is a public, deterministic workflow-measurement instrument. It discovers supported
local development tools, aggregates approved local activity, produces a participant-reviewable
export, and uploads only through a separate executable.

The repository contains public source, schemas, adapter definitions, sanitized fixtures, participant
instructions, release automation, and tests. It never contains participant reports or private Second
application state.

## Workflow authority

Inside the RIG Platform checkout, the parent workflow contract governs this leaf. Standalone public
clones use this file and the program map below. Routine integrated work creates no workflow artifact.
Assurance, compute, and action authority remain independent.

Program sequencing lives in `docs/plans/SECOND_OBSERVER_PROGRAM_MAP.md`. Executors never edit it.

## Product invariants

1. Build the collector and uploader in Rust.
2. Keep collection deterministic. Do not call an LLM from product code.
3. Keep collection and upload in separate binaries. The collector has no HTTP dependency.
4. Never execute a detected tool. Detect through allowlisted filesystem and process metadata only.
5. Never recursively scan a home directory or workspace.
6. Require an explicit consent manifest before content-bearing reads.
7. Aggregate content-derived signals in memory. Never persist or export raw prompts, commands,
   transcripts, tool output, paths, repository names, remotes, URLs, or stable machine identifiers.
8. Treat missing, unsupported, permission-denied, and disabled measurements as distinct states.
9. Preserve source-specific accounting. Never combine incompatible provider token totals.
10. Report descriptive observations only. Do not emit productivity, correctness, causal-effect, or
    cost-saving verdicts.

## Repository boundaries

- `second-observer` owns local discovery, consent, aggregation, export, verification, and upload.
- The private Second repository owns Cloudflare intake, hosted study reports, and case-study records;
  those implementations and records do not belong in this public repository.
- Raw participant inputs remain on the participant machine.
- Uploaded aggregate archives remain private Second study data and never enter this repository.

## Rust rules

- Centralize dependency versions and features in the workspace manifest.
- Forbid unsafe Rust.
- Keep schema and metric semantics independent of operating-system and vendor adapters.
- Isolate filesystem adapters from aggregation and export policy.
- Keep uploader dependencies out of the collector dependency graph.
- Run formatting, Clippy with warnings denied, and the full workspace test suite.

## Evidence discipline

- Mark factual documentation claims `[verified]`, `[inferred]`, or `[assumed]`.
- Use sanitized synthetic fixtures. Do not copy local histories into tests.
- A detected application is not measured activity.
- Retained history is not complete history unless the adapter proves retention coverage.
- A passing schema or replay test proves only that named apparatus property.
