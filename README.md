# Fastrock

A native Go/go-fltk desktop workspace for Codex conversations and Rally work.
Windows 11 x64 is the primary target; macOS builds use the same application code.
Dark mode is the default. System, light and dark themes and font size are in Settings.
The status bar starts hidden; restore it from View and close it with its × icon.

The shell follows the Codex GUI layout: horizontal document tabs, a conversation-only
sidebar grouped by project, an optional conversation information panel, independent
conversation composers, and a settings document. Rally lives in document tabs,
not in the conversation list. Its navigation, board/list controls, scope selectors,
work-item details and saved views follow the supplied Rally Anywhere reference.
There is no embedded web browser and no official Rally logo.

## Requirements and build

Install **Codex CLI 0.162.0 or later** and put `codex` on PATH. The supported protocol
baseline is the current 0.162 release; use the latest Codex release. Fastrock checks
both the version and app-server initialization. It does not download, bundle,
compile, or replace Codex. A missing/obsolete/incompatible CLI produces a startup
error with Retry Codex and settings actions while the workspace stays usable.
The first native GUI frame is displayed before Fastrock invokes Codex, including
its version check. CLI checks and startup run in the background; closing during
startup cancels pending work. Fastrock never updates Codex itself: update the
installed CLI separately, then retry. `fastrock --doctor` is an explicit headless
diagnostic and checks the CLI without opening a GUI.

An app-server SQLite initialization error concerns Codex's local runtime state,
even when the CLI version is supported. Run `codex app-server` from the same
Windows account to check whether it also fails independently of Fastrock.
The terminal UI and a GUI with an embedded backend can behave differently.
Fastrock preserves the reported cause and does not reset Codex databases.

Use **Go 1.27.2** and **`CGO_ENABLED=1`**. macOS needs the Xcode command-line
tools; Windows needs MinGW-w64 UCRT GCC/G++ on PATH (MSYS2's `ucrt64/bin`,
with `CC` and `CXX` selecting its `gcc.exe` and `g++.exe`). `go-fltk` supplies pinned static
FLTK archives, linked into the main executable. No FLTK DLL/dylib, Rust compiler,
or embedded browser is required. Windows builds also link the compiler runtimes
statically. `dev/check-linkage.go` checks the final executable's imports.

Standard buttons and plain text fields use FLTK widgets. The transcript, rich/code
editors, tabs, boards, menus and other custom areas retain the existing layout and
canvas rendering. See [the UI and dependency notes](third_party/README.md).
For Windows cross-builds on macOS, install MinGW-w64 (`brew install mingw-w64`);
`WINDOWS_CC` and `WINDOWS_CXX` can override the compiler paths.

```sh
make windows       # build/fastrock.exe, needs MinGW-w64 GCC/G++
make mac           # build/Fastrock.app on macOS
make check         # vet and headless tests, no GUI launched
```

On Windows, `powershell -ExecutionPolicy Bypass -File dev/build.ps1` produces
`build/fastrock.exe`. Run it from a terminal whose PATH includes Codex, or arrange
that PATH for your desktop login. The `make mac` development bundle is unsigned; release packaging signs and notarizes it. Local `make` builds report a `dev-<checkout>` version (including a dirty marker). The development macOS bundle uses numeric version `0.0.0`; release packaging fills both bundle version fields with the validated release version.
No GUI tests are run on macOS.

## Updates and installation

Release builds check GitHub automatically and download verified updates in the
background. **Help → Check for updates** checks manually. Close all Fastrock
windows to install a ready update and restart. Windows uses a per-user installation
and automatically creates a Start menu shortcut; administrator access is never
required. macOS releases ship as a drag-to-Applications DMG. Development builds
remain portable. See [release packaging and update behavior](docs/UPDATES.md).

## Codex conversations

Choose a project folder and start a conversation. Existing CLI/app-server history
is loaded through Codex. Use the horizontal tabs to switch conversations, files,
Rally pages and Settings; Ctrl/Cmd+T opens a new document tab. The sidebar contains
only conversations. Streaming responses, expandable reasoning/tool output,
image attachments, plan mode, queue/edit/delete, steer, interrupt, resume, fork,
rename, archive, structured review targets and Markdown export are connected to app-server. Conversation history pages load as you approach the end of the sidebar.

Fastrock inherits the installed Codex environment, account, configuration,
provider, model catalog, MCP servers and skills. It has no second API key or model
provider configuration. Model, reasoning effort and speed selectors use the
app-server catalog. **GPT-6.1-Sol Ultrafast with AWS Bedrock** is available when the
installed Codex and its provider advertise that tier. If they do not, Fastrock
explains the mismatch and keeps other available configurations usable. It never
silently substitutes a provider or speed. Settings supports account sign-in, provider setup, MCP/skills/plugins/hooks,
keyboard bindings, memory controls and Common/Codex configuration through its API. Local model discovery
and downloads use the configured local server; all inference still goes through
Codex. Restart Codex from Settings to apply provider changes across open windows.

## Rally

In Settings → Rally, enter the server endpoint and API token, then select a
workspace and project. Tokens use Windows Credential Manager or macOS Keychain,
not settings.json. Normal servers require HTTPS; HTTP is allowed only on loopback
for local fixtures. Parent/child project scope, iteration/release, search, WSAPI
filters, column selection, grouping and private saved views control the data.

Open Team Board, Backlog, User Stories, Iteration Status, Tasks, Defects, quality,
portfolio, planning, timeline or report pages from Rally navigation or the command
palette. Boards and tables display real WSAPI results. Charts compute partial counts and
estimates from the loaded result window; they do not invent historical metrics.
Work-item details support creation, editing, deletion, comments, tasks, children,
attachments, revision history and schema-driven fields, including `c_*` fields.
The Team Board has unified cards with owners, iterations, estimates and ready/blocked
status. Drag a card to a state column to move it after checking its current revision.
Search matches IDs, names, owners and readable descriptions; quick filters include
owner, state, blocked and ready. Boards scroll horizontally on narrower windows.

Story and task details use a wide content editor with a properties sidebar. Description,
notes, acceptance criteria, custom HTML fields and discussions support native bold,
italic, underline, strike, headings, lists, links, preview, HTML source and undo/redo.
Opening an item preserves its original HTML exactly. Editing normalizes supported
formatting; use HTML source for complex markup such as embedded media or tables.
CSV export quotes spreadsheet formula cells. Deletions show a confirmation.

**Ask AI** opens the Rally assistant with the same Codex provider and model catalog.
It can query work, explain results, filter/group native views and propose batches
of changes. Proposed writes are previewed for an explicit Apply action. The apply
path rechecks reviewed revision timestamps and stops on the first failure; WSAPI
does not offer an atomic multi-item transaction. Rally fields are treated as data,
not instructions, and API tokens are not placed in model prompts.

## Persistence and diagnostics

Preferences, private views, open tabs and unsent conversation drafts/queues are
stored under the platform's user configuration directory in `fastrock/`.
Conversation history remains owned by Codex. An interrupted queued prompt is
retained and is not automatically submitted on application startup.

Useful environment variables for isolated testing:

- `FASTROCK_HOME`: alternate preferences/session directory.
- `FASTROCK_RALLY_TOKEN`: token override for fixtures or a secret-injecting launcher.
- `CODEX_HOME`: standard Codex setting, inherited without modification.
- `FASTROCK_CODEX_INTEGRATION=1`: enable the installed-CLI integration test.
- `FASTROCK_AUTOMATION`: Windows-only local UI smoke scenario JSON; requires a build with `-tags=fastrock_automation`.

```sh
go run ./cmd/mock-rally       # loopback :18081, token mock-token
# In another terminal:
go run ./cmd/mock-model       # loopback :18080, deterministic Responses API
FASTROCK_CODEX_INTEGRATION=1 go test -run TestInstalledCodexRallyRoundTrip ./internal/codex
```

The integration test creates a temporary Codex home with the mock provider; it
does not modify your normal Codex configuration or send requests to a paid model.
The mock Rally fixture follows the existing local fastrally test patterns and
contains only fictional work. `dev/windows-smoke.ps1` runs the native UI against
both fixtures and writes screenshots and assertions to its designated test folder.
Install a complete Codex distribution in that folder or on PATH; its code-mode
helper must accompany the executable.

See [architecture and limits](docs/ARCHITECTURE.md) and
[validation](docs/VALIDATION.md). Apache-2.0; dependency notices are in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Right-click a document tab to **Pop out into new window** or **Move to another
window**. Windows share one installed Codex app-server. The source tab closes
only after the destination acknowledges its state; drafts and rich formatting
move with it. Closing the original window leaves other windows running.

Rally boards load pages as the viewport needs them. There is no story-count
setting. Refresh remains available above board and detail views. The working
set retains at most 2,048 lightweight cards per view and 24 MiB of cached API
pages; idle views and transcripts are evicted under memory pressure. Fastrock
measures aggregate native process memory across its windows separately from Codex: physical footprint on macOS and private committed bytes on Windows.

See [interaction coverage](docs/INTERACTIONS.md) for the source audit and remaining parity work.


Review status: [the 540-item assessment](docs/REVIEW-ASSESSMENT.md) records fixes,
disagreements and remaining work. Rally charts/planning summarize only loaded
items. Board lanes use scoped workflow metadata; some named Home/report pages
still share generic content. Native Windows interaction/DPI testing of this
review patch remains outstanding; headless checks do not establish GUI parity.
