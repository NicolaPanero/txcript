# txcript fork

The `fork` branch is official txcript at
`8cd3b0e63f797b1531a14197f41a0e9eeedec8c6` plus the fixes that
[NicolaPanero/superset](https://github.com/NicolaPanero/superset) needs to move
conversations between agents natively:

- Cursor: sessions written for Cursor carry the conversation's time zone
  (without it Cursor's agent rejects the chat), only the active transcript is
  read, literal XML examples are not discarded as environment setup, the `<timestamp>` prefix of CLI queries is dropped, and messages read
  from Cursor keep their order (Cursor stores one time for a whole chat).
- Grok: `synthetic_reason` and the display log separate human prompts from
  injected instructions, compaction summaries and automatic continuation.
  Useful runtime context remains explicitly labelled as assistant context,
  without being attributed to the user. Combined context/query records,
  literal tag examples and image-only prompts retain their real input.
- Codex: structured tool outputs are kept, shell failures retain their native
  exit status, and the AGENTS.md instructions Codex records as a user message
  are not read as a prompt.
- OpenCode: tool results stay paired with their calls.
- `txcript list --json`: the session list as JSON (with `updated_at`), for
  Zed Fork's "Find chat…" and Superset's "Import chat"; `--under DIR`
  (repeatable) filters by folders and `--preview` adds each session's first
  prompt.
- `examples/superset-transfer.rs`: Superset's conversion helper.

Tags are named `v<txcript version>-fork.<n>`, and the CLI reports that
version. `v0.14.4-fork.3` is Superset's
`tools/txcript-transfer/native-transfer.patch`; `fork.4` adds the message
order for Cursor to the library, so the CLI's exports get it too; `fork.5`
skips Codex's AGENTS.md prelude; `fork.6` publishes releases automatically; `fork.7` adds `list --json`; `fork.8` adds `--under`, `--preview` and
`updated_at` to it. `fork.9` fixes Grok context attribution, literal tag
examples and Codex shell errors, and adds the conversion regression matrix.

## Releasing

1. Commit the fix on the `fork` branch.
2. `scripts/set-fork-version.sh <N>` (the next number after the latest
   `v…-fork.N` tag), and commit the version change.
3. Push the branch and a `v<version>-fork.<N>` tag.

`.github/workflows/fork-release.yml` then tests, builds the `txcript` CLI and
the `superset-transfer` helper for Apple Silicon, and publishes them as the
latest release. Superset++ pins the tested version in
`scripts/fork/txcript-version.json`; update that pin only after the txcript
release succeeds. A dispatch requests a Superset build but does not advance
its pin. Zed no longer participates in this release flow.

## Conversion checks

`cargo test --locked -p txcript --no-default-features --features opencode`
runs the library, native format integration tests, regression tests and
property-based codec checks. `cargo test --locked -p txcript --example
superset-transfer --no-default-features --features opencode` verifies the
conversion helper. These checks run on pushes to `fork` and before releases.

The explicit matrix covers all 20 directions between Claude Code, Codex,
Cursor Agent, Grok and OpenCode, plus each codec's own serialization cycle.
It checks roles, ordinary and literal XML text, reasoning text, Read/Write/
Edit/Bash/MCP calls, shell errors, structured output and images. Grok native
fixtures also cover runtime injections, compaction, legacy untagged context,
combined setup/query records, image-only input and malformed tags.
Superset++ separately exercises the real helper and temporary stores in CI.
No test runs a model, uses live credentials or changes an existing session.

Native formats can still differ by agent version. Provider-private reasoning
and credentials are not portable, and tool results may become JSON text in
formats without structured output. Fixtures cannot prove compatibility with
every past or future CLI build; keep a minimal regression for each new issue.
