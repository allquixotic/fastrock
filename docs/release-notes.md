# Fastrock 0.4.1 — build 4

Fastrock 0.4.1 is a stable Windows release with buffered workspace storage to
reduce foreground disk I/O on machines with expensive file operations.

Build 4 repairs conversation opening on slow storage:

- Saved history opens independently of agent activation. Credentials, extension
  startup and workspace recovery no longer gate reading old messages.
- Explicit Resume waits for recovery and agent initialization while preserving
  the displayed history. Fastrock removes its fixed session/history deadlines;
  real failures show retry controls. Uncertain tools still require review.
- New workspaces reference shared Git commits without walking or indexing every
  file. Single-file reads resolve only that path and its revision overlays.
- Session operations share Goose's SQLite pool instead of repeatedly opening
  pools and checking schema initialization. Cold scrollback cache creation and
  writes run in the background, with new pages initially readable from RAM.

The existing buffered durability policy remains:

- Ordinary workspace edits commit atomically in RAM. A dedicated worker flushes
  journal buffers to the OS on a one-second cadence and requests permanent
  storage after 30 seconds, within the authorized 60-second durability window.
  Continuous writes do not postpone the oldest pending write's deadline.
- Explicit editor Save, manual memory/project edits, merges, restores and manual
  checkpoints sync the journal before reporting success. Recovery intents and
  migration activation retain their immediate durability barriers. New
  conversation and fork workspaces sync before their recovery bindings are saved.
- Quit drains workspace operations and syncs pending changes without blocking the
  interface. A sync error keeps Fastrock open. Diagnostics shows pending journal
  work and failures; failed or overdue storage pauses further writes.
- Disposable scrollback pages also buffer journal writes. Authoritative
  conversation history remains in unmodified embedded Goose's SQLite storage.
- Retains embedded Goose and vwork, native conversation/Rally UI, provider
  profiles, scheduling, local search, conservative memories and Fjall LZ4/Git
  delta storage. Existing redb stores migrate with native progress controls;
  the original database is preserved.

A crash can lose recent buffered edits within the 60-second window. The sync
worker starts early to allow for slow I/O; a stalled device, suspended process
or failed storage cannot be given an absolute wall-clock guarantee. Explicit
successful durable saves include earlier committed changes in that store.

The reported work machine is not available for direct reproduction. This build
addresses the verified activation/deadline and eager-indexing defects; it does
not claim to measure that machine's security-software overhead. Goose remains
unmodified, including its provider and extension network deadlines. Live private
provider profiles remain user-validated; automated acceptance uses local fixtures.

Use **Quit** in the previous version's tray menu before launching 0.4.1. Back up
the complete Fjall directory while Fastrock is stopped. No separate backend
installation is required.

Signed and timestamped Windows x64 ZIP with CI provenance. Application and
dependencies use optimization level 2, with LTO off. macOS compatibility is
compiled; no Mac binaries are published.
