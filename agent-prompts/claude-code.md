# Claude Code prompt

The collector is interactive and drives itself. An agent's only useful job is getting you a
verified binary; after that it should get out of the way, because `second-observer run` asks
you the questions directly and an agent in the middle can only mistranslate your answers.

```text
Help me install Second Observer. Do only these steps and then stop.

1. Detect this machine's OS and architecture.
2. Download the matching archive and SHA256SUMS from the latest release of
   github.com/ontigon/second-observer. Use `gh release download` if the GitHub CLI is
   available, otherwise curl. Do not use a web search or a browser.
3. Verify the checksum and show me the output:
       shasum -a 256 -c SHA256SUMS --ignore-missing
   Stop if it does not say OK.
4. If `cosign` is on PATH, also verify the Sigstore bundle using the commands in the
   repository README and show me the output. If `cosign` is not installed, say so, say that
   this leaves provenance unverified, and continue — do not treat it as a failure and do not
   install anything to work around it.
5. Extract the archive. On macOS, if the binary carries com.apple.quarantine, tell me the
   `xattr -d` command and let me decide.
6. Tell me the full path to the extracted binary, then stop.

Do not run the collector. Do not run discover, consent, collect, preview, export, or compare.
Do not read, search, or summarise my local history. Do not create a .second-observer directory.
I will run `second-observer run` myself from my home directory and answer its questions.
```

Then, in your own terminal:

```text
cd ~
/path/to/second-observer run
```

It asks for a home directory, timezone, which adapters to approve, baseline or post, window
length, and whether to enable local content analysis. It shows you the consent manifest before
collecting and the entire payload before exporting. Nothing uploads.
