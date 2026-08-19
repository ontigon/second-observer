# Second Observer

Second Observer is a public, deterministic instrument for describing retained local development-tool activity across a frozen baseline and later matched window.

It produces aggregate study exports. It does not make productivity, correctness, causal-effect, or cost-saving claims.

## Status

[verified] This repository currently contains the public participant, privacy, security, and release surface.

[verified] Release `v0.1.6` provides the `second-observer` collector and isolated `second-observer-upload` uploader for macOS arm64/x86-64, Linux arm64/x86-64, and Windows x86-64. Published checksums, Sigstore identities, archive contents, SBOM graphs, sanitized dependency records, and signed manifest bindings pass independent download verification.

## Participant workflow

```text
second-observer run
```

One interactive command. It asks for the home directory, timezone, adapters, phase, window
length, and content-analysis choice, shows the consent manifest before collecting and the entire
payload before exporting, and writes nothing until you approve each. No agent required.

Running it again reuses those answers, so a repeat collection is one keypress — or none, with
`second-observer run --repeat`. Every run is a separate snapshot; nothing is overwritten. List
them with `second-observer snapshots` and choose between them when you report.

Every step is also available as a separate non-interactive command that infers nothing:

```text
second-observer discover --home <participant-home>
second-observer consent init --phase <baseline|post> --adapter <id> [--adapter <id> ...]
second-observer collect --profile retained-history --baseline <days> --phase <baseline|post> --home <participant-home> --timezone <iana-timezone>
second-observer preview
second-observer export
second-observer verify <export>
second-observer-upload send <export> --study-code <code> --confirm
```

The collector does not upload. `second-observer-upload send` requires a separate explicit approval after you review the exact payload shown by `preview`.

Use `consent init --content-analysis` only for locally aggregated relay, routing, and correction heuristics. It adds text field classes to the consent manifest but never exports text. Preserve `.second-observer/study-identity.json` for the matched post collection.

Run a later matched window with a new consent manifest and `--phase post`, then use
`second-observer compare <baseline-export> <post-export>`. A consent manifest authorizes exactly one
phase. `--baseline <days>` sets the window length (1-365, default 28); baseline and post must use the
same value and the same approved adapters, or the pair is refused as `INCOMPARABLE`.

Read [the participant workflow](docs/participant/WORKFLOW.md), [privacy contract](PRIVACY.md), and [security policy](SECURITY.md) before collecting.

## Boundaries

- `second-observer` discovers, collects, previews, exports, and verifies locally. It has no HTTP client.
- `second-observer-upload` uploads one finalized export. It cannot inspect local telemetry sources.
- The instrument reads only consented, adapter-allowlisted local sources. It never executes a detected application.
- Exports exclude raw prompts, commands, transcripts, tool output, paths, titles, repository names, remotes, URLs, and stable machine identifiers.
- An observed tool is not necessarily a measured tool. Missing, unsupported, permission-denied, and disabled remain distinct states.

## Agent-assisted use

Use one prompt for your agent client:

- [Claude Code](agent-prompts/claude-code.md)
- [Codex](agent-prompts/codex.md)
- [Cursor](agent-prompts/cursor.md)
- [Zed](agent-prompts/zed.md)

Each prompt runs the same deterministic workflow. The agent must not inspect histories itself, infer consent, broaden scope, or upload without explicit approval.

## Release verification

Two checks, and they are not equally available. Do the first one always. Do the second one if
you have `cosign`; its absence is not a reason to stop.

**Required — checksum.** Proves the archive was not altered in transit or storage. `shasum`
ships with macOS and Linux; Windows has `Get-FileHash`.

```text
shasum -a 256 -c SHA256SUMS --ignore-missing
```

**Recommended — signature.** Proves the archive was built by this repository's release workflow
and not merely by someone who could also edit `SHA256SUMS`. Requires
[cosign](https://github.com/sigstore/cosign) (`brew install cosign`).

```text
cosign verify-blob --bundle <asset>.sigstore.json --certificate-identity-regexp 'https://github.com/ontigon/second-observer/.github/workflows/release.yml@refs/tags/.*' --certificate-oidc-issuer https://token.actions.githubusercontent.com <asset>
cosign verify-blob --bundle RELEASE-MANIFEST.sigstore.json --certificate-identity-regexp 'https://github.com/ontigon/second-observer/.github/workflows/release.yml@refs/tags/.*' --certificate-oidc-issuer https://token.actions.githubusercontent.com RELEASE-MANIFEST.json
```

If you run only the checksum, you have verified integrity but not provenance. That is a weaker
guarantee, stated here so the choice is explicit rather than accidental.

On Windows PowerShell, verify the selected archive against `SHA256SUMS` before running the same
`cosign verify-blob` commands:

```powershell
$archive = "second-observer-<version>-x86_64-pc-windows-msvc.zip"
$expected = ((Get-Content SHA256SUMS | Where-Object { $_ -match ([regex]::Escape($archive) + '$') }) -split '\s+')[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "SHA-256 mismatch for $archive" }
```

See [the release policy](docs/release/RELEASE_POLICY.md) for release contents, verification constraints, and SBOM requirements.

## License

Dual-licensed under [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
