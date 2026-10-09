# Validation

## FLTK port, verified on 2026-10-09

- `make mac` and `make windows` build with Go 1.27.2 and CGo enabled.
  macOS arm64 and amd64 both link successfully; Windows amd64 builds with
  MinGW-w64. Import-table checks find no FLTK dynamic library or Windows
  GCC/libstdc++/winpthread DLL. The binaries use normal system libraries.
- The full headless tests, race suite, `go vet -tags=fltk_headless ./...` and the
  pinned staticcheck with the headless tag pass. Headless
  tests also run with `CGO_ENABLED=0`; this never initializes FLTK or a display.
- A physical Windows input fixture creates an FLTK field and button and verifies
  Unicode replacement, Select All/Copy, canvas focus isolation, Enter commit and button dispatch reach
  the application model. See `dev/native-smoke.go` and
  `dev/windows-native-input.ahk`.
- The complete Windows smoke scenario passes: dark/light boards, filtering,
  story/task details, saved rich HTML, conversation streaming, Rally-assistant
  tools and shutdown. It uses only loopback fixture services. The mock server
  now supports the mixed Artifact queries used by Team Board.
- The Windows pop-out scenario also passes with 10,006 mixed artifacts: bounded
  paging, refresh/filter, rich draft transfer, chat transfer, Codex restart and
  unsent draft preservation. Its final sample reports 300 MiB across the windows.
- Screenshots were inspected from Windows. No GUI was launched on the Mac.
  Exhaustive physical interaction and mixed-monitor DPI parity are not claimed.

## Prior nucular validation, 2026-10-09

Before the FLTK port, Fastrock built with **Go 1.27.2 and `CGO_ENABLED=0`** for Windows amd64,
macOS arm64 and macOS amd64. `go list -deps` reports no `CgoFiles` in either
the Windows or macOS application dependency graph. The vendored nucular
software renderer uses Ebitengine 2.10.0 for native presentation.

At that checkpoint, `make check` passed root vet/tests and the vendored nucular tests with the
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
CGO_ENABLED=0 GOTOOLCHAIN=go1.27.2 go test -tags=fltk_headless \
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

## Navigation and updates (2026-10-09)

- Headless regression renders an expanded 100,000-conversation folder, checks
  bounded draw commands, collapse/reopen, changed title search and archive scope.
  A separate 10,000-project fixture verifies that offscreen headers also stay
  outside the rendered row set.
  Its alternating collapse/expand software-render benchmark on this Mac measured
  0.059–0.065 ms/frame for both 100 and 100,000 rows (prior implementation:
  6.33 ms for 100,000). Full-window sidebar toggle plus Rally redraw: 0.30 ms. This is not a Mac GUI test.
- Session regression verifies only open conversations and retained drafts are
  copied, excluding unrelated loaded server history from recurring snapshots.
- Windows native UI fixture: 10,000 conversations and 10,000 Rally stories;
  search, disclosure clicks, sidebar toggle, dark/light Rally navigation, View and
  Help menus. Screenshots remain local under `build/`.
- Windows update unit suite exercises a live process wait, native Start menu link,
  verified staging, corrupt/truncated/oversized/missing-platform downloads,
  traversal rejection, cancellation and rollback. The test restores the original
  user shortcut after checking it.
- Windows helper smoke test keeps the old executable running, verifies it is not
  replaced early, exits it, then confirms replacement and automatic restart of a
  new fixture executable. No external release or production API is needed.
- macOS arm64 DMG and app packaging checked non-interactively. Mac GUI execution
  remains prohibited; public Developer ID signing/notarization is not claimed.

Windows headless timings (Core Ultra 9 275HX): folder toggle/render 0.098 ms
for both 100 and 100,000 chats; complete sidebar visibility/Rally redraw 0.405 ms.
These timings exclude the native compositor. Physical input checks used the
Windows GUI; no GUI or display-driver test ran on the Mac.

## Compact FLTK navigation (2026-10-09)

The section/page selectors are bordered native buttons at 25 logical pixels.
Every Rally control row can be collapsed independently with its small minus
button. Hidden rows share a compact + restore strip; + All restores them together.
Preferences retain hidden rows across restarts without changing filters or scope.
The saved-view and timebox rows use inline labels on wider windows. New-tab
recents begin collapsed and remain expandable; existing folder, file, resume and
Rally actions remain available.

Tabs retain a 160-logical-pixel minimum width. A Windows mouse fixture verifies
independent row visibility buttons, single-click scrolling, double-click jumps
to each end, and automatic selection reveal across 13 tabs. Run
`dev/windows-smoke.ps1 -Layout` alongside `dev/windows-layout-input.ahk` on Windows.
Headless checks cover selection reveal at 100/150/200% scale, drag ordering,
collapse/restore, recent-folder expansion and tooltip/native-button overlap.
Screenshots are saved locally in `build/ui-refinement-screenshots/`.
The complete headless suite, vet, pinned staticcheck and targeted race checks pass after these refinements. Both native build targets retain static FLTK linkage.

The per-row compaction fixture is `dev/windows-smoke.ps1 -Rows`. It captures
expanded, partially hidden, fully hidden and restored Rally controls in both
themes. Headless checks also cover independent restore, legacy preference
migration, persistence, board-height recovery and wrapped rows at multiple DPIs.
The 797-test headless suite, vet, pinned staticcheck, targeted race checks, and
Windows mouse checks pass. The full Windows fixture also passes filtering,
rich-text save/reopen, child-task navigation, chat and Rally assistant checks.
Updated screenshots are in
`build/rally-row-screenshots/`; both release binaries verify static FLTK linkage.

## Initial-release CI follow-up (2026-10-09)

The first GitHub run exposed Unix-only fixture paths in delivery permissions and
thread creation, a Windows filename suggestion that lost a drive-like prefix,
and a create fixture that rejected the expected background board/schema refresh.
Fixtures now use native absolute paths and serve the read-only refresh routes;
filename suggestions parse normalized separators consistently across hosts.

The full headless suite passed locally. The complete `internal/codex` and
`internal/ui` test binaries passed on Windows, and the affected tests passed
with the race detector on macOS. No GUI was launched on the Mac.
