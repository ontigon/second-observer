# Security policy

## Supported releases

[assumed] Supported releases will be the latest published signed release and the immediately preceding release.

Do not treat a branch checkout, an unsigned artifact, or a locally rebuilt binary as a verified participant release.

## Reporting a vulnerability

Do not file public issues for vulnerabilities that could expose participant data, bypass consent, read disallowed local sources, or upload an unreviewed export.

Report privately to the maintainers through the security contact published with each release. Include:

- affected version and operating system;
- a minimal synthetic reproduction;
- expected and observed behavior;
- whether any local content, path, identifier, or export data could be disclosed.

Do not include participant histories, study exports, secrets, tokens, or personal data in the report.

## Security properties

- The collector must make no network or DNS requests.
- The uploader must not read telemetry source locations.
- The uploader must accept HTTPS endpoints only, reject redirects, and read only one bounded regular
  finalized export; test-only loopback HTTP is not a participant transport.
- Discovery must not execute detected tools.
- Content-bearing reads require a separate consent grant.
- The participant must review the exact outgoing payload before an upload command can proceed.
- Release binaries must be checksummed and signed with GitHub Actions OIDC-backed Sigstore bundles.

## Scope

In scope: consent bypasses, disallowed file access, source execution, raw-data persistence or export, collector networking, uploader source access, archive integrity failures, and release-artifact tampering.

Out of scope: unsupported adapter formats, requests to reconstruct deleted local data, and reports containing real participant data.
