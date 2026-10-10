# Fastrock product contract

This Rust codebase imports codex-gui at the revision pinned in codex-gui-base.json.
Its native Slint conversations, approvals/questions, file/diff viewers, settings,
provider forms and sidebar remain the foundation. The original imported contract
is retained in docs/upstream/GUI.md for reference.

Fastrock changes the backend boundary: the installed external `codex app-server`
owns inference, configuration and agent execution. The GUI speaks JSON RPC over
stdio (or an explicitly selected daemon/remote connection). It never embeds a
second Codex runtime. All new Rally UI uses native Slint controls and layouts.

Rally documents remain horizontal tabs; sidebar rows remain LLM chats. Detailed
layout, behavior, safety boundaries, migration requirements and acceptance
coverage are in docs/RALLY-INVENTORY.md. SPEC.md records implementation tasks and
invariants. Preserve draft isolation and never silently apply assistant writes.

Compile speed has precedence over binary size and runtime optimization in every
profile. No optimized build without Sean's explicit authorization. GUI acceptance
runs on Windows; Sean's Mac is restricted to headless compilation and tests.
