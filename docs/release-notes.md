# Fastrock 0.4.1

Fastrock 0.4.1 is a stable Windows release with buffered workspace storage to
reduce foreground disk I/O on machines with expensive file operations.

- Ordinary workspace edits commit atomically in RAM. A dedicated worker flushes
  journal buffers to the OS on a one-second cadence and requests permanent
  storage after 30 seconds, within the authorized 60-second durability window.
  Continuous writes do not postpone the oldest pending write's deadline.
- Explicit editor Save, manual memory/project edits, merges, restores and manual
  checkpoints sync the journal before reporting success. Recovery intents and
  migration activation retain their immediate durability barriers.
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

This release changes buffering and durability. The exact cause of the reported
work-machine `session/load` timeout remains unconfirmed. Live private provider
profiles remain user-validated; automated acceptance uses local fixtures.

Use **Quit** in the previous version's tray menu before launching 0.4.1. Back up
the complete Fjall directory while Fastrock is stopped. No separate backend
installation is required.

Signed and timestamped Windows x64 ZIP with CI provenance. Application and
dependencies use optimization level 2, with LTO off. macOS compatibility is
compiled; no Mac binaries are published.
