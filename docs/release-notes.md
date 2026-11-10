# Fastrock 0.5.1

Saved conversations no longer require checkpoint review or Resume. Send normally
after reopening a conversation. Interrupted tool calls are recorded as failed
before activation and are never automatically replayed. Existing conversation
messages remain intact; explicitly queued messages remain available through the
queue's Send button or follow the next new message after it completes. Interrupted
restore operations keep their current saved state and retain the previous state in Checkpoints for optional undo.

The message composer now saves its text, cursor and undo/redo history to disk.
Ctrl+Z undoes, Ctrl+Y or Ctrl+Shift+Z redoes; the native context menu offers both.
History is isolated per chat and survives switching tabs, unexpected process exit
and clean restart. Sending clears the composer as an undoable edit. Background
saves coalesce for 200 milliseconds, then atomically sync the snapshot; work since
the latest completed disk save can still be lost in a crash. History retains up to
512 edits and 2 MiB of text deltas, plus the last edit when it exceeds that limit.

Goose 1.58.0 remains unmodified. Provider configuration, credentials, conversations,
workspace history and manual checkpoints are preserved. Windows x64 ZIP only;
macOS receives a compatibility compile. Existing build/signing/timestamping and
memory-guarded Windows verification requirements remain in force.

Use **Quit** in the previous version's tray menu before launching 0.5.1.
