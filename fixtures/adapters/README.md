# Adapter fixtures

Synthetic fixtures are permitted and required for deterministic adapter tests. They exercise supported,
missing, malformed, unsupported-version, and permission-denied paths without containing
participant-derived histories, identifiers, paths, or content.

- `claude-code/positive.jsonl`: recognized retained JSONL events.
- `claude-code/malformed.jsonl`: malformed and unsupported records.
- `claude-code/permission-denied.jsonl`: source content for a restrictive-permission test.
- `codex/positive.jsonl`: recognized retained JSONL event.
- `codex/unsupported.jsonl`: unsupported retained JSONL record shape.
- `cursor/positive.sql`: synthetic candidate SQLite schema; the v1 adapter remains detection-only.
- `cursor/unsupported.sql`: synthetic unsupported SQLite schema; no Cursor content is parsed or exported.
- `shell-history/positive.txt`: opt-in command-text fixture.

Missing fixtures are represented by an absent fixed root. Permission-denied tests apply restrictive
permissions to the synthetic `permission-denied.jsonl` source because repository permission bits differ
by platform.
