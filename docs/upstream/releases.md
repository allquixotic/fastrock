# Signed releases

Windows main-push builds use GitHub-hosted runners, Azure OIDC login and the
`allquixotic-signing/windows-public` certificate profile. PR builds remain
unsigned. Verify all four EXEs with `Get-AuthenticodeSignature`: status `Valid`,
publisher `CN=Sean McNamara, O=Sean McNamara, L=Pasadena, S=MD, C=US`, and a
timestamp certificate. Packaging follows signing; the ZIP is then attested.

Mac releases use `.github/workflows/macos-release.yml`, dispatched on main
only, and a temporary ARM64 Mac Actions runner labeled `codex-gui-signing`.
Register it with `--ephemeral --disableupdate`, run it interactively for one
job, and confirm GitHub removes it afterward. Do not install a runner service
or export private keys. The runner checkout is separate from the working repo.
Only register/dispatch this runner for an explicitly requested release.

The Mac must have Xcode tools, Python 3.11+, Rust 1.95.0, the two Mac Rust
targets, Rosetta for Intel IPC validation, the pinned 2031 Application signing identity/private key, and the
`AC_NOTARY` keychain profile. `scripts/build-macos.sh` builds a universal
arm64/x86_64 GUI plus the pristine upstream code-mode helper for macOS 14+.
Mac tests explicitly skip `window_runtime::tests`; never launch the GUI there.

`scripts/package-macos.py` inventories native code, rejects non-system dynamic
dependencies, signs the helper and GUI inside out, and enables hardened runtime.
Only the V8 helper receives the stable upstream `allow-jit` and
`allow-unsigned-executable-memory` entitlements; Intel V8 requires the latter.
The GUI receives neither, and no executable disables library validation or
enables debugger access. IPC smoke tests force both architectures and run
JavaScript in the signed helper without starting the GUI. The early
`dev/macos-packaging-check.sh` gate checks real Apple lipo slice verification and
the exact helper entitlement inventory before compilation. Build with
`LZMA_API_STATIC=1` so local pkg-config libraries cannot leak into the bundle;
`verify-macos-dependencies.py` rejects every non-Apple dynamic dependency. Sign with Application SHA-1
`9A3CFFC04D3472208A62C48E707EA6D4261998A1`, team `B6XDYNLMPU`, expiring
September 17, 2031; never select an ambiguous certificate by name. No PKG is built.

The script notarizes a temporary app ZIP, records its submission ID and checks
Apple's Accepted status and log. It staples and validates the app and passes
Gatekeeper before constructing the DMG. It then signs, notarizes, staples and
assesses the DMG, mounting it read-only with `-nobrowse` to verify the contained
app's signature, ticket and version. Evidence contains public metadata only.
If interrupted, use `notarytool info/wait/log` with the recorded submission ID;
never upload an unchanged rejected artifact again.

GitHub's pinned `actions/attest` creates build provenance for each final ZIP/DMG,
after all platform signing/stapling. Both workflows export Sigstore bundles;
retain them beside release packages for offline verification. Attestations
prove workflow/source provenance; platform signatures identify the publisher.
Checksums are computed after all mutations. Do not claim a self-hosted build
has the isolation guarantees of a GitHub-hosted runner.

Before publication, download artifacts from successful runs of the exact same
commit and verify checksums, build.json version/upstream/commit, platform
signatures and provenance:

```sh
gh attestation verify codex-gui-VERSION-windows-x64.zip --repo allquixotic/codex-gui
gh attestation verify codex-gui-VERSION-macos-universal.dmg --repo allquixotic/codex-gui
```

Also constrain verification to the expected source digest and signer workflow
using `--source-digest` and `--signer-workflow`. Save verified JSON with
`--format json`. Mac provenance uses a self-hosted runner: do not require the
`--deny-self-hosted-runners` policy for that package. Verify the Windows package
with that policy. Test the signed Windows package on Windows, never on the Mac.

Create a draft release pinned to the successful full SHA, upload both packages,
checksums and platform Sigstore bundles, and verify GitHub asset digests against
local bytes before publishing. Recheck latest stable upstream. Remove temporary
runner registration/credentials; clean only this project's regenerable build
outputs after all assets are verified. Preserve signing/notarization evidence.

## Version 0.4.0

Published [v0.4.0](https://github.com/allquixotic/codex-gui/releases/tag/v0.4.0)
on October 8, 2026 (October 9 UTC), from source
`1b512fecad7e28826ef98538a69dd9b05327275a`. Upstream is stable Codex 0.162.0,
revision `c1382380de69521303b416720a52f42d51af6248`.

The [Windows workflow](https://github.com/allquixotic/codex-gui/actions/runs/37874908459)
passed 633 tests and all interaction/package/signing gates. The exact downloaded
ZIP passed question forms, purpose/selection and conversation suites on `games`
with software and GPU renderers, including Queue/Steer, pending-message
edit/delete/races, sidebar tooltip bounds and stable activity after reopening.
All four Authenticode signatures had the expected publisher and timestamps;
the bundled code-mode helper executed JavaScript through its native IPC.

The [Mac workflow](https://github.com/allquixotic/codex-gui/actions/runs/37874969609)
passed 642 headless tests; strict Clippy passed locally. No Mac GUI was launched.
The universal arm64/x86_64 app and helper use Application certificate SHA-1
`9A3CFFC04D3472208A62C48E707EA6D4261998A1`, team `B6XDYNLMPU` (expires
September 17, 2031). Actual signed helper IPC/JIT passed on both architectures.
Apple accepted app submission `e5fc97d0-2ade-408a-94d1-83e5b24e6ab0` and DMG
submission `18432a61-2d51-4a88-8621-6c27d39cecdd`; both logs had no issues.
App and DMG staples, Gatekeeper, mounted app, signatures, architectures and
embedded build metadata passed independent verification.

Online and bundled GitHub attestations matched the exact source, main ref and
signer workflows. Windows additionally passed the hosted-runner policy. All
eight public release assets matched local SHA-256 digests after publication.
Public signing records accompany the packages. The temporary Mac runner
deregistered and its checkout was removed; final artifacts and detailed evidence
remain in ignored `dist/release-0.4.0`.

```text
7a120a888da0d0ef54c12164041aa4d4afb67d65a8c0649c1fb4004f9e2ada46  codex-gui-0.4.0-windows-x64.zip
a4b79651c38a762decf0f8b9c5515dda7b8fffda5676459ea01e901227d78401  codex-gui-0.4.0-macos-universal.dmg
```
