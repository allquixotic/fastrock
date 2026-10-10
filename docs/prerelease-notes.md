Fixes Team Board failing with "Rally returned duplicate records" when loading Rally's built-in dropdown metadata. Enum choices may all use the placeholder reference `"null"`; Fastrock now preserves each choice and accepts the complete zero-index catalog response. Duplicate checks for actual objects remain enforced. Regression tests and Windows fixture acceptance exercise these real WSAPI response forms.

Fastrock rebuilt on codex-gui with all Rally UI implemented in native Slint controls. Rally pages occupy horizontal document tabs; the sidebar remains Codex conversations. Codex uses the installed external `codex app-server` (CLI 0.162.0 or newer).

These rapid test builds deliberately use opt-level 0, no debug information, no LTO, 256 codegen units and incremental compilation. CI reuses a persistent build cache and packages without a compression/optimization pass.

The Windows x64 executable is Authenticode signed by Sean McNamara with an RFC 3161 timestamp. CI verifies its signature and runs the Rally fixture suite on those exact signed bytes before publication. The Apple Silicon Mac DMG is added after Developer ID signing, Apple notarization, stapling and Gatekeeper verification finish. No Mac GUI tests are run. Each platform includes a GitHub build attestation and SHA-256 checksum.

The detailed Rally accounting, migration ledger and verification records are included in the repository and Windows ZIP. Automated tests use local fixtures, with no production Rally writes.
