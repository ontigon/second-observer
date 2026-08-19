# Participant workflow

## The short version

```text
second-observer run
```

That is the whole thing. It asks what it needs, shows you what it found, shows you the exact
payload, and writes nothing until you say yes. You do not need an agent, and you do not need to
remember any other command.

## Getting the binary

Download the archive for your platform from the
[latest release](https://github.com/ontigon/second-observer/releases/latest), together with
`SHA256SUMS`, then verify and extract:

```text
shasum -a 256 -c SHA256SUMS --ignore-missing
tar xzf second-observer-<version>-<platform>.tar.gz
```

Checksum verification is required. Signature verification with `cosign` is stronger and
recommended, but not having `cosign` installed is not a reason to stop — see
[release verification](../../README.md#release-verification) for what each check does and does
not prove.

**macOS:** if you download through a browser, Gatekeeper marks the binary and refuses to run it.
Either download with `gh release download` or `curl`, or clear the flag after verifying the
checksum:

```text
xattr -d com.apple.quarantine second-observer
```

Run the collector from your home directory or a scratch folder, not from inside a source
checkout: it writes its state into `.second-observer/` in the current directory.

## Running it again

Every run is a separate snapshot. Nothing is overwritten: each collection gets a fresh run ID
and its own export file, and all of them are recorded in an index you can select from later.

The second time you run it, it shows the answers you used last time and offers to reuse them:

```text
Take another snapshot with these same answers? [Y/n]
```

Press Enter and it goes straight to collecting. To skip even that question:

```text
second-observer run --repeat
```

`--repeat` reuses the stored answers with no questions at all. It does **not** skip the consent
summary or the payload review — those two still require an explicit `y`, because an accidental
keystroke must never stand in for consent.

Discovery runs again on every repeat. If a tool you previously approved has stopped reporting
`observed`, it says so before collecting, because that becomes a coverage change rather than a
change in your behaviour.

List what you have collected:

```text
second-observer snapshots
second-observer snapshots --json     # run id, window, adapters, digest, path
```

Take as many as you like. Choosing which snapshot to use for a report or a benchmark is a
decision you make later, from that list.

## What `run` asks you

These are asked on your first run, and proposed as defaults on every run after it.

1. **Home directory.** It proposes yours; confirm or replace it.
2. **Timezone.** It proposes the system zone. It requires an IANA name such as
   `America/Toronto`, and rejects abbreviations like `EST` — an abbreviation pins a fixed offset
   with no daylight-saving transition, which shifts every day boundary for half the year.
3. **Which adapters to approve.** It runs discovery first, shows you what is present, and
   proposes exactly the ones that returned `observed`. Take the default unless you have a reason
   not to. Comparison later requires every approved adapter to be observed in **both** phases, so
   approving a tool you do not use makes every future comparison `INCOMPARABLE`.
4. **Baseline or post.** A post collection must match its baseline on window length, adapter
   list, and `.second-observer/study-identity.json`.
5. **Window length in days.** Default 28. Baseline and post must use the same value; `compare`
   derives both lengths from the recorded bounds and refuses an unequal pair.
6. **Optional content analysis.** Off by default. It derives relay, routing, and correction
   heuristics from message and command text locally. No text is stored or exported.

Then it shows you the consent manifest and waits. Then it collects, shows you the entire payload
in readable form, and waits again. Nothing is written to an export until that second yes, and
nothing is ever uploaded.

## Doing it by hand

Every step `run` performs is also a separate command, and those commands infer nothing — they
require every value explicitly. Use them for scripted or audited collection:

```text
second-observer discover --home <participant-home>
second-observer consent init --phase <baseline|post> --adapter <id> [--adapter <id> ...]
second-observer collect --profile retained-history --baseline <days> --phase <baseline|post> --home <participant-home> --timezone <iana-timezone>
second-observer preview          # readable form; add --json for the exact canonical bytes
second-observer export
second-observer verify <export>
```

`discover` reports known tool availability. It does not execute tools and does not establish that
a tool has usable retained history: an adapter can report `observed` and still return typed
missing values if its retained records fall outside your window.

Each consent manifest authorizes exactly one phase. Keep
`.second-observer/study-identity.json` between baseline and post collections; it stores random
participant and device IDs owner-only, while each collection receives a fresh run ID.

`preview` is the review boundary. Stop if anything is unexpected.

## Comparing your own phases

```text
second-observer compare <baseline-export> <post-export>
```

This compares one person's two phases. It is not a cross-participant tool: two different people's
exports return `INCOMPARABLE` on `study_identity_changed`, by design.

## Optional upload

Upload only after you have reviewed the payload and explicitly decide to send it:

```text
second-observer-upload send <export> --study-code <code> --confirm
```

The upload client submits exactly one finalized export and cannot inspect local telemetry
sources. Save the returned receipt for status or revocation:

```text
second-observer-upload status <receipt> --study-code <code>
second-observer-upload revoke <receipt> --study-code <code> --confirm
```

Do not give a study code to an agent unless you have already approved the specific upload command.

## Interpretation

The report compares descriptive measures only. It displays coverage limitations and preserves
missing values. It does not establish productivity, correctness, causation, or cost savings.
