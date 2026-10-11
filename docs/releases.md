# Official 0.2.0 and rapid signed prereleases

Sean explicitly authorized the official **0.2.0** build with optimization and
crash symbols. `cargo build --profile official` selects `opt-level=1`, `debug=2`,
`split-debuginfo="packed"`, `lto="off"`, `strip="none"`, 256 codegen units and
incremental compilation. Build scripts/proc macros retain fast settings. Use
`"off"` because Cargo's `lto=false` can still perform local ThinLTO at nonzero
optimization levels. Other/default build profiles and prereleases stay fast;
new stable version optimizations require explicit authorization.

The v0.2.0 tag publishes a normal GitHub release marked latest. CI chooses the
profile from the version and verifies the declared settings, source and input
hashes before signing. Application filenames omit `-fast` for this release.
Separate `fastrock-0.2.0-windows-x64-symbols.zip` and
`fastrock-0.2.0-macos-arm64-symbols.zip` contain full symbols plus `SYMBOLS.json`.
They have checksums and attestations alongside the application packages.
Windows CodeView GUID/age must match the PDB; Mac Mach-O UUID must match the
dSYM before publication. Missing or mismatched symbols fail the release.

For Windows crash dumps, extract this release's PDB alongside the matching
`fastrock.exe`, or add its directory to WinDbg's symbol search path. In WinDbg,
`.sympath+ C:\path\to\symbols`, `.reload /f fastrock.exe`, and `!analyze -v`
load the matching symbols and inspect the dump. Symbols cover Fastrock and
compiled Rust dependencies; Windows system symbols are separate. Build-source
paths in the PDB can be mapped to a checkout of the `source_commit` recorded in
`SYMBOLS.json`.

For Mac crash reports, preserve the matching `fastrock.dSYM` and executable.
`dwarfdump --uuid` checks their match. LLDB can load the executable and use
`target symbols add /path/to/fastrock.dSYM`; `atos` can symbolicate a report's
addresses using its image load address and the matching dSYM DWARF file.
Optimization can inline functions or eliminate local variables even with full
debug information. No stripped/optimized symbol substitute is generated.

## Rapid prereleases


Commit, push directly to main, then push the matching prerelease tag. Do not create
PRs or publish locally built release assets. `.github/workflows/release.yml`
builds, signs, verifies, attests and publishes the Windows x64 ZIP through CI; it
then adds the signed/notarized Apple Silicon Mac DMG to the same prerelease.
Tags must match Cargo.toml/Cargo.lock, for example `v0.2.0-alpha.4`. No force pushes.

Default dev/release/test profiles favor compile speed: opt-level 0, debug 0, split-debuginfo off,
LTO false, codegen-units 256, incremental true, strip none. Build dependencies also
use opt-level 0 and 256 codegen units. Environment overrides may not alter these declared settings. `dev/check-build-policy.py` verifies the policy
before supported scripts and CI. Sean must explicitly authorize any exception.

Build with `cargo build --locked --bin fastrock`; never add an optimization pass.
Mac-to-Windows builds use `cargo xwin build --locked --target
x86_64-pc-windows-msvc --bin fastrock`. Both use the development profile. The
software renderer avoids building a second GPU stack. Codex CLI 0.162.0 or newer
is an external prerequisite, never bundled or silently installed.

The CI build job runs on this Mac with its persistent Cargo target cache. It
compiles each platform once, runs headless Rust tests, and uploads only binaries
and build receipts with compression disabled. It does not archive the multi-GB
incremental cache on every commit/tag, and Windows CI does not compile again.
This avoids the previous repeated compile and cache-upload delays. Initial cache
warming or dependency changes can still take longer.

Start these temporary runners after pushing the tag (two terminals):

```sh
python3 scripts/start-release-runner.py build
python3 scripts/start-release-runner.py signing
```

They run one trusted release job each and remove their GitHub registrations.
Only release-tag workflows can use them; the build verifies its source is on
main. No pull request jobs or Mac GUI tests run. The normal login keychain holds
the pinned Developer ID identity and AC_NOTARY; no Apple private keys are exported.
Apple Silicon is the rapid Mac prerelease architecture; imported universal-build
records under docs/upstream are historical.

Windows CI verifies input hashes/source/run, obtains Azure signing authorization
through OIDC, and signs fastrock.exe with SHA-256 and an RFC 3161 timestamp. It
requires the configured Sean McNamara publisher, runs fixture-only Rally UI checks
on that exact signed executable, then creates a stored ZIP with BUILD.json,
licenses, source pins and signing evidence. No unsigned fallback is published.
Hosted UI checks use `--ci` for deterministic board callbacks. The default suite
on `games` provides physical mouse drag coverage; both record their scope.

Mac CI verifies the arm64 binary and Apple-only dynamic dependencies, signs with
Developer ID Application SHA-1 `9A3CFFC04D3472208A62C48E707EA6D4261998A1`,
requires Accepted app and DMG notarization logs, staples the app before building
an uncompressed DMG, and validates stapling/Gatekeeper on both the DMG and its
mounted app. No JIT entitlements or embedded Codex runtime are needed.

Both packages have SHA-256 sidecars and GitHub build attestations checked against
the exact tag/source/workflow before upload. Windows publishes immediately after
its checks; Mac signing/notarization does not hold up Windows testing. Existing
asset names are never overwritten. Prior alpha.1–alpha.3 assets remain unsigned;
alpha.4 and later use this signed CI pipeline.

Check `df -h` on the Mac and `Get-PSDrive C` on games before larger builds. If free
space becomes low, remove identified inactive build outputs first; retain current
incremental caches and release evidence. Never stop VMs or unrelated compilers.
