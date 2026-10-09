# Validation

The automated suite checks WSAPI scopes/pagination, CRUD and custom fields,
metadata, token/redirect containment, bounded cancellation, no mutation retries,
JSON-RPC correlation and shutdown, version rejection, model/speed gating, proposal
review/revision conflicts, conversation tab/queue isolation, persistence and
presentation models. `go test` does not open the GUI.

The opt-in integration test launches the installed Codex 0.162.0 app-server with a
temporary mock provider. It streams a response and calls native Rally query/view
tools through Codex's current code-mode protocol. No real model or Rally account
is contacted.

Windows 11 native smoke coverage: startup, dark/light boards, artifact detail,
conversation streaming, Rally assistant tools and orderly shutdown. The scenario
uses the complete official Codex 0.162.0 binary distribution and loopback fixtures.
`dev/windows-smoke.ps1` is reproducible in a signed-in Windows desktop session.
Screenshot capture is Windows-only. macOS validation is compile/headless only.

Build with Go 1.26.0. Both the default Gio path and Go 1.27.2 Windows output exposed
blank-window problems on the validation host. The verified Windows configuration
uses nucular's Shiny renderer and Go 1.26.0; those choices are pinned and documented.

## Verified on 2026-10-09

- Go 1.26.0: `go vet ./...` and `go test -race ./...` passed.
- The installed Codex 0.162.0 integration test passed with native Rally tools.
- Windows 11 x64: the packaged GUI passed the smoke scenario, and screenshots
  confirmed visible boards, details, conversation output, and the Rally assistant.
- Windows-native UI and Rally unit tests passed; macOS arm64 compiled successfully.
- No GUI was launched or tested on macOS.
