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
- `richtext`: Unicode documents with coalesced format spans, changed-range undo/redo and compact saved drafts. One history budget covers every editor in the process and is divided by the reported window count. Weak owners and explicit disposal release old history; the original HTML needs only a fixed-size content fingerprint for exact restoration after undo or manual reversion.
- `ui`: application presentation using the hybrid FLTK desktop layer. Reads, writes and urgent control operations use separate
  bounded queues and post results to the UI owner. Layout has its own bounded queue.

The shell keeps the Codex GUI's top document strip, conversation sidebar, chat
workspace, details panel and Settings navigation. Rally uses a separate native
navigation/toolbar within its document. The layouts share theme tokens and fonts. Narrow windows move side panels into reachable popups; exact reference geometry and native mixed-DPI behavior remain open.
The original browser app is a design reference, not an embedded runtime.

`internal/desktop` is the hybrid widget layer. Native FLTK buttons and plain
single-line fields are reconciled from value-only control snapshots; text edits
return through the ordered event queue with sequence numbers and an expected
base value, so delayed input cannot overwrite a restored draft. Custom controls,
formatted editors and the transcript use the preserved MIT-licensed layout and
software renderer derived from nucular. There is no nucular module dependency.

`internal/desktop/internal/fltkdriver` owns go-fltk windows on the main OS thread.
Layout, rasterization and application work remain on their existing owners.
Native callbacks exchange events with those owners instead of touching app state.
FLTK applies monitor scaling to logical coordinates. A coalescing awake callback
presents changed frames and reconciles native controls; unchanged controls are
reused. Custom raster surfaces retain the existing capacity and damage caches.
Both desktop platforms use CGo and link the pinned FLTK archives statically.
Windows also links GCC/libstdc++ statically; macOS uses system C++ frameworks.
Headless tests (`-tags=fltk_headless`) do not initialize FLTK or any display.
File dialogs, clipboard and screenshots continue to use their existing system
APIs; macOS file selection retains the system AppleScript dialog.

The board caches projections, search text, group/column membership and wrapped card
titles. Changes to the data generation, filter, grouping, sort or width invalidate the
relevant caches. Sidebar grouping is cached separately from live conversation status.
Font measurement uses a bounded typed cache with short read/write critical sections;
cache misses measure outside the global lock. Individual faces synchronize glyph state,
and transcript workers reuse pooled faces. Native editor mutations advance text
revisions and report changed ranges. Rich editors apply those ranges once;
unchanged UTF-8 snapshots and document checkpoints are reused without rescanning
or copying their text.
Unopened rich fields do not allocate native/source editor buffers or paint
copies. Rich fields also no longer retain a second plain-text HTML editor.
Board virtualization includes both groups and cards; detail collections use explicit
128-record pages with range/total and retry behavior.

## Deliberate limits

This rewrite implements WSAPI-backed boards, lists, loaded-window charts,
planning summaries, planned-date lists and artifact details. Several named Home,
report, planning and custom-view pages still share generic content; this is an
implementation gap, not complete page-specific Rally parity. It does not emulate
Rally enterprise administration, historical Lookback analytics, app-catalog
plugins, or every server-specific custom page. These need APIs and permissions
beyond the supplied core WSAPI workflows. Charts explicitly report that their totals use only loaded items. Timeline views
list planned dates; Gantt controls, capacity data and historical analytics remain open.

The Codex shell exposes app-server conversations, approvals and configuration.
Native Settings support account sign-in, Bedrock setup, local model discovery
and downloads, MCP configuration/OAuth, skills, plugins, hooks, features, memory switches/reset,
imports, feedback, sandbox setup, keyboard bindings and version-checked config
edits. The installed CLI owns accounts and inference configuration. Local files
are read-only viewers with bounded paging.

The source-based interaction audit and remaining differences are recorded in
[INTERACTIONS.md](INTERACTIONS.md) and the item-by-item
[review assessment](REVIEW-ASSESSMENT.md). Full widget-for-widget parity is not yet
complete; the audit distinguishes implemented behavior from unfinished details.

Rally writes enforce origin checks and reviewed revisions but cannot provide
server-side transactional compare-and-swap across artifacts. A concurrent edit
between the final GET and POST is still possible with WSAPI. Batch results retain
the number of completed changes and require a fresh review after a failure.

## Window and memory ownership

Each native window uses its own FLTK process. A 256-bit environment-only
secret authenticates the loopback connection. Transfers use expiring tickets:
reserve, initial snapshot, rendered readiness, final snapshot, destination applied
acknowledgement, then source removal. Cancellation resolves atomically against
commit; bounded two-minute participant-only receipts retain outcomes without
retaining document data. Source-owned child handles terminate and reap cancelled
pop-outs, including late starts, while committed children are released. An
unconfirmed outcome retains both documents, and broker disconnect remains visible.
Empty cancelled pop-outs close automatically.

Closing a pop-out returns its documents serially to a live window, preferring
the main window. Each source tab stays frozen until committed; any failed or
conflicting destination retains the source window. Completed moves remove local
draft/outbox copies. With no other window, normal local session recovery applies.
Full native window arrangement restoration is still unimplemented.
Stable document identities keep separate drafts when both windows show the same
Rally page. Server notifications carry sequence numbers; approvals route to the
owning window. The original process hosts services until the last window closes.

Open-chat metadata is published to the broker only when it changes. Cross-window
messages retain human delivery approval; only bounded matching replies are observed
by the sender. Mailboxes move with their conversation tab.

Each window still owns a separate Rally client and cache; shared broker-side Rally
services are not implemented. Normal Rally queries use 128-record lightweight
pages, and refresh can request a larger existing range. All HTTP request paths
share four concurrent slots per client,
a 32-page/24 MiB LRU with a 45-second TTL, and a 2,048-card resident window per view.
Full descriptions load on demand. Filter changes cancel/supersede old requests;
mutations invalidate the cache, including pre-mutation requests still in flight.
Active views refresh on a jittered 55–65 second schedule, with exponential failure
backoff jittered by ±10%. Refresh fills the complete resident range before publishing. Manual Refresh
always remains visible; clean details also reload their active collection. Inactive
pages are invalidated and fetch when shown.

The UI uses a shared four-millisecond drain budget, capped at 64 callbacks and 64
stream events per frame. Adjacent stream deltas coalesce in a separate inbox with a
256-event/4 MiB admission target (one larger protocol-bounded event may enter an
empty queue). Reads use six workers and 64 waiting jobs, explicit writes two workers
and 16 waiting jobs, and urgent control operations two workers and 32 waiting jobs.
Admission failure restores pending state and reports the error. Reply waits and transfer watchdogs use timers rather than reserving workers. Sidebar page admission is UI-owned, and failed pages keep their cursor for retry. Network/filesystem/
process work, history preparation, transcript layout and persistence writes run
in workers. Initial session parsing also runs in a worker behind a restoration view. Some RPC result decoding, live item formatting, transfer installation
and capture of changed document snapshots still run on the UI thread. Sidebar metadata is immutable: ID-specific edits copy one balanced-tree path, and initial/import scans admit at most 256 records per call. Workers capture roots without copying the history, then search, sort and group it. Stale generations cannot replace current results. The information panel and conversation chooser share this metadata; the chooser renders only its visible rows.
Unchanged checkpoints reuse immutable per-document snapshots; changes publish
after a short debounce, then encode and write in the background. A separate final
flush captures the latest state even before a pending debounce expires. Session
schema 3 stores rich drafts as UTF-8 text and format spans, migrates the legacy
numeric-text/per-rune format, and validates individual documents independently.
Damaged records produce a recovery report and retain the original file; an atomic
last-good backup recovers missing or damaged snapshots. Future schemas disable
older writes. Transfers preserve editor and view positions independently of rich
content, so caret-only changes reuse saved text. Native editor initialization keeps
restored caret/selection state. Structured diffs share one UTF-8 source with file/hunk/line byte ranges and sparse
long-line column indexes. Parsing and bounded token-based intraline comparison run
in workers. Only visible rows draw, without full or per-file native text editors;
selection, collapse and scroll state remain separate from source bytes. A
single slow callback can exceed the drain deadline; complete per-document cancellation
and byte-weighted general job admission remain unfinished. CommonMark/GFM passes through an HTML/rich-text intermediate into cached styled
runs with native links, tables and direct selection; neither HTML nor remote resources execute.
The transcript layout queue has two workers and 32 slots. Native process memory is
sampled every 10 seconds across Fastrock processes for reporting: physical footprint
on macOS ([XNU accounting](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c)) and private committed bytes on Windows ([PROCESS_MEMORY_COUNTERS_EX](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex)). If native accounting is unavailable,
the sample falls back to Go-managed retained bytes. Eviction compares
Go heap allocation against a per-window share of a 256 MiB soft budget, with a
96 MiB minimum; the runtime limit also respects a live-heap floor. The specified
384 MiB aggregate pressure policy, hysteresis, dedicated headless service process
and hidden GPU-surface release remain unimplemented. Unsaved edits, pending approvals and queues are retained.
Real files page up to 4 MiB; long transcripts and render caches have separate bounds.
RSS and private committed memory differ; these targets are not hard process limits.


## Review validation boundary

The earlier review remediation is covered by local headless regression tests,
race tests and vet. The FLTK port additionally has native Windows input and
application smoke validation, plus static FLTK builds for Windows and both Mac
architectures; see [VALIDATION.md](VALIDATION.md). Exhaustive DPI, accessibility
and input-to-present measurements remain open. Earlier fixture results must not
be treated as evidence for untested changes.
Native updater trust is described in [UPDATES.md](UPDATES.md); signing a local test
package does not provision the GitHub release environment.

Clipboard reads/writes use one bounded ordered worker; native failures return to the UI notice system. Editor paste callbacks check the originating editor, visibility, content and selection before applying a delayed result. Collection pages decode into a typed Rally response envelope in one pass, preserving custom fields and error redaction.

Planning metrics and per-iteration totals cache by filtered-data revision. Planning and timeline use fixed-height viewport ranges with skipped extents, including deep scrolling. Memory pressure releases the planning summary along with other reconstructible projections.

Typography uses cached regular/bold/italic/bold-italic faces, with matching monospace variants. Startup discovers installed Segoe UI/Arial and Consolas/Menlo/fontconfig faces; unsupported fonts fall back to embedded Go fonts. Font collection members are selected by weight and slant. Shared caption/large/title roles offset the configured body size by -2/+3/+7, while code uses -1. Transcript wrapping and selection use the rendered face; rich native editors provide matching measurement and enough row height. Native mixed-DPI behavior remains unverified and is tracked separately.
