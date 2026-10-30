# Fastrock Rust / Slint port

Based on allquixotic/codex-gui; upstream identity in codex-gui-base.json.
Preserve upstream conversation features, Slint patches, licenses and attribution.
All newly written UI must use native Slint controls/layouts; no canvas renderer.
0.4.0 authorization, 2026-10-29: embed unmodified stable Goose and vwork in
Fastrock; no installed backend, external core service, or Codex connection.
Configured extensions, hooks, credential helpers and user/LLM tools may spawn
processes. Use supported Goose APIs; report upstream blockers, never fork Goose.
Preserve Goose configuration/credential entries; support Bedrock, direct OpenAI
and direct Anthropic, tools/hooks/recipes/code mode. Exclude local inference,
voice, remote agent servers and all T3 quota-related features.
Required routes: Bedrock Runtime OpenAI GPT-6.1 Sol/GPT-6 Luna and Bedrock
Anthropic Claude Opus 5.5 with US cross-region inference. Preserve working routes;
reasoning choices follow Goose, including custom saved values if supported.
Port pinned stable T3 UI/orchestration into native Slint/Rust. Include scheduling,
isolated child vwork workspaces, explicit merge, durable queues and recovery.
Restart restores state but needs Resume; never blindly replay uncertain tools.
Projects represent repositories. Permanent Rally virtual project contains
closable horizontal document tabs/boards without conversation binding. Retain
on-demand temporary Rally assistance and human Apply. Web previews use browser.
Close last window to tray; explicit Quit stops work and exits.
Vwork exclusively owns its store in-process; preserve compressed virtual source,
leases, revision guards and hooks. Search keeps one compressed compaction-length
summary per conversation in the same redb, replaced after each completed user
turn; backfill one current summary per old Goose conversation. Search is local,
case-insensitive substring first then bounded parallel fuzzy search. Cheap model:
explicit profile, else Bedrock GPT-6 Luna; unavailable means text search only.
Codex conversation files stay untouched and disappear from the UI.
Authorized 2026-10-29: application/dependencies opt-level=2, lto="off" including
local ThinLTO, debug=0, 256 codegen units, incremental, no stripping/packing.
Build scripts stay unoptimized. 2026-10-30 supersedes milestone publication:
no further released binaries until the complete spec is implemented and verified
as a release candidate. Continue source commits/pushes and local fixture builds;
do not tag/publish incomplete milestones. Final release requires full acceptance.
Windows x64 ZIP only; Mac compile only.
Benchmark on games before releases: 1,000 conversations, eight streaming agents,
10,000-message history against 0.3.x. Stop test at 32 GB process memory, then
reduce workload tenfold; monitor host/guest headroom and preserve VM availability.
Stable 0.3.0 and 0.3.1 are explicitly authorized with the existing fast profile and Windows
x64 ZIP publication only; compile Mac for compatibility but skip its release job.
Explicit exception: official 0.2.0 uses profile official, opt-level=1, debug=2,
packed PDB/dSYM symbols, lto="off", no stripping; keep 256 codegen units and
incremental builds. Publish matching verified symbol archives with the release.
Always commit, push to main and release through CI; never create PRs.
Release assets must be built, signed, verified and published by CI. Include Windows
Authenticode/timestamping and Mac Developer ID/notarization/stapling; no unsigned
fallback. Temporary Mac runners handle trusted release tags only and run no GUI.
Windows x64 first; macOS supported. Never launch GUI/tests on Sean's Mac.
Use games via `ssh games`; read C:\Users\SeanMcNamara\AGENTS.md first.
Keep network, filesystem and keyring I/O off Slint's UI thread.
Tokens only in OS credential vaults; preserve legacy fastrock service/endpoint keys.
Use local HTTP/process fixtures for tests. Never perform real Rally writes in tests.
Preserve existing Git history/unrelated changes; no force pushes.
Use supported RTK filters when useful; exact-output commands run directly.
Agentlocks retired. Delegate only when requested; one writer per shared file.
Check Mac and games free space before builds. If low, remove only identified stale,
regenerable build outputs; preserve active incremental caches and release evidence.
Migration acceptance checklist: docs/RALLY-INVENTORY.md. SPEC.md tracks work.
