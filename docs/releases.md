# Stable releases and rapid signed prereleases

## 0.4.0 development

Application code and dependencies use opt-level 2, no LTO or debug information,
256 codegen units, incremental compilation and no stripping. Build scripts remain
at opt-level 0. Goose and vwork are pinned, unmodified Rust dependencies;
release archives include their licenses. Existing 0.3.x binaries retain the
historical settings documented below.

Further binary publication is gated on feature-complete, verified release-candidate
readiness. Development source is committed and pushed to main; local fixture builds
are not releases. RC assets must pass signed CI acceptance. Mac builds check
compatibility without publishing Mac assets. Final 0.4.0 also requires the user's
live Bedrock profile validation; fixture acceptance does not establish live access.

The published alpha.1–alpha.3 builds are historical development milestones, not
release candidates. Their [prerelease notes](prerelease-notes.md) describe that
older scope. Current completion status and acceptance evidence are tracked in
`SPEC.md` and [verification](PORT-VERIFICATION.md).

## Stable 0.3.4

Conversation text accepts drags from adjacent padding. Tool results support
selection and native clipboard actions across visible virtual rows. Expanded
calls expose complete arguments/commands, including calls without output;
Copy All includes details and retained results. Fastrock-only update with no
vwork changes. Existing optimization/no-LTO settings and Windows signed ZIP
distribution continue, with a Mac compatibility compile.

## Stable 0.3.3

Conversation startup uses shared localhost vwork MCP with vwork 0.3.3 and an
installed Goose advertising HTTP MCP. Eligible opens/reloads no longer spawn
vwork clients. The native compaction action selects an entire database without
requiring a workspace. Both hops bind 127.0.0.1 and reject Origin/unexpected Host;
the daemon keeps bearer authentication. Older/custom transports retain stdio.

The compiler settings and Windows ZIP distribution match 0.3.2: application
opt-level 0, rendering/text allowlist at 2, LTO off, no debug info, 256 codegen
units and incremental builds. Mac compiles for compatibility. The installed
Goose/vwork Windows benchmark ran on games before the CI release; see
[verification](PORT-VERIFICATION.md#shared-vwork-transport-and-whole-database-compaction-2026-10-29).

## Stable 0.3.2

The scrolling performance release keeps application code at opt-level 0 and
optimizes only the explicit rendering/text dependency allowlist at level 2.
All profiles use `lto="off"`, including local ThinLTO. Debug info remains disabled;
256 codegen units, incremental compilation, and unstripped output are retained.
`dev/check-build-policy.py` rejects undeclared overrides. BUILD.json records the
exact optimized dependency list. Windows x64 ZIP publication continues; Mac is
compiled for compatibility only. CI signing, timestamping, tests and attestations
remain mandatory. `dev/windows-scroll-benchmark.py` measures the native software
renderer on games before release and exercises the signed binary in Windows CI.

## Stable 0.3.1

The requested maintenance release keeps the fast compiler profile and Windows x64
ZIP distribution. It adds vwork database compaction in the native UI. Mac is
compiled for compatibility; its release job remains disabled for this release.
Signing, hosted Windows tests, native acceptance and provenance remain mandatory.

## Stable 0.3.0

The user authorized a stable 0.3.0 release with the virtual-workspace feature.
It retains the fast development build profile: stability is independent of
compiler optimization. CI signs, verifies, attests and publishes the Windows
x64 ZIP as a normal latest release. The Mac build is checked but no Mac release
binary is published for 0.3.0. All Rust workspace test executables, including
the storage-neutral provider tests, run on Windows CI.

## Official 0.2.0

Sean explicitly authorized the official **0.2.0** build with optimization and
crash symbols. `cargo build --profile official` selects `opt-level=1`, `debug=2`,
`split-debuginfo="packed"`, `lto="off"`, `strip="none"`, 256 codegen units and
incremental compilation. Build scripts/proc macros retain fast settings. Use
`"off"` because Cargo's `lto=false` can still perform local ThinLTO at nonzero
optimization levels. Other/default build profiles and prereleases stay fast;
new stable version optimizations require explicit authorization.

The v0.2.0+build.1 tag publishes a normal GitHub release marked latest.
The +build.1 suffix identifies a packaging correction, not a prerelease; the
application version is 0.2.0. The original failed v0.2.0 tag is retained. CI chooses the
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

## Current prerelease workflow

Commit and push main, then push the matching prerelease tag. Do not create PRs
or publish locally built assets. For 0.4.0, `.github/workflows/release.yml` builds
both platforms, tests/signs/verifies/attests the Windows x64 ZIP and publishes it.
Mac is compile-only. Tags match Cargo.toml/Cargo.lock; no force pushes or asset
replacement. Existing releases retain their own compiler/distribution policy.

Use `python3 dev/check-build-policy.py` and `cargo build --locked --bin fastrock`.
Mac-to-Windows compilation uses `cargo xwin build --locked --target
x86_64-pc-windows-msvc --bin fastrock`. The software renderer avoids a second GPU
stack. Goose/vwork are embedded pinned dependencies; no installed backend is
required. Application/dependency opt-level is 2, build-script opt-level is 0,
LTO is off, debug info is off, codegen units are 256, incremental is enabled and
stripping is disabled. Environment overrides cannot alter the declared policy.

The temporary Mac build runner uses the persistent target cache, compiles each
platform and Windows test executables, and uploads binaries/receipts without
compressing the multi-GB incremental cache. It runs no tests or GUI. Start only
the build runner for 0.4.0 after pushing its trusted tag:

```sh
python3 scripts/start-release-runner.py build
```

It accepts one trusted release job and removes its registration afterward.
The separate signing runner is only needed for historical Mac-publishing releases;
0.4.0 must not start it. Release source must be on main; PR jobs cannot use these
runners. Preserve active incremental caches and release evidence.

Windows verifies input hashes/source/run, executes Rust and policy/watchdog tests,
uses Azure OIDC signing with SHA-256 and an RFC 3161 timestamp, and verifies the
configured publisher. Embedded Goose/vwork, Rally, selection, scrolling, search,
multiwindow navigation and recovery fixtures run on the exact signed executable.
All provider/Rally calls are loopback fixtures with isolated configuration.
Hosted `--ci` Rally moves use deterministic callbacks; games also supports the
physical mouse-drag fixture. Receipts record each fixture's scope.

Packaging includes BUILD.json, licenses, source pins, signing evidence, checksums
and GitHub attestations bound to the exact source/tag/workflow. No unsigned
fallback or replacement of published asset bytes is permitted.

Before release, run the full-scale comparison on games using
`dev/windows-scale-benchmark.py` and the standalone embedded-Goose seed fixture.
`dev/games-benchmark-monitor.py` supplies a live read-only hypervisor-memory
heartbeat. Native job limits cover only the fixture-owned process tree and stay
below both 32 GB and observed host/guest headroom. Missing/stale monitoring stops
the benchmark; aborts reduce the next workload tenfold. Preserve receipts and
state which workload completed. This benchmark controller does not run Fastrock
or tests on the Mac.

Check `df -h` on the Mac and `Get-PSDrive C` on games before larger builds. Clean
only identified inactive regenerable outputs when needed. Never stop VMs or
unrelated compilers.
