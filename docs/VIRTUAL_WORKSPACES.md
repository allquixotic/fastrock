# Embedded workspaces in Fastrock 0.4.0

The development branch links unmodified vwork 0.3.4 at
`c53cea4879bc0e12735ae1c89bd7d5ca3e9b1910`. Existing stores use the same Engine,
redb schema, compression settings, revisions and leases. Fastrock holds one owner
per canonical store path. Workspace I/O runs on bounded blocking workers;
compaction excludes active readers and writers. No vwork installation or daemon
is required. Fresh installations receive a default local store.

Current search summaries have a separate compressed table inside that same redb.
Replacement is atomic and checks the requested turn revision; stale background
results cannot overwrite a newer request. Background generation and deterministic
search are integrated. The hook helper uses the original vwork policy.

Existing enabled `vwork mcp --home <absolute-path> --workspace <id>` entries map
to embedded MCP handlers. Unsupported environment overrides, custom working
directories, tool filters or CLI forms fail explicitly instead of launching a
standalone core service. Keep the old files intact when diagnosing a migration.

The native Virtual window can explicitly integrate a fork: select the target
workspace, enter the source workspace name, choose **Review incoming changes**,
then **Apply reviewed merge**. The review shows source and target changes since
their shared ancestor, including file modes. Apply uses vwork's original merge
under the exclusive embedded owner lock and checks both reviewed revisions.
Changed revisions require another review; conflicts leave both workspaces
unchanged. The source workspace and original filesystem checkout are preserved.
Draft edits invalidate review. Incomplete previews disable Apply; the current
view bounds patches to 128 KiB and fewer than 200 changed paths per side.

Conversation-to-workspace binding and automatic isolated delegation are still
in development. The reviewed merge UI is newer than the alpha.1 release.

The sections below document historical 0.3.x behavior only.

---

# Virtual files with Goose and vwork

Fastrock can browse and edit virtual workspaces in a running [vwork](https://github.com/allquixotic/vwork)
0.2+ store. It does not link redb, open the database, mount a filesystem, create
symlinks, or export a checkout. The existing vwork daemon remains the database
owner. All changes go through its authenticated local API and revision checks.

## Enable it

1. Install/build vwork, register your Git repository, and create a workspace
   using vwork's setup instructions.
2. Start the daemon with `vwork --home <absolute-state-directory> start` for
   initial setup. With vwork 0.2.2+, Goose also starts/reuses it automatically
   when loading the enabled MCP extension, from either Fastrock or the Goose CLI.
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
process itself need not be running. Opening only the Virtual window requires a
running daemon.

With Fastrock 0.3.3, vwork 0.3.3 and Goose HTTP MCP support, ordinary vwork
extensions use the shared daemon directly. Opening or loading a conversation
creates no vwork client process. Fastrock starts the installed daemon once when
needed; `--no-auto-start` opts out. Closing a chat leaves the owner running.
Goose outside Fastrock retains its normal stdio configuration. Fastrock does not
rewrite that configuration, install backends, or embed a second agent runtime.

Both HTTP hops bind **127.0.0.1 only**. The Fastrock bridge checks the exact Host,
rejects any Origin header, and requires an unguessable route. The daemon checks
its existing bearer authentication and exact Host and rejects Origin headers.
Forwarding bypasses proxies and redirects. Per-request workspace bindings keep
concurrent conversations independent. Nonsecret session sidecars restore the
original binding and a fresh local route after Fastrock restarts.

Older daemons, shell wrappers, custom working directories/environments, tool
filters and unknown CLI arguments retain Goose's stdio transport. Shared MCP is
never added to restricted Rally sessions. If workstation policy denies background
process creation, start the daemon through the permitted manual route.

vwork 0.3.2 enables Goose Developer alongside vwork and preserves other MCPs.
Its optional policy hook only redirects recognizable direct Git repository
creation; reads, host-file edits and general scripts remain available. Regenerate
setup and replace old vwork hints/hooks when upgrading from the strict policy.
Updating the executable alone does not rewrite installed Goose configuration.

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
binding is a default, not an access boundary. vwork 0.3+ imports all persistent Git objects and source into redb. Keep original
repositories until any legacy migration finishes; imported registrations no longer
depend on the original checkout.

### Agent tools and global instructions

With vwork 0.3.2+, enable Goose's Developer extension for arbitrary host reads,
host-file edits and one-off Bun/Python/shell scripts. Keep other desired MCPs,
including Exa and AWS, in the existing Goose configuration. Fastrock inherits that
configuration. No Fastrock binary update is needed for this policy change.

Repository work defaults to vwork. Use leased temporary copies for tools that need
source files, import wanted edits and release promptly. Intentional
`workspace_export` copies remain indefinite. Avoid unmanaged clones, worktrees
and bulk source copies. Reads of existing repositories, skills and AGENTS.md are
allowed; use vwork reads when you need the current virtual revision.

Regenerate vwork's setup with the installed executable. Merge only its Developer
and vwork entries into the existing extensions mapping, retaining other MCPs,
credentials and provider/model settings. Remove obsolete Developer tool allowlists.
Replace the old vwork section in global Goose hints with the new generated text;
remove stale instructions forbidding all shell or filesystem use. Replace the
installed vwork-policy plugin too, so the old blanket-denial hook does not survive
alongside the new policy. Start a new conversation afterward.

The optional hook detects direct Git clone/init/worktree-add mistakes; reads,
file edits and unrelated MCPs skip it. Shell reads are allowed, and hook failure
also allows calls. Arbitrary scripts can still write repositories: this is a
best-effort prompt-and-hook policy, not an OS sandbox or filesystem monitor.
Goose 1.56.0 still launches selected hooks through `sh`, which may flash a console
on Windows; vwork's own subprocess suppression cannot change that launcher.

Skill discovery, hint loading and tool availability are separate. Skills under
`.claude/skills` do not automatically import `.claude/CLAUDE.md` or implement
Claude hooks/subagents. Global Goose hints normally live beside `config.yaml`
in `%APPDATA%\Block\goose\config\AGENTS.md` on Windows, or
`<GOOSE_PATH_ROOT>\config\AGENTS.md` when overridden. Developer can read/edit
those host files. The vwork MCP, recipe and generated AGENTS.md share one policy.

For missing virtual tools, check the enabled vwork extension, daemon and workspace
binding using `vwork_workspace_info` / `vwork_files_list`. The Virtual window talks
to the daemon separately; its presence does not prove the chat loaded MCP tools.
See [vwork Goose setup](https://github.com/allquixotic/vwork/blob/main/docs/goose.md)
and [Goose 1.56.0 hooks](https://github.com/aaif-goose/goose/blob/v1.56.0/documentation/docs/guides/context-engineering/hooks.md).

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
proxies, rejects redirects, limits workspace responses to 4 MiB (MCP to 32 MiB), and never puts the token
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

## Compacting the vwork store

With vwork 0.3.1 or newer, open **File → Virtual workspaces…**, choose a database in
the **Database** selector, then **Storage → Compact entire database** or the
matching button. A single configured database is selected automatically. No
repository or workspace selection is required, even for an empty catalog. It
targets the entire redb, including every repository and workspace, preserves unsaved editor text, and reports completion, a threshold skip,
or an active-operation deferral. Filesystem/network work stays off the UI thread.

vwork runs GC and checks allocated-page statistics first. Both the button and
hourly automatic idle checks compact only above the configured threshold (default
1 GB of estimated unused space). Automatic checks additionally require more than
five minutes of input inactivity and no running vwork jobs. New requests receive
a busy response during compaction; the daemon's connection remains unchanged.
See [vwork maintenance](https://github.com/allquixotic/vwork/blob/v0.3.1/docs/operations.md#maintenance)
for idle settings, recovery snapshots and startup integrity validation.

To refresh imported Git history, Goose can use `vwork_repo_fetch` on the existing
registration. Fetch updates stored refs and keeps current workspaces pinned;
create a workspace from the updated branch to start on newer source.
