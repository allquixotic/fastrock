# Validation

## Verified on 2026-10-09

Fastrock builds with **Go 1.27.2 and `CGO_ENABLED=0`** for Windows amd64,
macOS arm64 and macOS amd64. `go list -deps` reports no `CgoFiles` in either
the Windows or macOS application dependency graph. The vendored nucular
software renderer uses Ebitengine 2.10.0 for native presentation.

`make check` passes root vet/tests and the vendored nucular tests with the
`nucular_headless` tag. This tag disables native windows. The tests cover
WSAPI scopes/pagination, CRUD/custom fields, metadata, token/redirect containment,
cancellation, mutation retries, JSON-RPC correlation/shutdown, version rejection,
model/speed gating, proposal conflicts, conversation/queue isolation, persistence,
rich HTML editing and cache invalidation. Windows-targeted vet also passes.

Windows 11 x64 desktop validation uses the complete official Codex 0.162.0
distribution and loopback Rally/model fixtures. No real model or Rally account
is contacted. The installed-CLI integration test also passes on Windows, including
streaming and native Rally tools through Codex code mode. Coverage includes:

- Dark/light boards, search result counts, story and child task details.
- Rich HTML edits saved through WSAPI and retained after reopening.
- Physical keyboard Select All, bold formatting, typing, clipboard copy and Ctrl+S.
- Integrated tab closing, Ctrl+T/Ctrl+W, tab reordering and board drag-and-drop
  followed by an assertion of the persisted Rally state.
- Native open, save and folder dialogs opening and cancelling successfully.
- Conversation streaming, Rally assistant tools and orderly shutdown.

`dev/windows-smoke.ps1` runs the main UI scenario from a signed-in Windows
desktop session and writes screenshots plus assertions into its test directory.
Screenshots and other generated validation files are excluded from Git.
No GUI was launched or tested on macOS; macOS verification is compile/headless only.

## Go 1.27.2 investigation

The previous validation notes attributed a Windows blank-window problem to
Go 1.27.2 and pinned Go 1.26. A controlled retest of the unmodified previous
Shiny implementation built with Go 1.27.2 passed on the same Windows host.
That does **not** substantiate a Go compiler/runtime defect. The earlier version
attribution was too strong; the historical intermittent failure was not reproduced.

The replacement desktop adapter, Go 1.27.2 builds, native input, screenshots and
shutdown have been validated together. Gio is removed. Fastrock no longer uses
PowerShell/C# compilation for Windows dialogs or screenshot capture.

## Allocation measurements

The repeated 1,000-story filter benchmark, on the same Apple M5 Max and Go 1.27.2:

| Operation | Before | After |
| --- | --- | --- |
| Unchanged board filter | 99,731 ns, 104,193 B, 2,001 allocations | 7.9 ns, 0 B, 0 allocations |
| Unchanged 3,600-character editor snapshot | — | 1,945 ns, 0 B, 0 allocations |
| Cached font measurement | — | 16.4 ns, 0 B, 0 allocations |

These measure reuse after the first calculation, **not** initial filtering or
whole-frame rendering. Tests also assert zero allocations when reusing board
layout/sidebar grouping and during coalesced edits with sufficient text capacity.
The UI caches parsed search text, group membership, card wrapping and font widths;
rich-text undo storage is bounded and discarded snapshots release their references.

Reproduce the measurements without opening a window:

```sh
CGO_ENABLED=0 GOTOOLCHAIN=go1.27.2 go test -tags=nucular_headless \
  -run '^$' -bench 'Benchmark(BoardFilter|EditorSnapshot|FontWidthCached)$' \
  -benchmem ./internal/ui
```

## Native windows, streaming and responsiveness

`dev/windows-popout.ps1` additionally exercises a 10,000-story fixture:
128-row demand paging past the 2,048-card resident window, manual refresh,
server filtering, unsaved formatted story transfer, live chat transfer, unsent
composer preservation, Codex restart, and survival of both pop-outs after the
original window closes. One installed app-server serves all windows. The final
restart/pop-out run completed without failed assertions; 214 draw callbacks,
maximum 8.88 ms, reported 174 MiB aggregate RSS at the last in-app sample.
A subsequent post-close sample measured 261.0 MiB RSS and 464.9 MiB private
committed memory across the three Fastrock processes, excluding Codex and fixtures.
These are fixture measurements, not a guarantee for arbitrary window counts or data.

Additional headless regression tests cover expiring/acknowledged/cancelled window
transfers, restart with connected windows, duplicate-page draft isolation, cached
page ownership and invalidation during in-flight requests, failure backoff,
UTF-8 file boundaries, secret file contents excluded from session snapshots,
configuration key quoting/layer origins, approval decisions, MCP schema validation,
caret-local completion, styled Markdown tables, cross-window chat discovery and
native soft wrapping, direct rich-text selection and source line/column links.
Physical Escape, Ctrl+Comma and image paste are also verified. All tests use
deterministic or local data. Detailed interaction coverage is in
[INTERACTIONS.md](INTERACTIONS.md).
