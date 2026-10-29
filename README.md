# Fastrock

Native Rust/Slint workspace for Goose conversations and Rally work. The codebase is
based on [allquixotic/codex-gui](https://github.com/allquixotic/codex-gui), with Rally
rebuilt using Slint buttons, editors, lists, layouts and dialogs. Rally documents
occupy horizontal tabs; the sidebar remains a list of LLM conversations.

Goose is the default agent. Install/configure **Goose CLI 1.56.0 or newer** separately;
Fastrock starts one `goose acp` child over stdio and multiplexes concurrent
conversations. It inherits Goose's environment, provider configuration, extensions,
and session store. Set `FASTROCK_GOOSE` to select an installed executable.
No Goose crates, agent runtime, inference engine or server are compiled into Fastrock.
Update Goose independently to receive its model support and fixes.

Open a conversation and expand **Goose options** above Send for the provider,
model, mode and other choices advertised by Goose. Configure credentials and
extensions using `goose configure`. [Architecture and capability details](docs/goose-acp.md)
explain why stdio ACP supports this design and where the two backends differ.

Installed Codex CLI **0.162.0 or newer** remains an explicit option under Settings →
Connection, or `fastrock --remote codex`. `FASTROCK_CODEX` selects its executable.
Existing Codex conversations remain available through that connection. Explicit
saved Codex/remote selections are preserved; an unset connection now selects Goose.
Fastrock never installs or updates either backend.

## Releases

[Official **0.2.0**](https://github.com/allquixotic/fastrock/releases/tag/v0.2.0%2Bbuild.1) uses basic optimization (`opt-level=1`), full debugging
symbols (`debug=2`) and `lto="off"`. Matching Windows PDB and Mac dSYM symbol
archives are published separately for crash debugging. Development and
prerelease builds retain the fast settings below.

## Test builds

[Fastrock 0.3.4](https://github.com/allquixotic/fastrock/releases/tag/v0.3.4)
is available as a signed Windows x64 ZIP. Extract it and run `fastrock.exe`.
All release assets are built and published by CI. Current releases compile
macOS for compatibility; historical Mac releases include signed, notarized DMGs.
Application code uses optimization level 0, no debug information, no LTO,
256 codegen units and incremental compilation. The explicitly authorized
rendering/text dependency allowlist uses optimization level 2. The `official`
0.2.0 profile is separate. The software renderer avoids compiling a second GPU
stack. See [build/release instructions](docs/releases.md).

```sh
python3 dev/check-build-policy.py
cargo build --locked --bin fastrock
```

The executable is `target/debug/fastrock` (`fastrock.exe` on Windows). Windows and
macOS build scripts use the same fast development profile. Never launch the app or
tests on Sean's Mac; execute tests on Windows. The Mac compiles and signs only.

## Virtual workspaces

When Goose is configured with an enabled, installed vwork MCP extension, the
toolbar offers **Virtual** to browse and edit all configured stores, repositories
and workspaces. Files stay in vwork; saves check revisions and preserve drafts
on conflict. The controls disappear when vwork is not configured. See
[setup and usage](docs/VIRTUAL_WORKSPACES.md). With vwork 0.3.3, conversations
reuse its daemon through localhost HTTP without per-conversation client processes.
The Compact control covers the entire selected database.

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
