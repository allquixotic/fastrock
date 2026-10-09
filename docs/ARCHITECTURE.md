# Architecture

`cmd/fastrock` loads preferences and starts the native shell. Production packages
under `internal` are separated from the deterministic mock servers in
`cmd/mock-rally`, `cmd/mock-model`, `internal/mockrally` and `internal/mockmodel`.
No fixture records are loaded by the application.

- `codex`: lifecycle, newline JSON-RPC, request cancellation, events, capability
  catalog and service tiers. All inference is delegated to `codex app-server`.
- `rally`: direct WSAPI v2.0 authentication, scopes, pagination, metadata, CRUD and
  attachments. Absolute references must stay on the configured origin and under
  its WSAPI path. Redirects are disabled to keep tokens on that origin. GET retries
  are bounded and cancellable; mutation requests are never automatically retried.
- `assistant`: typed read/view/proposal tools and revision-checked application.
- `workspace`: document tabs, conversation state and independent prompt queues.
- `settings`: atomic JSON preferences/session writes and the OS credential vault.
- `platform`: file/folder dialogs, URL opening and Windows screenshot capture.
- `ui`: nucular presentation. Network work runs in goroutines and posts results
  to the UI loop; presentation state is owned by that loop.

The shell keeps the Codex GUI's top document strip, conversation sidebar, chat
workspace, details panel and Settings navigation. Rally uses a separate native
navigation/toolbar within its document. The layouts share theme tokens and fonts.
The original browser app is a design reference, not an embedded runtime.

## Deliberate limits

This rewrite implements WSAPI-backed boards, lists, current-scope charts,
planning summaries, date timelines and artifact details. It does not emulate
Rally enterprise administration, historical Lookback analytics, app-catalog
plugins, or every server-specific custom page. These need APIs and permissions
beyond the supplied core WSAPI workflows. Current-scope charts are labeled as
such. Timelines display planned dates; they are not historical burndown charts.

The Codex shell exposes app-server conversations, approvals and configuration.
The Settings account/MCP/skills pages inspect the server's results; provider setup
and CLI sign-in remain in Codex. Local file documents are read-only viewers.
This is a functional native rewrite, not a pixel-identical rendering of Slint or
an implementation of every unrelated Codex desktop service.

Rally writes enforce origin checks and reviewed revisions but cannot provide
server-side transactional compare-and-swap across artifacts. A concurrent edit
between the final GET and POST is still possible with WSAPI. Batch results retain
the number of completed changes and require a fresh review after a failure.
