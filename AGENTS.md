@/Users/sean/.codex/RTK.md

# Fastrock Agent Instructions

Fastrock is a Rust/Slint desktop coding-agent app. Development priorities:
performance first, AWS Bedrock first, no hosted Fastrock cloud, no telemetry,
no routine approval prompts, and all command execution through external `rtk`.

## Required Workflow

- Use Caveman style for agent communication: terse, technical, no filler.
- Wrap normal shell commands with `rtk`.
- Do not run raw shell commands unless debugging `rtk` itself or a command cannot
  reasonably be proxied. If bypass needed, explain why first.
- Prefer `rg` / `rg --files` for search.
- Read relevant files before editing.
- Use `apply_patch` for manual file edits.
- Do not use destructive git commands.
- Do not commit unless user asks.
- Treat `SPEC.md` as product source of truth.

## Development Commands

Expected command shape:

```bash
rtk cargo fmt --check
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo test --workspace
rtk cargo run -p fastrock-app
```

When workspace is not scaffolded yet, create only files needed for current task.
Do not invent placeholder crates unless user asks to build implementation.

## Architecture Rules

- Slint event loop owns main thread. Never block it.
- All network, disk, subprocess, SSH, SSM, MCP, indexing, parsing, and database
  work goes through background actors/tasks.
- Communicate between UI and runtime with typed commands/events.
- Use bounded channels for high-volume streams.
- Coalesce model token deltas and command output before UI updates.
- Persist conversation history as append-only events with materialized indexes.
- Store secrets only in OS keyring; store references in SQLite/config.

## Bedrock Rules

- Bedrock Mantle is primary provider path.
- Bedrock Runtime is secondary provider path.
- Mantle endpoint shape is `https://bedrock-mantle.{region}.api.aws/v1`.
- Runtime endpoint shape is `https://bedrock-runtime.{region}.amazonaws.com`.
- Mantle Responses API is preferred when model supports it.
- Mantle Chat Completions or Anthropic Messages are compatibility fallbacks.
- Runtime ConverseStream is preferred; InvokeModel streaming is fallback.
- Keep Mantle quotas/accounting separate from Runtime quotas/accounting.
- Maintain one shared AWS credential/profile resolver for Mantle SigV4 and
  Bedrock Runtime.
- Explicitly support AWS CLI named profiles for both
  `bedrock-mantle.{region}.api.aws` SigV4 calls and
  `bedrock-runtime.{region}.amazonaws.com` Runtime calls.
- Test AWS profile behavior with fake shared config/credential files, not real
  AWS accounts.
- Do not call real AWS services in tests. Use mocks or sanitized fixtures.

## RTK Rules

- Fastrock depends on external `rtk`; do not vendor or reimplement it.
- Local commands run as `rtk <command...>` or `rtk <shell> -lc <command>`.
- Remote commands must invoke remote `rtk` when available.
- If remote `rtk` is missing, fail with install guidance unless explicit fallback
  setting allows raw remote execution.
- Command transcript must include cwd, target, status, stdout/stderr, and
  truncation/savings metadata when available.

## Plan Mode and Goal Loop

- Plan mode is read-only and enforced by tool routing, not prompt text alone.
- Goal loop state belongs to the conversation runtime and persists.
- Active goals track objective, status, token budget, tokens used, elapsed time,
  and timestamps.
- Do not let plan-mode turns mutate project files.
- Do not let hidden conversations block active UI input.

## Testing Expectations

- Add tests with behavior changes.
- Provider tests use fake Bedrock Mantle/Runtime servers.
- Command tests use fake `rtk`.
- Remote tests use fake SSH/SSM transports unless explicitly running manual
  integration checks.
- Performance-sensitive work needs a benchmark or stress fixture before merge.

## Documentation

- Update `SPEC.md` when product behavior, interfaces, invariants, or task graph
  change.
- Keep docs concrete. Name data shapes, actors, commands, events, and failure
  modes.
- Do not leave implementation-critical choices as vague future decisions.
