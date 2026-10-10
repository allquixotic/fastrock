# Fastrock Rust / Slint port

Based on allquixotic/codex-gui; upstream identity in codex-gui-base.json.
Preserve upstream conversation features, Slint patches, licenses and attribution.
All newly written UI must use native Slint controls/layouts; no canvas renderer.
Rally documents use horizontal tabs; sidebar contains Codex conversations only.
Keep Codex external: spawn installed `codex app-server` over stdio. Never embed
codex-core, inference, app-server, V8 or a second agent runtime in Fastrock.
Inherit installed Codex environment/configuration; never install/update Codex.
All build profiles prioritize compilation speed: opt-level=0, no LTO, no debug
information, 256 codegen units, incremental builds, no stripping/packing pass.
Optimized builds require Sean's explicit authorization. Prereleases use fast builds.
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
