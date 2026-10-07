# txcript fork

The `fork` branch is official txcript at
`8cd3b0e63f797b1531a14197f41a0e9eeedec8c6` plus the fixes that
[NicolaPanero/superset](https://github.com/NicolaPanero/superset) and
[NicolaPanero/zed](https://github.com/NicolaPanero/zed) need to move
conversations between agents natively:

- Cursor: sessions written for Cursor carry the conversation's time zone
  (without it Cursor's agent rejects the chat), only the active transcript is
  read, the `<timestamp>` prefix of CLI queries is dropped, and messages read
  from Cursor keep their order (Cursor stores one time for a whole chat).
- Codex: structured tool outputs are kept, and the AGENTS.md instructions
  Codex records as a user message are not read as a prompt.
- OpenCode: tool results stay paired with their calls.
- `examples/superset-transfer.rs`: Superset's conversion helper.

Tags are named `v<txcript version>-fork.<n>`, and the CLI reports that
version. `v0.14.4-fork.3` is Superset's
`tools/txcript-transfer/native-transfer.patch`; `fork.4` adds the message
order for Cursor to the library, so the CLI's exports get it too; `fork.5`
skips Codex's AGENTS.md prelude; `fork.6` publishes releases automatically.

## Releasing

1. Commit the fix on the `fork` branch.
2. `scripts/set-fork-version.sh <N>` (the next number after the latest
   `v…-fork.N` tag), and commit the version change.
3. Push the branch and a `v<version>-fork.<N>` tag.

`.github/workflows/fork-release.yml` then tests, builds the `txcript` CLI and
the `superset-transfer` helper for Apple Silicon, and publishes them as the
latest release. Zed Fork and Superset build with the latest release: right
away when the repository has a `DISPATCH_TOKEN` secret (a fine-grained token
with "Actions: read and write" on NicolaPanero/zed and NicolaPanero/superset),
otherwise at their next daily check. Each app's release name ends with the
txcript fork number it bundles (`-tx<N>`), so their in-app updates offer it.
