# Fastrock

A native Go/nucular desktop workspace for Codex conversations and Rally work.
Windows 11 x64 is the primary target; macOS builds use the same application code.
Dark mode is the default. Light mode and font size are in Settings.

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
error with an Exit button. `fastrock --doctor` checks the CLI without opening a GUI.

Use the pinned **Go 1.26.0** toolchain through the build scripts. Go 1.27.2 produced
blank Windows windows during validation. The local nucular fork selects its
software renderer on Windows; macOS uses Gio. See [the fork notes](third_party/README.md).

```sh
make windows       # build/fastrock.exe, Windows x64 cross-build
make mac           # build/Fastrock.app on macOS
make check         # vet and race tests, no GUI launched
```

On Windows, `powershell -ExecutionPolicy Bypass -File dev/build.ps1` produces
`build/fastrock.exe`. Run it from a terminal whose PATH includes Codex, or arrange
that PATH for your desktop login. macOS compilation requires Xcode Command Line
Tools. The generated app bundle is unsigned. No GUI tests are run on macOS.

## Codex conversations

Choose a project folder and start a conversation. Existing CLI/app-server history
is loaded through Codex. Use the horizontal tabs to switch conversations, files,
Rally pages and Settings; Ctrl/Cmd+T opens a new document tab. The sidebar contains
only conversations. Streaming responses, expandable reasoning/tool output,
image attachments, plan mode, queue/edit/delete, steer, interrupt, resume, fork,
rename, archive and Markdown export are connected to app-server.

Fastrock inherits the installed Codex environment, account, configuration,
provider, model catalog, MCP servers and skills. It has no second API key or model
provider configuration. Model, reasoning effort and speed selectors use the
app-server catalog. **GPT-6.1-Sol Ultrafast with AWS Bedrock** is available when the
installed Codex and its provider advertise that tier. If they do not, Fastrock
explains the mismatch and keeps other available configurations usable. It never
silently substitutes a provider or speed. Settings can inspect account/MCP/skills
and read or edit Codex's own configuration through its API.

## Rally

In Settings → Rally, enter the server endpoint and API token, then select a
workspace and project. Tokens use Windows Credential Manager or macOS Keychain,
not settings.json. Normal servers require HTTPS; HTTP is allowed only on loopback
for local fixtures. Parent/child project scope, iteration/release, search, WSAPI
filters, column selection, grouping and private saved views control the data.

Open Team Board, Backlog, User Stories, Iteration Status, Tasks, Defects, quality,
portfolio, planning, timeline or report pages from Rally navigation or the command
palette. Boards and tables display real WSAPI results. Charts compute counts and
estimates from the current selection; they do not invent historical metrics.
Work-item details support creation, editing, deletion, comments, tasks, children,
attachments, revision history and schema-driven fields, including `c_*` fields.
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
- `FASTROCK_AUTOMATION`: Windows-only local UI smoke scenario JSON.

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
