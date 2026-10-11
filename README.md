# Fastrock

Native Rust/Slint workspace for Codex conversations and Rally work. The codebase is
based on [allquixotic/codex-gui](https://github.com/allquixotic/codex-gui), with Rally
rebuilt using Slint buttons, editors, lists, layouts and dialogs. Rally documents
occupy horizontal tabs; the sidebar remains a list of LLM conversations.

Codex remains external. Install Codex CLI **0.162.0 or newer** on PATH; Fastrock
starts `codex app-server` over stdio and inherits its configuration and environment.
`FASTROCK_CODEX` can name another installed executable. Fastrock contains no
inference runtime, app-server, V8, or bundled Codex helper executables. Existing
Codex GUI conversation features remain available through the external protocol.

## Releases

Official **0.2.0** uses basic optimization (`opt-level=1`), full debugging
symbols (`debug=2`) and `lto="off"`. Matching Windows PDB and Mac dSYM symbol
archives are published separately for crash debugging. Development and
prerelease builds retain the fast settings below.

## Test builds

Windows x64 prereleases are available from [Fastrock releases](https://github.com/allquixotic/fastrock/releases).
Extract the signed ZIP and run `fastrock.exe`. Apple Silicon Macs get a signed,
notarized and stapled DMG. All release assets are built and published by CI.
These builds deliberately have **zero
optimization**, no debug information, no LTO, 256 codegen units and incremental
compilation. Default profiles, including `release`, follow this policy; the explicitly
authorized `official` profile is separate. Optimized
builds require explicit authorization. The software renderer avoids compiling a
second GPU stack. See [rapid build/release instructions](docs/releases.md).

```sh
python3 dev/check-build-policy.py
cargo build --locked --bin fastrock
```

The executable is `target/debug/fastrock` (`fastrock.exe` on Windows). Windows and
macOS build scripts use the same fast development profile. Never launch the app or
GUI tests on Sean's Mac; use Windows for UI acceptance. Mac checks are headless.

## Rally

Open a page from the Rally menu, then configure the endpoint, workspace and API
token in Rally Settings. Tokens stay in the OS credential vault, using the same
Fastrock service/endpoint keys as the Go app. Existing `settings.json` and Rally
`session.json` documents migrate to the new atomic `rally.json` store. Descriptions,
notes, filters, selections and unsent drafts remain document-specific.

The detailed [Rally inventory and migration ledger](docs/RALLY-INVENTORY.md)
records the old layout, all implemented workflows, underlying API behavior and
verification evidence. [The user guide](docs/gui.md) covers the native controls.
Tests use local HTTP/process fixtures and never make real Rally writes.

## Source and attribution

`codex-gui-base.json` pins the imported codex-gui revision and preserved Fastrock
baseline. `upstream.json` and Cargo.lock pin the Codex protocol/config utility
crates; the actual server always belongs to the installed CLI. The original Go
app remains recoverable in Git history. Apache-2.0; see LICENSE and NOTICE.
This is an independent community app, not an official OpenAI product.
