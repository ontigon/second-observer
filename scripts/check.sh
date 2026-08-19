#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo build --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

while IFS= read -r -d '' json_file; do
  jq -e . "${json_file}" >/dev/null
done < <(find registry schemas fixtures -type f -name '*.json' -print0 | sort -z)

jq -e '
  .properties.windows.items.enum == ["retained_history", "baseline_28d", "post_28d"]
  and .properties.windows.minItems == 2
  and .properties.windows.maxItems == 2
  and .properties.windows.uniqueItems == true
' schemas/consent-v1.schema.json >/dev/null

collector_tree="$(cargo tree -p second-observer -e normal --prefix none)"
if rg -q '(^| )((reqwest|hyper|http|h2|tower|ureq|curl)( |$))' <<<"${collector_tree}"; then
  echo "collector network dependency detected" >&2
  exit 1
fi

uploader_tree="$(cargo tree -p second-observer-upload -e normal --prefix none)"
if rg -q 'observer-(core|adapters)' <<<"${uploader_tree}"; then
  echo "uploader crossed the collector or adapter dependency boundary" >&2
  exit 1
fi

if rg -n --glob '*.rs' '(reqwest|TcpStream|TcpListener|UdpSocket|std::net|hyper|ureq|curl)' \
  crates/observer-core crates/observer-adapters crates/observer-cli; then
  echo "collector source references a network client or socket API" >&2
  exit 1
fi

release_workflow=".github/workflows/release.yml"
rg -q 'cargo build --locked --release' "${release_workflow}"
rg -q -- '--bin second-observer --bin second-observer-upload' "${release_workflow}"
rg -q 'toolchain: 1.97.1' "${release_workflow}"
rg -q 'runner: macos-15-intel' "${release_workflow}"
rg -q 'SHA256SUMS' "${release_workflow}"
rg -q 'sha256sum -c SHA256SUMS' "${release_workflow}"
rg -q 'Set-Content -Encoding ascii' "${release_workflow}"
rg -q 'upload-artifact: false' "${release_workflow}"
rg -q 'upload-release-assets: false' "${release_workflow}"
rg -q 'Require an annotated release tag' "${release_workflow}"
rg -q 'RELEASE-MANIFEST.json' "${release_workflow}"
rg -q 'cargo metadata --locked --format-version 1' "${release_workflow}"
