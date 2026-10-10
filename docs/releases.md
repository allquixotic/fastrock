# Rapid prereleases

Every Rust profile favors compile speed: opt-level 0, debug 0, split-debuginfo off,
LTO false, codegen-units 256, incremental true, strip none. Build dependencies also
use opt-level 0 and 256 codegen units. No environment override may re-enable
optimization or debug information. `dev/check-build-policy.py` verifies this policy
before supported scripts and CI. Sean must explicitly authorize any exception.

Build with `cargo build --locked --bin fastrock`; never add an optimization pass.
`scripts/build-windows.ps1` and `scripts/build-macos.sh` enforce the same settings.
For Mac-to-Windows testing, use `cargo xwin build --locked --target
x86_64-pc-windows-msvc --bin fastrock`; its binary is in the target's `debug` folder.
Do not launch GUI tests on Sean's Mac.

`.github/workflows/windows.yml` caches Cargo artifacts, checks policy/formatting,
runs Rust tests and the fixture-only native UI suite, and packages the development
binary. Pushes and PRs upload artifacts. Tags matching `v*-alpha.*`, `v*-beta.*` or
`v*-rc.*` publish GitHub prereleases after those checks succeed. No production
Rally endpoint or token is used by CI.

Hosted UI checks pass `--ci` to exercise board callbacks without depending on an
OS pointer gesture. Before publishing locally, run the default suite on `games`
for real Windows drag coverage. Both modes record their scope in acceptance.json;
CI retains fixture evidence on failure as well as success.

The cache retains the target directory, including workspace incremental artifacts,
without changing CARGO_INCREMENTAL. A new tag can restore the default branch's
cache; GitHub does not share caches between distinct tags. Keep the default branch
cache warm after landing the port. The first hosted build needs to compile its
dependencies. See [GitHub's cache scope rules](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache).

To package an already tested binary:

```sh
python scripts/package-prerelease.py target/debug/fastrock.exe dist
```

The ZIP uses stored entries to avoid compression waits, includes licenses, the
Rally accounting, source pins and BUILD.json, and gets a SHA-256 sidecar. Packaging
does not compile or optimize. Windows CRT is static. Codex CLI is an external
prerequisite, never bundled or silently installed. Current alpha packages are
unsigned; signing is separate from Rust compilation and must be described accurately.

Check `df -h` on the Mac and `Get-PSDrive C` on games before larger builds. If free
space becomes low, remove identified inactive build outputs first; retain current
incremental caches and release evidence. Never stop VMs or unrelated compilers.

Historical codex-gui release/signing notes are preserved under `docs/upstream/`.
Their signing and test results do not verify Fastrock alpha builds.
