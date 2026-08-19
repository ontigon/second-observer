# Codex prompt

```text
Use only the released Second Observer binaries and run this deterministic workflow exactly:

0. Download the correct `v0.1.1` archive for this machine from `https://github.com/ontigon/second-observer/releases/tag/v0.1.1`. Verify its SHA-256 entry and Sigstore bundle using the repository release instructions, then extract it. Do not build from source.
1. Ask me for the collection phase (`baseline` or `post`), explicit participant home directory, and IANA timezone. Do not infer any of them.
2. second-observer discover --home <participant-home>
3. second-observer consent init --phase <baseline|post>
4. Show me the consent manifest and wait for my explicit approval before collection.
5. After approval: second-observer collect --profile retained-history --baseline 28 --phase <baseline|post> --home <participant-home> --timezone <iana-timezone>
6. second-observer preview
7. Show me the exact preview and excluded-field assertions. Do not upload.
8. If I explicitly approve the shown export and provide a study code, run:
   second-observer export
   second-observer verify <export>
   second-observer-upload send <export> --study-code <code> --confirm

Do not inspect, search, summarize, or reason over local histories yourself. Do not infer consent, phase, home directory, timezone, study code, add adapters, change collection scope, execute detected tools, or call alternative collection scripts. Do not upload unless I explicitly approve the exact previewed export in this conversation. Stop on any error or difference from the approved scope and report it without retrying through another method.
```
