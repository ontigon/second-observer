# Privacy and threat model v1

Status: accepted for Second Observer v1 implementation on 2026-08-18.

## Trust boundary

The participant controls collection and export. Installation is not consent. Agent execution is not
consent. Collection consent and upload approval are separate events.

`second-observer` reads approved local sources and has no network client. `second-observer-upload`
reads one finalized export and has no adapter dependency or source-discovery capability.

## Discovery boundary

Discovery checks only versioned OS-specific locations from `registry/adapters-v1.json`. It may inspect
existence, file type, size, permissions, and modification time. It never executes detected software,
loads shell initialization, enumerates environment values, reads process arguments, or recursively
walks a home directory or workspace.

An adapter may traverse only its named application-owned history directory with a declared depth,
file-pattern, and byte limit after consent.

## Consent

Before the first content-bearing read, a human reviews a consent manifest naming:

- approved adapters and source classes;
- approved metadata and content field classes;
- prohibited field classes;
- content-analyzer opt-in;
- collection windows;
- configuration expiry.

Metadata collection is the default. Content analysis remains disabled unless the manifest explicitly
sets `content_analysis` to `true`.

## Forbidden export fields

Exports, logs, exceptions, temporary files, fixtures, and receipts must not contain raw prompts,
commands, transcripts, assistant output, tool input/output, file bodies, paths, filenames, project or
repository names, branch names, remotes, URLs, clipboard data, environment values, credentials,
provider/session/thread identifiers, telemetry machine identifiers, usernames, emails, or stable
hardware identifiers.

Study and device identifiers are random and scoped to one study. They are not derived from local
machine or provider identifiers.

## Content-derived features

When approved, deterministic analyzers may read message or command text in memory to calculate only
registered scalar features such as byte counts, code-fence share, routing-pattern counts, correction
counts, and bounded overlap ratios. Raw content and content fingerprints are discarded before export.
The collector never sends local content to an LLM.

## Filesystem and parser threats

The collector rejects or safely classifies symlinks escaping an approved root, hard-link surprises,
FIFOs, sockets, device files, oversized records, excessive nesting, malformed UTF-8, malformed JSON,
malformed SQLite, control characters, and ANSI escape sequences.

The collector operates as the current user and refuses elevated/root execution. Outputs use owner-only
permissions where the OS supports them, atomic rename, and cleanup of temporary files.

## Network and supply-chain threats

The collector binary contains no HTTP dependency, updater, crash reporter, analytics client, DNS
lookup, or remote schema fetch. CI checks the collector dependency graph and rejects network-client
and socket identifiers in collector source. It does not claim runtime syscall tracing.

Releases use locked dependencies, checksums, SBOMs, source revision metadata, and GitHub OIDC-backed
Sigstore bundles. The package does not load adapter code or metric definitions from the network.

## Upload boundary

The uploader transmits exactly the previewed finalized export. It requires an explicit command, study
code, local digest verification, and participant confirmation. It returns a receipt and supports
status and revocation.

The private service stores aggregate exports only. Validation failure quarantines or deletes the
object according to the private retention policy and never promotes partial values into a report.
