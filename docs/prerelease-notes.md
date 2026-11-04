# Fastrock 0.4.0-rc.13

- Workspace storage now uses Fjall 3.1.13 with engine-managed LZ4. Standard Git deltas share related content; identical Git objects remain stored once across repositories and workspaces.
- Git size checks, deduplication and garbage collection use small metadata records without inflating file contents.
- Independent conversations use optimistic transactions and separate workspace heads, refs and summary keys. Ordinary writes do not update shared reference counts. Fjall handles incremental compaction.
- Existing redb stores migrate automatically on first use, with a native progress bar and Pause/Resume. Migration verifies every logical record before activation and preserves the original redb. Later startup opens Fjall directly.
- **File → Migrate legacy storage…** imports another legacy store into an unused workspace home. Interrupted migrations resume from committed batches. Existing destinations are never overwritten.
- Search summaries, memories, checkpoints and cold transcript pages now rely on Fjall's compression. Goose remains pinned and unchanged.

Use **Quit** in the previous version's tray menu before launching RC13. The one-time migration needs additional disk space for the new store; it retains the original redb. Keep that source until satisfied with the migration. Back up the complete Fjall directory while Fastrock is stopped.

See [storage and migration](https://github.com/allquixotic/fastrock/blob/v0.4.0-rc.13/docs/FJALL-STORAGE.md) and the [user guide](https://github.com/allquixotic/fastrock/blob/v0.4.0-rc.13/docs/gui.md).

Signed Windows x64 ZIP; macOS compatibility compile only. This is a release candidate; final live AWS profile acceptance remains with the user.
