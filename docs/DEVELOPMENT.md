# Fastrock Development Workflow

Fastrock development follows `SPEC.md` as the product and engineering source of
truth and `AGENTS.md` as the agent workflow source of truth.

## Required Command Shape

All normal shell commands must be wrapped with `rtk`:

```bash
rtk cargo fmt --check
rtk cargo test --workspace
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo run -p fastrock-app
```

Do not run provider tests against real AWS accounts in CI. Use fake Bedrock
Mantle, Bedrock Runtime, RTK, remote, and MCP fixtures from
`fastrock-test-support`.

## Spec Task Workflow

1. Read `SPEC.md`.
2. Select the next pending `T` row whose dependencies are complete.
3. Mark the row `>`.
4. Implement code and tests for the invariants named in that row.
5. Run formatting, tests, and clippy.
6. Mark the row `x` only after verification passes.

Do not mark the full goal complete until every explicit task, invariant, and
release criterion is verified against the current worktree.

## Agent Workflow

- Use terse technical communication.
- Use Caveman/RTK workflow from `AGENTS.md`.
- Use `apply_patch` for manual edits.
- Do not use destructive git commands.
- Do not commit unless the user explicitly asks.
- Keep provider tests fake or fixture-backed.
- Keep UI-thread work presentation-only; all blocking work belongs behind
  actors or background tasks.

## Release Verification

Before a release candidate:

```bash
rtk cargo fmt --check
rtk cargo test --workspace
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo build --release -p fastrock-app
```
