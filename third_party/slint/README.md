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
