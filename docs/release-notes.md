# Fastrock 0.4.2

Fastrock 0.4.2 improves workspace commands on repositories with long Git history
and many objects.

Changes in this release:

- Short Git logs traverse only the needed history frontier instead of sorting
  all reachable history before returning the first result. Previews use commit
  date order and retain both parents of merge commits.
- Temporary checkouts export Git history in bounded, streamed packs instead of
  creating a separate loose file for every historical object. Full history,
  tags, revision guards and isolated checkout ownership remain available.
  Creating a new temporary checkout still reads and exports repository history.
- Heavy workspace commands use at most four of eight worker slots, leaving
  capacity for small commands. Cancellation retains a running worker's slot
  until its work completes.
- Diagnostics separates workspace store, admission, worker and execution waits,
  and measures saving new-workspace recovery state. Measurements contain fixed
  operation categories, never paths or command contents.

Retains the conversation-opening improvements from 0.4.1 build 4:

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

The reported work machine is not available for direct reproduction. This release
addresses verified history traversal, file amplification and worker admission
defects; it does not measure that machine's security-software overhead. Goose remains
unmodified, including its provider and extension network deadlines. Live private
provider profiles remain user-validated; automated acceptance uses local fixtures.

Use **Quit** in the previous version's tray menu before launching 0.4.2. Back up
the complete Fjall directory while Fastrock is stopped. No separate backend
installation is required.

Signed and timestamped Windows x64 ZIP with CI provenance. Application and
dependencies use optimization level 2, with LTO off. macOS compatibility is
compiled; no Mac binaries are published.
