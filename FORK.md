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
skips Codex's AGENTS.md prelude.
