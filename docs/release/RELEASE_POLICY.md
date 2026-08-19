# Release, checksum, and SBOM policy

## Published artifacts

Each release publishes archives for macOS arm64/x86-64, Linux arm64/x86-64, and Windows x86-64. Every platform archive contains both binaries:

- `second-observer`, the network-isolated collector;
- `second-observer-upload`, the isolated uploader.

Each release also publishes:

- `SHA256SUMS`, covering every release archive;
- one Sigstore bundle per archive;
- a CycloneDX SBOM for each archive;
- one sanitized Cargo dependency record for each target;
- a signed `RELEASE-MANIFEST.json`, its Sigstore bundle, source commit, and release notes.

## Signing and verification

The release workflow signs archive digests with GitHub Actions OIDC and emits Sigstore bundles. Consumers must validate both the checksum and the bundle identity before executing a downloaded archive.

Release automation rejects lightweight tags, builds from an annotated tag with locked dependencies, and records the resolved source commit in `RELEASE-MANIFEST.json`. A successful build proves only artifact reproducibility for the pinned source and target; it does not prove adapter coverage or measurement validity.

## SBOM requirements

Each CycloneDX SBOM derives from the locked Cargo package and resolve graph, not a target-directory scan. Each sanitized dependency record identifies package name, version, source, checksum, license, and dependency edges without manifest paths, workspace paths, environment values, or build-host identifiers. The signed release manifest binds the source commit, target triples, archive digests, dependency-record digests, and SBOM digests. Release assets must not include participant files, study exports, local paths, environment values, credentials, or build-host identifiers.

## Revocation

If a signing identity, archive, dependency, or release workflow is compromised, maintainers must mark the affected release withdrawn, publish replacement verification material, and document the affected version range without disclosing participant data.
