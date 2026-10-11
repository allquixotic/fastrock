Fastrock 0.2.0 is the first official Rust/Slint release, rebuilt on codex-gui with native Rally pages, Team Board, filters, saved views, editing, attachments, bulk changes and reviewed AI proposals. Codex uses your installed external `codex app-server` (CLI 0.162.0 or newer).

This release includes the Team Board metadata fix from alpha.5. Rally's built-in dropdown values no longer trigger a false duplicate-record error.

The official binaries use basic compiler optimization (`opt-level=1`), full debug information (`debug=2`) and **LTO completely disabled** (`lto="off"`). Matching Windows PDB and macOS dSYM archives are published separately for crash investigation. Optimization can still make some local variables unavailable or move source lines during debugging.

Windows x64 is Authenticode signed and timestamped. The Apple Silicon Mac app and DMG are Developer ID signed, notarized and stapled. Each application and symbol package has a SHA-256 checksum and GitHub build attestation. Windows fixtures exercise the exact signed executable; all Rally test data stays on loopback fixtures.

Download the application package for your operating system. Keep the corresponding `*-symbols.zip` with crash reports; it must match this exact version. See [debugging instructions](https://github.com/allquixotic/fastrock/blob/main/docs/releases.md) for PDB/dSYM use. Debug symbols do not need to be installed to run Fastrock.
