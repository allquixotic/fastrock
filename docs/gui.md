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

Conversations appear in the sidebar. Click the active title in the header for
rename, fork, compact, export and other conversation actions. **Open tabs** also
reaches open child conversations and files. The sidebar footer opens settings;
search uses current compressed summaries and never sends your query to a model.

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
