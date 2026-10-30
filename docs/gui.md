# Using Fastrock

The 0.4.0 development build embeds Goose and vwork; no separate installation is
needed. Existing Goose settings and OS-vault credentials remain in place. Expand
**Goose options** above Send for the model, mode and effort choices Goose exposes.
Use **Use model ID** for an exact model or Bedrock inference-profile ID absent
from the catalog. **Save effort default** remembers the current effort for that
provider and model, including named profiles. New chats and explicit model or
provider switches use it when Goose offers that value. Restored chats and forks
retain their own settings. **Clear effort default** affects future selections;
it leaves the current chat's effort unchanged.
See [runtime status and current limitations](goose-acp.md). Settings → Goose
provides provider defaults, named direct-provider profiles with OS-vault keys,
and the cheap-model selection for search summaries.

Open **Command Palette** from the sidebar or View menu, or press
**Ctrl+Shift+P** (**Cmd+Shift+P** on Mac). Search commands, registered projects,
and loaded conversations; prefix `>` to show commands only. Use arrows and Enter,
or click a result. Escape closes the palette without interrupting active work.
The keyboard binding can be changed in Settings → Keyboard.

Open **Terminal** in the conversation header, View menu, command palette, or
with **Ctrl+`** (**Cmd+`** on Mac). Each conversation can have multiple native
terminal tabs. Drag the top divider to resize; wheel through the last 5,000 rows.
Select cells to copy, or use **Copy** for the visible screen. **Paste** supports
bracketed paste when the foreground program requests it. Keyboard copy/paste is
Ctrl+Shift+C/V on Windows and Cmd+C/V on Mac; Ctrl+C goes to the shell. Escape
inside the terminal does not interrupt the model. **Hide** keeps the shell running.

A conversation bound to vwork opens an isolated, renewable temporary copy.
Enter relative source paths, one per line, then **Import paths and close** to stop
the shell and import those files with a revision check. Concurrent edits reject
the import. Closing preserves other source edits in a vwork recovery workspace,
shown in the terminal status. Ordinary folder conversations run in their local
folder. Closing the conversation or explicitly quitting stops owned shell
processes; closing the last window to the tray keeps them running. Restart never
replays a shell or command automatically. Windows terminals start `cmd.exe /D /Q`;
PowerShell and other interactive tools can be launched from it.

Open **Checkpoints** in the conversation header or command palette. Fastrock
captures the conversation boundary and virtual source revision before and after
completed turns. **Save checkpoint** adds a named manual snapshot. Select a row
to inspect file changes and the message count; the displayed diff runs from the
checkpoint to the current files, so restoring reverses it. Choose **Files**,
**Conversation**, or both, then **Restore selected**. The original repository
checkout remains unchanged. Restore first saves a retained backup that can also
be selected later.

File restore requires an isolated workspace, no active terminal/temporary lease,
and a complete current review. Large or stale reviews cannot restore files.
Conversation rewind checks exact message content and boundaries; if compaction
changed that history, the panel explains why conversation restore is unavailable.
If a restore is interrupted, reopen Checkpoints to **Restore retained backup**
or **Keep reviewed state**. Queued work stays paused until Resume. Saved tool
calls with uncertain outcomes open as read-only history; restore a completed
checkpoint or fork before the interrupted turn to continue without replaying them.

Conversations appear in the sidebar. Click the active title in the header for
rename, fork, compact, export and other conversation actions. **Open tabs** also
reaches open child conversations and files. The sidebar footer opens settings;
search uses current compressed summaries and never sends your query to a model.

Right-click a repository or conversation for **Project settings / New chat on
branch…**, or use **Project settings** in the command palette. Save a default
branch, tag, `HEAD`, or full commit ID for future chats. **New chat** uses the
reference currently entered without changing the saved default. References come
from the registered repository snapshot; each chat gets its own compressed
workspace. Existing conversations and your source checkout stay intact. Conflicting
saves from another window require Reload.

The sidebar also offers **Show only this project**, **Copy repository path**,
**Copy starting Git reference**, and **Copy conversation**. The latter retrieves
complete history in the background, with a 100,000-item / 64 MiB export limit.
Older workspaces without a recorded starting reference use their base commit ID.
**Changes** in the header, or `/diff`, opens a local native diff without asking a
model to review it. Binary-only comparisons open as text with their limitations.

`/model` and `/permissions` expand Goose options. `/mcp`, `/plugins`, and `/hooks`
open Tools & hooks. `/new [name]` creates an isolated chat in the same repository.
`/ps` shows existing conversation terminals without starting a shell; `/stop`
confirms closing those terminals and retains unimported edits for recovery.
Commands requiring unavailable Codex capabilities are excluded from suggestions;
manually entering one keeps the draft and explains the limitation.

The permanent **Rally** sidebar project opens the last Rally document or a new
Team Board. Its horizontal strip contains only Rally documents, each independently
closable. The project stays available with no open documents and when the Goose
connection is unavailable. **Close other tabs** applies only to the current area;
finish writes and handle unsaved Rally documents individually before bulk closing.

Open additional pages from the Rally menu. Configure Rally Settings inside the
document: endpoint, workspace, team/project scope and API token. The token is
stored in the OS credential vault. Team/Parents/Children apply to subsequent
queries. Rally documents have no binding to an ordinary conversation.

Use the six section buttons, page selector, private views, iteration/FY Quarter,
search, quick filters or the advanced query to choose work. Each document keeps its
own view and drafts. The minus buttons hide control rows without changing their
values; the restore strip brings them back. Switch among list, board, planning,
metrics, charts and timeline with the native view controls.

Click an artifact ID or Open to see its details. Text, number, date, checkbox,
choice and reference controls come from Rally schema metadata. Rich fields offer
native selection/clipboard, formatting, HTML source and styled preview. Unchanged
HTML stays byte-for-byte intact. Save explicitly writes validated changes. Reload
merges remote changes and shows overlapping field conflicts; resolve each before
saving. Related tabs load discussions, attachments, tasks and other collections
only when the artifact supports them.

Board moves use workflow metadata and relative ranking; Undo checks the latest
revision. Bulk editing offers common writable fields, a review and per-item
outcomes. Writes are never automatically retried. CSV exports retrieve the whole
applied query with spreadsheet-formula neutralization; visual summaries cover
only the loaded records and say so.

Ask AI opens an explicit temporary Goose session in the Rally assistant dock.
Its typed tools read scoped data and prepare changes. Review the old/new values,
select the intended changes, then explicitly Apply. A partial failure stops the
batch and preserves the exact outcomes. Descriptions returned by Rally are data,
not instructions. No token enters the assistant prompt or transcript.

Preferences and document drafts are saved atomically. On first migration the Go
Fastrock preferences and Rally session are read; new settings take precedence on
later runs. Existing OS-vault token keys remain compatible.

Ctrl/Cmd+F reveals and focuses search; slash focuses search when a text editor has
not consumed the key. Alt+B/L switches Board/List, Alt+N creates, Alt+R refreshes,
and Escape returns from details with Save/Discard/Cancel when needed. Native
Slint controls supply focus traversal, clipboard and accessibility.

New window moves the active Rally document to a separate native window. The
window selector transfers it to an existing window in the same Rally scope.
Drafts, selections, proposals, unapplied filters and scroll positions travel with
the document; pending writes prevent transfer. Transfers use persisted ready,
commit and acknowledgement states. Retired windows retain their drafts for
recovery at the next main-window start.

## Goose tools and hooks

Settings → Tools & hooks lists Goose extension defaults and open conversations.
Enable or disable extension defaults for new chats, or add a command/HTTP MCP
extension. New extensions start disabled. Credential fields accept names of
existing Goose vault/environment entries; the editor never writes secret values.
Unknown configuration fields and existing credentials are preserved.

Conversation tools shows the selected chat's active extensions and tool catalog.
Code mode uses Goose's in-process JavaScript tool composition. Removing an
extension affects only the idle selected chat; tool permission choices are Goose
preferences shared across chats. Fastrock's workspace bindings remain managed.

Hooks lists user/project plugins and their effective activity. Create a user hook
plugin directly in the page; it starts disabled. Edit a plugin's
hooks.json in the native editor, then Save or Discard. Save validates the bounded
hook definitions, rejects stale edits and retains the original as a backup.
Defaults and hooks apply to newly activated sessions. The bundled vwork policy is
protected. Malformed Goose configuration is preserved and must be repaired before
native settings can overwrite it.

### Goose recipes

Settings → Recipes lists the existing Goose recipe library. Select a recipe to
edit its YAML/JSON, or create a new recipe. Save validates against Goose's schema;
editing an existing recipe retains its original bytes in a backup alongside it.
Reload instead of overwriting a document changed by another editor. Deletion
requires its own confirmation and also retains a backup.

A saved recipe exposes its declared parameters, including selection, numeric,
date, boolean and file inputs. Choose a registered repository and use **Prepare
recipe conversation** to create an isolated vwork workspace. The prepared prompt
appears in the composer; review it and Send to begin. Model/provider settings
come from the recipe, within Fastrock's supported provider scope. Recipe extension
lists retain Fastrock's embedded workspace and coordination tools. Existing
conversations keep their stored recipe snapshot when the library file changes.

Goose's ACP API renders supplied recipes again. Fastrock checks that this second
render leaves the prepared recipe unchanged; parameter text that would introduce
another template expansion is rejected before providers or extensions activate.
