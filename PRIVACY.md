# Privacy contract

Second Observer measures retained local activity descriptively. It is not a complete-history claim unless an adapter can prove complete retention coverage.

## Collection

- `discover` uses documented, allowlisted locations and filesystem metadata only. It does not execute detected software.
- `consent init` must show every adapter, source class, field class, and local analyzer before content is read.
- Metadata collection is the default. Content-derived analysis requires a distinct explicit opt-in.
- Raw prompts, commands, transcripts, tool output, paths, titles, repository names, remotes, URLs, identifiers, and stable machine identifiers do not persist or upload.
- Content-derived signals are aggregated in memory and exported only as approved scalar metrics.

## Review and upload

- `preview` displays the exact proposed payload and excluded-field assertions.
- `export` writes a local aggregate archive with integrity digests.
- `second-observer` does not upload.
- `second-observer-upload send` is a separate command and requires your explicit approval after preview.
- The uploader accepts one canonical finalized export of at most 262,144 bytes. It rejects symlinks,
  hard-linked files on Unix, non-HTTPS endpoints, and redirects.

## Retention

[assumed] The private intake retains validated aggregate archives for 30 days, retains derived study records until study closure or revocation, and deletes rejected or quarantined uploads immediately.

[assumed] Revocation deletes the stored aggregate archive, derived study metrics, and hosted report while retaining a non-content deletion receipt.

## Limits

The instrument cannot recover unavailable history. It preserves missingness and coverage discontinuities instead of replacing them with zeroes. Cross-provider token accounting remains source-specific.
