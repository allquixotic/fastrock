# Using Fastrock

Install Codex CLI 0.162.0 or newer and launch Fastrock. Codex configuration and
credentials belong to the installed CLI; Fastrock starts its app-server over
stdio. Settings → Connection also supports an explicitly selected daemon/remote
server. The imported conversation controls are documented in
[the upstream guide](upstream/user-guide.md); its embedded-backend and packaging
sections have been superseded by Fastrock's README and release instructions.

Open Rally pages from the Rally menu. Configure Rally Settings inside the document:
endpoint, workspace, team/project scope and API token. The token is stored in the
OS credential vault. Team/Parents/Children apply to all subsequent queries. Rally
pages never appear in the conversation sidebar.

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

Ask AI opens the Rally assistant dock using the same external Codex model catalog.
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
