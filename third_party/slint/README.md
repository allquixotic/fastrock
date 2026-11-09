# Slint 1.18.1 GUI bridges

`i-slint-core` is the crates.io 1.18.1 source package with four changed files.
Cargo downloaded-package metadata and its nested lockfile are omitted. Slint
licenses and copyright notices are preserved unchanged.

- `textlayout/sharedparley.rs`: accumulate wrapped-link rectangle hits (`|=`),
  so every wrapped line can be clicked or hovered. Also expose a narrow
  `rich_text_cursor` bridge to the existing cached shaped layout and draw
  rich selections/carets using native selection geometry.
- `textlayout/sharedparley/shaping.rs`: preserve rich paragraph byte ranges in
  the plain text used for native cursor hit-testing and selection painting.
- `items/text.rs`: internal rich-text anchor/cursor, selection colors and caret
  properties. These add no Slint language API or new layout pass.
- `item_rendering.rs`: forward these properties through `RenderText`, retaining
  zero-selection defaults for ordinary text. All renderers share the layout.

`src/window_runtime.rs` checks wrapped links, clipping, selection invalidation
and painted highlights with the software renderer on Windows.
`dev/windows-purpose-selection-smoke.py` checks real pointer/keyboard input and
clipboard output. Never execute rendering or GUI tests on Sean's Mac.

Keep each bridge until an upstream Slint release provides the corresponding
public behavior. Remove the Cargo override and vendor directory only after all
bridges can use upstream APIs and Windows rendering/interaction tests pass.

`i-slint-backend-winit` retains the crates.io 1.18.1 package, including licensing.
`renderer/sw.rs` uses `retained_buffer.rs` on Windows: dirty regions rasterize into
private CPU storage and every native redraw presents the complete retained frame.
False buffer-age reports after RDP surface loss therefore need no full text redraw.
`software_metrics.rs` exposes bounded, opt-in, in-memory frame measurements used by
`dev/windows-scroll-benchmark.py`; it performs no rendering-thread I/O or polling.
Other platforms retain upstream presentation behavior.

`accesskit.rs`, `accesskit_focus.rs` and `winitwindowadapter.rs`: preserve
re-entrant structural notifications during accessibility updates and defer one
full rebuild after the adapter borrow ends. Focus-only, partial and full updates
use only nodes already in the published/current tree; otherwise they temporarily
focus the root until rebuilding publishes the requested control. Deferred
destruction prunes dead component identities/text caches. Regression coverage:
`tests/accesskit_focus.rs` (real AccessKit consumer) and
`dev/windows-accessibility-smoke.py` (active Windows UI Automation). Tracks
[upstream Slint #10269](https://github.com/slint-ui/slint/issues/10269).
