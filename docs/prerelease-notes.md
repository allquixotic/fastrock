# Fastrock 0.4.0-rc.12

- Old conversations remain readable when workspace storage is slow or locked. After a two-second checkpoint recovery probe, unavailable storage produces a read-only history view. **Resume** retries the check before allowing work; uncertain tools and interrupted restores still require review.
- Launching Fastrock again restores the existing Windows tray process and forwards requested folders or threads. It no longer starts a competing owner of the same profile. Explicit **Quit** releases ownership.
- Workspace tool discovery no longer opens redb, and embedded store startup defers expired-workspace cleanup. Cleanup still preserves changed files. Database lock errors give ownership guidance instead of suggesting backup restoration.
- Restore diagnostics identify active loading stages, and request timeout messages no longer refer to Codex. Conservative memories, provider profiles, Bedrock routes, Rally, scheduling and existing conversation features remain supported. Goose remains unchanged.

When upgrading from RC11 or older, use **Quit** in the old version's tray menu before launching RC12. Closing its window alone leaves it running. Do not delete or replace a workspace database because of a lock conflict.

See [memory behavior and limits](https://github.com/allquixotic/fastrock/blob/v0.4.0-rc.12/docs/MEMORIES.md) and the [user guide](https://github.com/allquixotic/fastrock/blob/v0.4.0-rc.12/docs/gui.md).

Signed Windows x64 ZIP; macOS compatibility compile only. This is a release candidate; final live AWS profile acceptance remains with the user.
