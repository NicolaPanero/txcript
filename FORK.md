# txcript fork

The `fork` branch is official txcript at
`8cd3b0e63f797b1531a14197f41a0e9eeedec8c6` plus the fixes that
[NicolaPanero/superset](https://github.com/NicolaPanero/superset) and
[NicolaPanero/zed](https://github.com/NicolaPanero/zed) need to move
conversations between agents natively:

- Cursor: sessions written for Cursor carry the conversation's time zone
  (without it Cursor's agent rejects the chat), only the active transcript is
  read, and the `<timestamp>` prefix of CLI queries is dropped.
- Codex: structured tool outputs are kept.
- OpenCode: tool results stay paired with their calls.
- `examples/superset-transfer.rs`: Superset's conversion helper.

Tags are named `v<txcript version>-fork.<n>`. The fixes are the same as
Superset's `tools/txcript-transfer/native-transfer.patch`.
