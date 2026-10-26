Fastrock 0.3.0 is a stable release with native virtual-workspace browsing and editing, the external Goose ACP backend, optional Codex backend, and the existing native Rally workspace.

When Goose has an enabled, installed vwork MCP extension, the toolbar gains a **Virtual** button. Browse all configured stores, repositories and workspaces; filter and page through files; edit UTF-8 text; create/fork virtual workspaces; inspect saved diffs and logical sizes. Saves use the loaded revision and preserve drafts on conflicts. Fastrock talks to the existing daemon and never opens redb or exports a checkout.

The feature stays hidden when Goose is not configured for vwork. Text editing is limited to 2 MiB; binary files and symlinks are listed but not editable. Unsaved drafts live in memory, so save before quitting. [Setup and usage](https://github.com/allquixotic/fastrock/blob/v0.3.0/docs/VIRTUAL_WORKSPACES.md).

This release distributes a Windows x64 ZIP only. Its executable is Authenticode signed and timestamped, with a SHA-256 checksum and GitHub build attestation. CI builds both supported platforms and executes Rust tests plus Goose/Rally fixture checks on Windows. No live provider account or production Rally data is used.

The `-fast` filename describes the retained unoptimized build profile, not release stability. The GitHub release is stable/latest. vwork, Goose and optional Codex remain separately installed dependencies. No Mac binary is published for this version.
