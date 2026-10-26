# Virtual files with Goose and vwork

Fastrock can browse and edit virtual workspaces in a running [vwork](https://github.com/allquixotic/vwork)
0.2+ store. It does not link redb, open the database, mount a filesystem, create
symlinks, or export a checkout. The existing vwork daemon remains the database
owner. All changes go through its authenticated local API and revision checks.

## Enable it

1. Install/build vwork, register your Git repository, and create a workspace
   using vwork's setup instructions.
2. Start the daemon with `vwork --home <absolute-state-directory> start`.
3. Generate and merge vwork's Goose configuration fragment into your existing
   Goose config. Keep your provider/model settings. A minimal extension looks
   like this (use your actual **absolute** paths):

   ```yaml
   extensions:
     vwork:
       enabled: true
       type: stdio
       cmd: 'C:\Tools\vwork\vwork.exe'
       args:
         - --home
         - 'C:\Users\you\AppData\Local\vwork'
         - mcp
         - --workspace
         - task-one
   ```

4. Launch Fastrock. Its toolbar gains a **Virtual** button, which opens a native
   browser/editor window. Discovery runs
   at startup and every ten seconds. **Refresh** retries immediately.

There is no Fastrock feature toggle to maintain. Without an enabled installed
vwork MCP extension in Goose config, the button and all virtual-file controls are
absent. Disabling the extension removes the button and hides the window at the next refresh. The Goose
process itself need not be running. Fastrock never launches commands from Goose
configuration, changes that configuration, or starts the daemon for you.

### Configuration lookup

Lookup uses the first applicable rule:

1. `FASTROCK_GOOSE_CONFIG`: an absolute path to a specific `config.yaml`.
2. `GOOSE_PATH_ROOT`: `<absolute-root>/config/config.yaml`.
3. Platform defaults:
   - Windows: `%APPDATA%\Block\goose\config\config.yaml`.
   - macOS: `~/Library/Application Support/Block.goose/config.yaml` and
     `$XDG_CONFIG_HOME/goose/config.yaml` (default `~/.config/goose/config.yaml`).
   - Linux: `$XDG_CONFIG_HOME/goose/config.yaml` (default `~/.config/goose/config.yaml`).

Use `FASTROCK_GOOSE_CONFIG` when you launch Goose with a custom config location
or want Fastrock to follow one particular profile. Environment overrides must
be set in the environment that launches Fastrock; a GUI app may not inherit
your terminal's shell configuration.

Only enabled `stdio` entries with an installed executable named `vwork` or
`vwork.exe` and the `mcp` argument qualify. Absolute executable paths are best;
bare names resolve through Goose's configured `GOOSE_SEARCH_PATHS`, the
environment's `GOOSE_SEARCH_PATHS`, and `PATH`. Shell wrappers are deliberately
not interpreted. The `--home PATH` and `--home=PATH` forms select a store;
without them the standard vwork data directory is used. Home paths must be
absolute (`~/...` is supported). Entries pointing at the same store are merged.

Multiple qualifying extensions can expose multiple stores. Every registered
repository and workspace within those stores appears in the catalog, including
workspaces other than the extension's default `--workspace`. vwork's workspace
binding is a default, not an access boundary. Repositories still use their
existing Git-backed base data; keep the registered source repositories present.

### Agent tools and global instructions

The vwork setup deliberately disables Goose's Developer and Extension Manager
extensions. Keep them disabled when routing source operations through vwork.
An agent recommending Developer merely because it cannot run a shell has missed
this configuration. vwork supplies virtual file operations and configured
build/package tasks; it does not supply an arbitrary host shell or access to
host files such as `%USERPROFILE%\.claude\CLAUDE.md`.

Skill discovery, instruction loading, and tool access are separate. Discovering
skills under `.claude/skills` does not import `.claude/CLAUDE.md`, make Claude Code
hooks/subagents available, or grant access to other physical files.

Goose **1.56.0 loads hints independently of Developer**. To supply global rules,
manually merge the desired instruction text into `AGENTS.md` beside the active
Goose `config.yaml`, preserving existing rules. The default Windows location is
`%APPDATA%\Block\goose\config\AGENTS.md`; with `GOOSE_PATH_ROOT`, use
`<GOOSE_PATH_ROOT>\config\AGENTS.md`. Its default context filenames include
`AGENTS.md` and `.goosehints`; retain `AGENTS.md` if overriding
`CONTEXT_FILE_NAMES`. Start a new conversation after setup. This supplies context
without adding a host filesystem tool. Adapt shell/Git instructions to vwork and
omit requirements that depend on unavailable Claude-specific hooks or agents.

To distinguish working vwork tools from a missing extension, ask the agent to
call `vwork_workspace_info` and `vwork_files_list` for its bound workspace. Goose
may prefix these names, for example `vwork__vwork_workspace_info`. Successful
calls establish virtual file access, not host file access. If the tools are
absent or fail, inspect the enabled vwork extension, daemon, and workspace binding;
do not treat enabling Developer as the repair. The Virtual window discovers
configuration and talks to the daemon separately, so its presence alone does not
prove that a particular Goose conversation loaded the MCP tools.

Verified against [Goose 1.56.0 prompt construction](https://github.com/aaif-goose/goose/blob/v1.56.0/crates/goose/src/agents/prompt_manager.rs)
and [hint lookup](https://github.com/aaif-goose/goose/blob/v1.56.0/crates/goose/src/hints/load_hints.rs).

## Browse and edit

- Filter the catalog by store, repository, or workspace name. At most 200
  matching entries are shown; narrow the filter for a larger catalog.
- Select a workspace. A repository's **(new workspace)** entry lets you create
  a workspace from that repository's current HEAD. **Fork workspace** copies
  the selected virtual workspace using vwork's sharing semantics.
- Enter a relative path prefix such as `src` and choose **Browse**. **Previous**
  and **Next** page through up to 200 file paths at a time. Select a path to open
  it directly from vwork.
- Edit and **Save**. Files are read in binary mode then decoded as UTF-8, keeping
  BOMs, CRLF, final newlines and executable modes. Saves send exact editor bytes
  and the loaded workspace revision. No line-based preview is used for editing.
- **Discard** restores the last loaded/saved text. **Reload** reads the current
  version after you have saved or discarded local edits.
- **Workspace diff** shows saved changes against the Git base. **Logical size**
  shows vwork's logical-size report. Neither operation materializes files. Diff
  does not include edits that are still only in the editor buffer.

The editor accepts UTF-8 text up to 2 MiB. Binary files, symlinks, submodules and
larger files remain visible in listings but cannot be edited here. List entries
are files, not an Explorer-style directory tree. Reports are bounded at 64,000
characters. This pane edits existing files; use vwork's tools for adding,
renaming/deleting files, merging, commits, exports and build/package operations.

If an agent changes any file in the workspace after you opened yours, Save
reports a revision conflict and **keeps your text**. Copy it somewhere safe if
needed, Discard, Reload, reconcile against the latest contents, and save again.
The interface deliberately offers no force-overwrite option. While a request
is running, conflicting controls are disabled. Navigation to another file or
workspace requires Save or Discard first. Each daemon request times out after
20 seconds; errors appear in the pane, and Refresh reconnects after a restart.

**Unsaved virtual drafts are in memory only. Save before closing Fastrock.**
Disabling the provider hides its controls but retains the current draft during
that app session. Re-enabling the same store makes it accessible again. Editing
virtual files does not change Fastrock's existing local editor, local project
folders, or the tools available to Fastrock's own agent conversations.

## Storage-neutral integration

`fastrock-workspaces` defines the `WorkspaceProvider` trait and typed
`Request`/`Reply` values for catalog, listing, reading, writing, create/fork,
diff and size. `vwork::VworkProvider` adapts these operations to the daemon API.
The UI uses a separate buffer and asynchronous work/events; virtual paths never
enter the local editor's filesystem save path. Another backend can implement
this interface without depending on redb or changing the editor's file model.

The adapter reads `connection.json` for each request so daemon restarts can
change the port and token. It accepts only `http://127.0.0.1:<port>/`, bypasses
proxies, rejects redirects, limits responses to 4 MiB, and never puts the token
in UI state, persistence, or diagnostics. Goose config reads are capped at
1 MiB. Missing or malformed configuration fails closed (no provider UI).

## Verification

Normal tests use fake config files and a local fake HTTP daemon; no AWS or LLM
account is needed. The browser stress fixture uses 100,000 catalog entries and
a million-file paginated result without building a million GUI rows.
Native Slint controls are compiled with the application. Browser state and HTTP
fixtures execute on Windows CI along with the existing Goose/Rally acceptance
checks. No GUI or tests run on the Mac in this release workflow.

```sh
cargo test --workspace
```

An optional test exercises a real, separately built vwork binary against a
temporary Git repository. It creates and destroys only its own test daemon and
state directory, and verifies the original working files remain unchanged:

```sh
FASTROCK_TEST_VWORK=/absolute/path/to/vwork \
  cargo test -p fastrock-workspaces --test live_vwork -- --ignored
```

PowerShell equivalent:

```powershell
$env:FASTROCK_TEST_VWORK = 'C:\Tools\vwork\vwork.exe'
cargo test -p fastrock-workspaces --test live_vwork -- --ignored
```

The provider adapter was originally verified against a separately built vwork
0.2 daemon using the opt-in round trip. The current application retains the
Goose ACP, Codex and Rally architecture; virtual files have their own editor
buffer and do not enter local filesystem save paths.
