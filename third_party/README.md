# nucular

Source: https://github.com/aarzilli/nucular
Commit: 58b808aa577248d4d3d0cd7af89ea4a3d0dc5d43
License: MIT (see nucular/LICENSE).

The fork retains nucular's widgets, layout, input, and software rasterizer. The desktop
presentation adapter in `internal/ebitenscreen` implements the small Shiny `screen`
interface using Ebitengine **2.10.0**. Windows and macOS now use the same path with
**Go 1.27.2 and CGO_ENABLED=0**. Gio and its dependencies have been removed. The
adapter supports software buffers; texture APIs fail explicitly because nucular does
not use them. Ebitengine is responsible for native windows and GPU presentation.

Changes from upstream:

- Integrated tab, board and formatting widgets live in the application, without
  altering nucular's general widget behavior.
- `TextEditor.Snapshot` caches UTF-8 without allocating on unchanged text;
  `PaintText` allows formatted runs with native cursor, selection and scrolling.
  A placeholder field gives empty search/composer editors a visible hint.
- Parsed TrueType fonts are actually inserted into the existing font cache.
  A bounded typed width cache avoids interface boxing and linked-list allocations.
- Pure Go Windows clipboard uses movable global memory and bounded UTF-16 reads;
  macOS clipboard uses NSPasteboard through purego, without a compiler or subprocess.
- The `nucular_headless` build tag excludes the desktop driver during unit tests.
- Wrapped-label height and padding fixes plus `WrapText` retain the previous patch.
  SIMD tests remain amd64-only, matching their assembly implementation.

The upstream license and source notices are retained. Ebitengine and purego are
Apache-2.0; see their pinned modules for copyright and license text.

Additional local changes preserve wrapped editor text/caret/selection without
inserting newlines, expose asynchronous clipboard text reads, and stop the
renderer updater when its native window closes (releasing its frame buffers).

Menu and closable-popup Escape handling removes the top popup before the next
layout pass; regression coverage protects non-closable approval dialogs.
