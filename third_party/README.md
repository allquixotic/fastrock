# nucular

Source: https://github.com/aarzilli/nucular
Commit: 58b808aa577248d4d3d0cd7af89ea4a3d0dc5d43
License: MIT (see nucular/LICENSE).

Local patch: select nucular's existing Shiny software renderer on Windows by
changing build constraints in shiny.go, gio.go, and gio_windows.go. macOS keeps
Gio. This avoids the blank output observed with Gio on the Windows validation
host. Use the pinned Go 1.26.0 build scripts; Go 1.27.2 also produced blank output
with Shiny in that environment. No widget API changes.

Additional patch: repair wrapped-label height/padding handling so a one-line label
and the last wrapped line render, and expose `WrapText` to reserve the measured
height in the application. Regression tests are in `text_wrap_test.go`.
The upstream SIMD benchmark test is restricted to amd64, matching its assembly
implementation, so the wrapping tests can run on macOS arm64 without a GUI.

Upstream whitespace in the workflow, documentation example and assembly was
normalized without changing behavior. The unused Windows Gio file is retained
with an `ignore` build constraint.

The rich-text style tests were updated for upstream's embedded `TextStyle.Flags`
and parameterless link callback. Both module test suites run in `make check` and CI.
