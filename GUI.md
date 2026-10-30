# Fastrock product contract

Fastrock 0.4.0 embeds unmodified stable Goose and vwork in one process, with a
native Slint interface adapted from the pinned stable T3 Code release. The
imported codex-gui implementation and attribution remain the presentation
foundation; its historical contract is retained in docs/upstream/GUI.md.
SPEC.md defines the current implementation and acceptance requirements.

Goose owns inference, tools and provider behavior through supported public Rust
APIs. No installed Goose, Codex connection, or external core daemon is required.
Configured extensions, hooks, credential helpers and requested tools may spawn
processes. Goose configuration and credentials remain separate from Fastrock UI
preferences. New tokens are stored only in the OS credential vault.

Projects represent registered repositories; conversations bind to isolated vwork
workspaces. Keep compressed source, explicit integration, durable coordination,
and restart recovery requiring Resume. The permanent Rally sidebar project holds
independent, closable horizontal documents and boards. Its optional temporary AI
assistant proposes changes for human Apply. The Rally migration requirements and
acceptance evidence are in docs/RALLY-INVENTORY.md.

Use native Slint controls and layouts, bounded transcript memory, efficient
incremental updates and background filesystem/network/keyring operations. Search
runs locally over titles and current compressed summaries, with a designated
cheap model refreshing summaries after completed turns.

Application and dependencies use optimization level 2, no LTO, no debug info,
256 codegen units and incremental builds; build scripts remain unoptimized.
Run tests and GUI acceptance on Windows games; Sean's Mac is compile-only.
Publish signed Windows x64 ZIPs through CI only after feature-complete release
candidate readiness; final release requires all acceptance, including required
Bedrock model routes. Source commits and pushes to main continue during development.
