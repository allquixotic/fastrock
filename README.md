# Fastrock

Native Rust/Slint workspace for Goose conversations and Rally work. The codebase is
based on [allquixotic/codex-gui](https://github.com/allquixotic/codex-gui), with Rally
rebuilt using Slint buttons, editors, lists, layouts and dialogs. Rally documents
occupy their own horizontal tabs under the permanent Rally sidebar project.
Conversations use the sidebar and a separate title/action header.

The 0.4.0 development branch embeds unmodified Goose **1.57.0** and vwork
**0.3.4**. No separate Goose or vwork installation is required. Goose runs on
Fastrock's Tokio runtime through bounded memory streams. Existing Goose
configuration, credentials, extensions and sessions remain separate from Fastrock
UI settings. Configured extensions, hooks, credential helpers and user tools may
start processes.

Only Goose's Bedrock, direct OpenAI and direct Anthropic providers are enabled.
Expand **Goose options** above Send for the session's model, mode and supported
reasoning choices. External Codex and remote agent connections are removed;
Codex files remain untouched. [Embedded runtime status and limitations](docs/goose-acp.md)
describe the verified foundation and outstanding 0.4.0 work. The T3 sidebar/header foundation is present; full repository/workspace binding,
auxiliary panels, orchestration and large-history acceptance remain in progress.

Search matches titles and current summaries locally, followed by bounded parallel
fuzzy matching. After a completed turn, a background worker uses Goose's compaction
library to replace that conversation's compressed summary in the vwork database.
Existing Goose conversations are backfilled once; their original messages stay
intact. **Settings → Goose** shows the designated cheap profile (Bedrock Luna by
default). An unavailable cheap model leaves existing summaries and local search
available, without selecting a more expensive model.

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
The 0.4.0 branch uses optimization level 2 for application code and dependencies,
no debug information, no LTO, 256 codegen units and incremental compilation.
Build scripts remain unoptimized; symbol stripping is disabled. The `official`
0.2.0 profile is separate. The software renderer avoids compiling a second GPU
stack. See [build/release instructions](docs/releases.md).

```sh
python3 dev/check-build-policy.py
cargo build --locked --bin fastrock
```

The executable is `target/debug/fastrock` (`fastrock.exe` on Windows). Windows and
macOS build scripts use the same development profile. Never launch the app or
tests on Sean's Mac; execute tests on Windows. The Mac compiles only for current releases.

## Virtual workspaces

The **Virtual** toolbar opens the embedded vwork stores, repositories and
workspaces. Existing vwork homes are retained; fresh installations receive a
default local store. Files remain compressed in vwork, and saves enforce revision
checks. Compact covers the selected database. Fastrock exclusively owns each
open store, including the compressed current-summary table. Legacy vwork MCP
entries are mapped to the same in-process owner before session activation;
unsupported custom entries report an error. [Workspace details](docs/VIRTUAL_WORKSPACES.md).

## Rally

Choose **Rally** in the sidebar or open a page from the Rally menu, then configure the endpoint, workspace and API
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
crates retained as presentation/configuration types, without a Codex agent runtime. The original Go
app remains recoverable in Git history. Apache-2.0; see LICENSE and NOTICE.
This is an independent community app, not an official OpenAI product.
