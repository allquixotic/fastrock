# Fastrock Rust / Slint port

Based on allquixotic/codex-gui; upstream identity in codex-gui-base.json.
Preserve upstream conversation features, Slint patches, licenses and attribution.
All newly written UI must use native Slint controls/layouts; no canvas renderer.
Rally documents use horizontal tabs; sidebar contains agent conversations only.
Default backend: installed `goose acp` over stdio, one process for concurrent
sessions. Installed `codex app-server` remains an explicit connection option.
Never compile Goose crates or embed codex-core, inference, app-server, V8 or a
second agent runtime in Fastrock. Inherit backend environment/configuration;
never install/update either backend. Negotiate ACP capabilities at runtime.
Build profiles prioritize compilation speed: application opt-level=0, no LTO, no debug
information, 256 codegen units, incremental builds, no stripping/packing pass.
Optimized builds require Sean's explicit authorization. Prereleases use fast builds.
Authorized 2026-10-26: allowlisted rendering/text dependencies use opt-level=2,
lto="off" including local ThinLTO. Keep other fast settings; benchmark on games
before 0.3.2 CI release. Continue Windows ZIP distribution and Mac compatibility compile.
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
