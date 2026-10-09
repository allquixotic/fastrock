# Desktop dependencies

Fastrock uses [go-fltk](https://github.com/pwiecz/go-fltk) at commit
`3e944122e7b1f66db5e3a968e679285930c7e10e`. Its version and checksums are pinned
in go.mod/go.sum. The module supplies static FLTK archives for Windows amd64 and
macOS arm64/amd64; the compiler links them into Fastrock. Build helpers verify
that no FLTK DLL/dylib or Windows GCC runtime is imported by the executable.
The Go bindings are MIT; FLTK is LGPL-2.0 with static-linking exceptions.

The custom controls in `internal/desktop` are derived from
[aarzilli/nucular](https://github.com/aarzilli/nucular), commit
`58b808aa577248d4d3d0cd7af89ea4a3d0dc5d43` (MIT). Its original license is retained
as `internal/desktop/LICENSE`. This is retained code, not an independent rewrite.
No nucular module is required. Ebitengine and Shiny no longer supply the
application's window backend.

FLTK owns standard buttons and plain single-line fields, including their native
focus, text input, clipboard and undo. Value snapshots and sequenced events join
these controls to existing application state. Rich editors, transcripts, tabs,
board cards, popups and other custom controls keep their canvas layout and
rendering. That code retains the existing soft wrapping, selection, clipboard,
font caches, damage tracking and regression tests. The `fltk_headless` build tag
excludes the native driver and CGo for deterministic tests without a display.

Purego remains for platform APIs. The embedded Droid and Proggy fonts retain
their upstream licensing; Go fonts are supplied by golang.org/x/image.
