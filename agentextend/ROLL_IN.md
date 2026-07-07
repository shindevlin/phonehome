# AgentExtend Roll-In

This directory preserves the AgentExtend TypeScript implementation inside the Phonehome repository for review and migration.

Status:
- Source was copied from the sibling `agentextend` checkout on 2026-07-07.
- It is not yet wired into the Rust `phonehome` binary.
- Treat this as a contained subproject until the extension protocol is mapped onto Phonehome backends/routes.

Recommended next migration steps:
1. Keep TypeScript tests passing inside `agentextend/`.
2. Identify protocol surfaces that belong in `phonehome/src/routes.rs` and backend adapters.
3. Move durable docs into the Phonehome README/DEVELOPER docs after the API is stable.
