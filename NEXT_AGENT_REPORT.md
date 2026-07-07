# Next Agent Report - phonehome

Date: 2026-07-07
Branch: `review/local-fix-20260707`
Remote: `shindevlin/phonehome`

## Current State

- Review branch is pushed and tracking `origin/review/local-fix-20260707`.
- Multi-backend work is present: BTCPC/Hone, Codex, Gemini, OpenRouter, chat log, routes/linking.
- `agentextend/` was rolled in as a contained TypeScript subproject with `agentextend/ROLL_IN.md`.
- Local `.phonehome` and `.claude/` are ignored.

## Test Results

- `cargo test`: PASS
- `npm test` inside `agentextend/`: FAIL after package JSON fix because `jest` is not installed.
- Logs:
  - `/mnt/btcpc-storage/repos/docs/test-reports/2026-07-07/logs/phonehome__cargo_test.log`
  - `/mnt/btcpc-storage/repos/docs/test-reports/2026-07-07/logs/phonehome__agentextend__npm_test.rerun.log`

## Notes

- Rust tests pass but coverage is minimal: one whisper tone test.
- AgentExtend is preserved but not wired into the Rust binary yet.

## Recommended Next Steps

1. Install dependencies in `phonehome/agentextend` and rerun `npm test`.
2. Decide how AgentExtend maps into Phonehome routes/backends.
3. Add Rust tests for backend routing and command handling.
