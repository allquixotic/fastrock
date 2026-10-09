# Architecture

`cmd/fastrock` loads preferences and starts the native shell. Production packages
under `internal` are separated from the deterministic mock servers in
`cmd/mock-rally`, `cmd/mock-model`, `internal/mockrally` and `internal/mockmodel`.
No fixture records are loaded by the application.

- `codex`: lifecycle, newline JSON-RPC, request cancellation, events, capability
  catalog and service tiers. An authenticated loopback broker multiplexes native
  windows over one installed `codex app-server`. A server restart keeps window
  connections and unsent drafts; active turns stop.
- `rally`: direct WSAPI v2.0 authentication, scopes, pagination, metadata, CRUD and
  attachments. Absolute references must stay on the configured origin and under
  its WSAPI path. Redirects are disabled to keep tokens on that origin. GET retries
  are bounded and cancellable; mutation requests are never automatically retried.
- `assistant`: typed read/view/proposal tools and revision-checked application.
- `workspace`: document tabs, conversation state and independent prompt queues.
- `settings`: atomic JSON preferences/session writes and the OS credential vault.
- `platform`: file/folder dialogs, URL opening and Windows screenshot capture.
- `richtext`: Unicode document/format spans, HTML parsing/serialization and undo history.
- `ui`: nucular presentation. Network work runs in goroutines and posts results
  to the UI loop; presentation state is owned by that loop.

The shell keeps the Codex GUI's top document strip, conversation sidebar, chat
workspace, details panel and Settings navigation. Rally uses a separate native
navigation/toolbar within its document. The layouts share theme tokens and fonts.
The original browser app is a design reference, not an embedded runtime.

The vendored nucular software renderer presents RGBA frames through a small
Ebitengine 2.10 adapter. Native input is translated into nucular events; frames are uploaded only when
changed, and idle windows use event-driven presentation. Both desktop
platforms compile without CGo. File dialogs, clipboard and screenshots use system
APIs directly; macOS file selection retains the system AppleScript dialog.

The board caches projections, search text, group/column membership and wrapped card
titles. Changes to the data generation, filter, grouping, sort or width invalidate the
relevant caches. Sidebar grouping is cached separately from live conversation status.
Font measurement uses a bounded typed cache, and editor UTF-8 snapshots are reused.

## Deliberate limits

This rewrite implements WSAPI-backed boards, lists, current-scope charts,
planning summaries, date timelines and artifact details. It does not emulate
Rally enterprise administration, historical Lookback analytics, app-catalog
plugins, or every server-specific custom page. These need APIs and permissions
beyond the supplied core WSAPI workflows. Current-scope charts are labeled as
such. Timelines display planned dates; they are not historical burndown charts.

The Codex shell exposes app-server conversations, approvals and configuration.
Native Settings support account sign-in, Bedrock setup, local model discovery
and downloads, MCP configuration/OAuth, skills, plugins, hooks, features, memories,
imports, feedback, sandbox setup, keyboard bindings and version-checked config
edits. The installed CLI owns accounts and inference configuration. Local files
are read-only viewers with bounded paging.

The source-based interaction audit and remaining differences are recorded in
[INTERACTIONS.md](INTERACTIONS.md). Full widget-for-widget parity is not yet
complete; the audit distinguishes implemented behavior from unfinished details.

Rally writes enforce origin checks and reviewed revisions but cannot provide
server-side transactional compare-and-swap across artifacts. A concurrent edit
between the final GET and POST is still possible with WSAPI. Batch results retain
the number of completed changes and require a fresh review after a failure.

## Window and memory ownership

Each native window uses its own Ebitengine process. A 256-bit environment-only
secret authenticates the loopback connection. Transfers use expiring tickets:
reserve, initial snapshot, rendered readiness, final snapshot, destination applied
acknowledgement, then source removal. Cancellation retains the source document.
Stable document identities keep separate drafts when both windows show the same
Rally page. Server notifications carry sequence numbers; approvals route to the
owning window. The original process hosts services until the last window closes.

Open-chat metadata is published to the broker only when it changes. Cross-window
messages retain human delivery approval; only bounded matching replies are observed
by the sender. Mailboxes move with their conversation tab.

Rally queries use 128-record lightweight pages, four concurrent requests per client,
a 32-page/24 MiB LRU with a 45-second TTL, and a 2,048-card resident window per view.
Full descriptions load on demand. Filter changes cancel/supersede old requests;
mutations invalidate the cache, including pre-mutation requests still in flight.
Active views refresh after 60 seconds, with exponential failure backoff. Manual
Refresh always remains visible. Inactive pages do not poll.

The UI drains at most 64 posted callbacks or 4 ms per frame. JSON decoding,
network/filesystem/process work, transcript layout and preference/session writes
run in workers. CommonMark/GFM is parsed into compact styled runs with native
links, tables and direct selection; neither HTML nor remote resources execute.
The transcript layout queue has two workers and 32 slots. RSS is
sampled every 10 seconds across Fastrock processes. A 384 MiB aggregate pressure
threshold evicts inactive reconstructible state; a 256 MiB Go soft heap budget is
shared across windows. Unsaved edits, pending approvals and queues are retained.
Real files page up to 4 MiB; long transcripts and render caches have separate bounds.
RSS and private committed memory differ; these targets are not hard process limits.
