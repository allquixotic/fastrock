# Rust/Slint port verification

The Windows x64 fast build passed the fixture acceptance suite on `games` on
2026-10-10. All 676 headless Rust tests passed on the Mac. The Mac executable
compiled successfully; no GUI was launched on the Mac.

The original Rally layout and behavior accounting, source-function index and
R01–R22 implementation ledger are in [RALLY-INVENTORY.md](RALLY-INVENTORY.md).
The imported codex-gui revision is `be94e52cc21c5d262805e044e75267885bc73a3d`;
the preserved Go baseline is `f523c6b7ae6f9b6a12ca951845b56d81cb76a879`.

## Commands and outcomes

| Check | Outcome |
|---|---|
| `python3 dev/check-build-policy.py` | Passed; dev/release/test and build-script settings, flags and entrypoints checked |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |
| `cargo test --locked --lib -- --skip window_runtime::tests` | 676 passed, zero failures, one GUI test filtered out on the Mac |
| `cargo xwin build --locked --target x86_64-pc-windows-msvc --bin fastrock` | Passed, dev/unoptimized; final incremental build 1 minute 20 seconds |
| `python dev/windows-rally-smoke.py fastrock.exe`, interactive session on `games` | Passed; 82 fixture requests and 13 explicit writes |
| Hosted Windows [run 38036468741](https://github.com/allquixotic/fastrock/actions/runs/38036468741), alpha.2 source | 667 Rust tests, compilation, CI-mode fixture acceptance and packaging passed; incremental cache saved |
| `cargo build --locked --bin fastrock`, Mac | Passed, dev/unoptimized; final incremental build 1 minute 24 seconds |
| Installed Mac Codex CLI 0.162.0, read-only stdio initialize/config/model-list | Passed; seven models returned, no inference or Rally writes |

Toolchain: Rust 1.95.0; native Slint 1.18.1 software renderer. Timings describe
these machines and their existing caches, rather than a promised cold-build time.
Two retained upstream compatibility/dead-code warnings remain on Windows; the
Mac also reports an unused platform-specific interruption helper. No compile
errors remain.

The exact executable exercised on `games` has SHA-256:

```text
e816033be6f99eeac89271e7a2610f2a0132aa2159e783e2fa42fd6aec6bfe49
```

`BUILD.json` inside each package records the source commit, dirty status, binary
hash, profile, upstream pins and external Codex prerequisite. A GitHub-built
binary has its own hash and fixture run. The packaging script never recompiles
or optimizes; ZIP entries are stored without a compression pass.

## Native Windows coverage

The suite launches the actual Slint executable with a loopback WSAPI fixture and
an external stdio Codex fixture. It records requests without authorization
headers and retains screenshots, state dumps, session files, an executable hash
and `acceptance.json` under `build/rally-smoke` on Windows. The local final copy is
`build/rally-windows-alpha3-evidence`. These generated outputs are ignored by Git.

Coverage includes horizontal document tabs, a chats-only sidebar, list paging,
300-card board paging, mixed concrete artifact types, vertical Owner swimlanes,
card-field settings, relative ranking and Undo. A Windows mouse gesture actually
drags a card between workflow lanes. A native User-picker button click is checked
for closing the picker and supplying a typed reference to bulk review.

The remaining settled callback scenarios cover typed edit/save, create with the
Completed-lane default, related Tasks/Attachments/Revisions/Discussions reads,
posting a discussion, rich HTML source save, sparse mutation responses, late
edits during delayed normal and Save-and-continue saves, retained in-flight write
guards on invalid input,
paging to the created artifact, dirty-navigation
Save/Discard/Cancel, private-view snapshots, unapplied-filter isolation, row
hide/restore, reviewed bulk updates, an assistant proposal followed by explicit
human Apply, narrow layouts, restart with unsaved fields/comments, and popout
plus transfer back with drafts and filters preserved. Assertions inspect the
corresponding request payloads and persisted state rather than button names alone.

Hosted runners use `--ci` to drive the board move callback rather than an OS mouse
gesture. The default suite on `games` still requires the real gesture. Alpha.1's
hosted run passed 665 Windows Rust tests and compilation, then failed its OS-drag
assertion. The follow-up hosted run passed all 667 Windows Rust tests and the
callback-mode fixture suite (80 requests/12 writes), then packaged successfully
and saved its incremental cache. Its source precedes the final Save-and-continue
fix; alpha.3 passed both fixture modes on `games` with 82 requests/13 writes.
The acceptance receipts explicitly record their modes, and CI retains evidence
even on failure.
The final alpha.3 hosted run 38039008786 also passed 667 Windows Rust tests,
the complete 82-request/13-write callback fixture suite, packaging and cache save.

HTTP and pure Rust tests also exercise complete collection paging, full-query
CSV scope and formula neutralization, malformed or changing pages, reference and
type guards, token redaction, mutation non-retry, revision conflicts, attachment
size/download/orphan cleanup, stale cache invalidation, three-way merge, partial
batch outcomes, acknowledgement/collection-summary/late-edit merging and legacy
settings/session migration. Transport tests cover
timeouts, cancellations, bounded frames/events and disconnect cleanup.

![Native workflow board](images/rally-board.png)

![Native detail editor](images/rally-detail.png)

![Narrow document with assistant below](images/rally-narrow.png)

## Scope and distribution

The alpha.3 recorded above is an unsigned Windows x64 alpha. It requires installed Codex CLI 0.162.0 or
newer; the installed CLI owns inference and configuration. `games` did not have
Codex on its interactive PATH, so its UI tests used the explicit external-process
fixture. The real installed CLI was checked read-only on the Mac.

No real Rally endpoint or token was used and no real Rally write was made.
Attachment file dialogs and individual DPI scaling configurations are implemented
but were not driven in this acceptance run. Vault key compatibility was checked
without replacing live credentials. Rich source preserves HTML beyond the
supported native formatting subset. Revision preflight cannot make WSAPI writes
atomic, and batch outcomes deliberately describe partial success.

Free space remained approximately 2.1 TiB on the Mac and 3.06 TB on `games`.
No cleanup was needed; useful incremental caches were retained. The build policy
and disk-space practice are persistent in AGENTS.md and docs/releases.md.

## Signed CI release: alpha.4

[v0.2.0-alpha.4](https://github.com/allquixotic/fastrock/releases/tag/v0.2.0-alpha.4)
was built, signed, verified, attested and published by
[CI run 38048069773](https://github.com/allquixotic/fastrock/actions/runs/38048069773),
source `32ab8aeb93dcd7401c32fc3392a19178daf01286`. The port was pushed directly
to main with existing history preserved. Future releases use this CI workflow;
PRs and locally published release binaries are excluded by AGENTS.md.

The build job used this Mac's persistent incremental cache and completed in
5m26s, including both platform binaries and 676 passing headless Rust tests.
Every invocation reported an unoptimized profile. The Windows job did no Rust
compilation and published its signed ZIP immediately after its checks. CI did
not archive the multi-GB build cache or compress binary artifacts/packages.

Windows Authenticode status was Valid for
`CN=Sean McNamara, O=Sean McNamara, L=Pasadena, S=MD, C=US`, certificate SHA-1
`59DE4BFCDD5B9C3B31CB3C591B2755BDA0969F07`, with Microsoft RFC 3161 timestamping.
The exact signed executable SHA-256 is
`72a87578ba6fbe0ed6beaeb258c2c269c1ccd5ba711aeb42f8417a3714103d3f`.
CI's native callback suite passed 82 requests/13 fixture writes on those bytes.
The downloaded published ZIP was independently checked against its SHA-256,
BUILD.json and GitHub attestation. On `games`, the extracted executable's
Authenticode signature was independently Valid, and the full physical mouse
suite passed 82 requests/13 writes, including native user-picker clicks.
ZIP SHA-256:
`bb37e4d65de22c81cebfa8b557df4f67df6c037d2518da009c45f98234b93b31`.

The Apple Silicon Mac app and uncompressed DMG use Developer ID Application
SHA-1 `9A3CFFC04D3472208A62C48E707EA6D4261998A1`, team `B6XDYNLMPU`, secure
timestamps and hardened runtime, without JIT entitlements. Its dependencies
were Apple system libraries only. Apple notarization submissions were:

- App: `87b0224b-f985-410b-bbd9-cd1312993a7f` — Accepted; log inspected.
- DMG: `32f269db-b4bf-464c-a4c0-1d5e4b12a8dc` — Accepted; log inspected.

CI stapled/validated the app before creating the DMG, then signed/notarized/
stapled/validated the DMG. Gatekeeper accepted the app, DMG and mounted app.
The final published DMG's checksum and GitHub attestation were independently
verified after download. DMG SHA-256:
`73a0092a79359158f1fbd0a70045fdd1cac5662b0ff467bf7888eace3a0c27c7`.
No Mac GUI was launched. Both temporary CI runners removed their registrations;
the useful incremental cache was retained. Mac free space was about 2.4 TiB;
`games` about 3.06 TB. No cleanup was required.

## Rally enum metadata regression: alpha.5

The reported Team Board error matches a client-side metadata bug: a generic
collection duplicate check treated each built-in enum's literal `_ref: "null"`
as the same object identity. Board loading queries both artifacts and their
schema/workflow; the error can therefore originate from dropdown metadata.
[Broadcom KB 57584](https://knowledge.broadcom.com/external/article/57584/wsapi-api-get-allowedvalues-for-rally.html)
shows distinct `StringValue` choices with this shared placeholder and complete
catalog responses using `StartIndex: 0` / `PageSize: 0`. Previous fixtures omitted
these response forms. No production subscription or token was accessed.

The fix applies only to `AttributeDefinition/<signed nonzero ID>/AllowedValues`.
It preserves all anonymous choices and normalizes a zero index only for the
first complete catalog. Duplicate persisted references, partial catalogs,
ordinary zero-index pages and repeated later pages remain rejected. Existing
workspace scoping and same-origin reference checks are preserved.

Four named `rally::client::tests::v24_*` regressions cover enum preservation,
unpaged response handling, real duplicate rejection and Team Board artifact +
schema + workflow loading. Before the fix, three failed with the duplicate/page
errors and the persisted-duplicate guard passed. After the fix, all 680 headless
tests passed; the Mac GUI test was excluded. Fast build policy, formatting,
Python syntax and diff checks passed. The Windows fixture now returns the
documented enum response and asserts every Team Board workflow lane survives.

Local headless evidence: `build/alpha5-headless.log`. Signed CI release receipts
are recorded after publication below.

[v0.2.0-alpha.5](https://github.com/allquixotic/fastrock/releases/tag/v0.2.0-alpha.5)
was built, signed and published by
[CI 38077735206](https://github.com/allquixotic/fastrock/actions/runs/38077735206),
source `311c2e4b1a362744e96b1750c35bd9106ad2aa09`. All three jobs passed.
Both platform binaries plus 680 headless tests completed in the 4m03s build
job, using the persistent cache and unoptimized profiles. Windows signed
fixture acceptance and independent physical mouse acceptance on `games` each
passed 82 requests / 13 fixture writes, explicitly recording
`realistic_allowed_value_metadata: true`. All four Team Board workflow lanes
survived; edit/drag/rank/undo/restart/window-transfer checks also passed.

The downloaded published Windows executable independently verified as Valid
Authenticode for Sean McNamara, with Microsoft RFC 3161 timestamping.
Executable SHA-256:
`abec190c7238ceea15b820be6d5aa74ddda1af01540bec8093bcd43bed420f51`.
ZIP SHA-256:
`f3c2b7419f30e47c2fb4f49444235b7d5bbe6fff35bcc933ae18332e569f36fb`.

The Mac app and DMG used pinned Developer ID
`9A3CFFC04D3472208A62C48E707EA6D4261998A1`, team `B6XDYNLMPU`.
App submission `2bcc0d2a-06a0-4190-814d-e11fcd555939` and DMG submission
`bed2a1a4-6f27-40a2-9f4a-a0834ad29081` were Accepted; both retrieved logs
had no issues. App and DMG stapling validated; Gatekeeper accepted the app,
DMG and mounted app. Published DMG SHA-256:
`60fcdc9e210b0e256c864dfb7bff1bb69ea6f66aff1150eee11ffebd5157006d`.

Both downloaded package checksums, Windows BUILD.json and GitHub attestations
were independently verified against the exact tag/source/workflow. Evidence:
`build/ci-alpha5-evidence/` (CI logs, signature/notarization records, hosted and
physical Windows fixture receipts) and `build/ci-alpha5-download/` (published
bytes and attestations). Both temporary runners deregistered; zero remained.
No production Rally access/writes or Mac GUI launch occurred. Mac free space
was about 2.4 TiB, games about 3.06 TB; active incremental caches retained.

## Official optimized 0.2.0

[Fastrock 0.2.0](https://github.com/allquixotic/fastrock/releases/tag/v0.2.0%2Bbuild.1)
is a published, non-draft, non-prerelease GitHub release marked latest. The
application version is exactly `0.2.0`; tag `v0.2.0+build.1` identifies the
packaging correction. Source: `780dfade69930ba2f325f2326f719d927609bf5b`.
[CI 38101740788](https://github.com/allquixotic/fastrock/actions/runs/38101740788)
passed all three jobs and published all ten assets.

Sean explicitly authorized `opt-level=1`, full `debug=2`, packed debug symbols,
`lto="off"`, `strip="none"`, 256 codegen units and incremental compilation for
this official version. Captured rustc invocations confirm optimization, debug
information and LTO settings for the application and protocol/config crates.
`lto="off"` disables local ThinLTO too. Other/default profiles and prereleases
remain unoptimized and prioritize compile speed.

Initial compilation took 6m28s for Windows and 6m49s for Mac. The first CI run
passed 680 headless tests but rejected the valid Mac dSYM because Cargo retains
a hash-suffixed filename inside it. The checker now resolves the DWARF file and
verifies UUID plus nonempty debug-info/line sections; a hashed-name regression
covers this case. The original `v0.2.0` tag was preserved. The corrected run
reused both binaries (0.88s/0.82s Cargo checks), completed its build job in 37s,
passed all 680 headless tests and four release-policy/symbol tests, and required
no second optimization pass. Windows signing/testing/publication took 5m56s;
Mac signing/notarization/publication took 3m42s.

Windows Authenticode and Microsoft RFC 3161 timestamp verification passed in
CI and independently on `games`. Publisher:
`CN=Sean McNamara, O=Sean McNamara, L=Pasadena, S=MD, C=US`;
certificate SHA-1 `ADBCA762010281BC5063C83068FD17E37C1EF6D2`.
Signed executable SHA-256:
`b0fe734f88abd724fc20549df0d9447fb92e67e0747a71d5a87b62ce48f34ec1`.
Hosted callback tests and physical mouse tests on the downloaded executable
each passed 82 requests / 13 fixture writes. The physical run confirms native
dragging and user-picker clicks; both runs record realistic allowed-value
metadata and cover the Team Board regression, editing, restart and window
transfer. The physical swimlane screenshot was also inspected.

The downloaded Windows PDB matches CodeView GUID
`b202a3cf-ddba-3329-4c4c-44205044422e`, age 1. LLVM independently confirms debug
information, types, globals and publics are present and the PDB is not stripped.
The downloaded Mac dSYM matches the signed app inside the published DMG:
UUID `BED72D69-F3F4-3DEF-BB21-9A9C1FBD0B95` (arm64), with nonempty DWARF info
and line sections. Symbol manifests identify the same source, workflow run,
compiler settings and signed executable hashes as the application packages.

Mac Developer ID SHA-1 `9A3CFFC04D3472208A62C48E707EA6D4261998A1`, team
`B6XDYNLMPU`, was used throughout. Apple accepted app submission
`21ea83c3-f16b-461d-8953-166df262d5d8` and DMG submission
`f2e52c47-da77-4968-bf70-b677768b29d4`; both logs have no issues. CI verified
stapling and Gatekeeper on app, DMG and mounted app. Independent checks of the
downloaded DMG verified signatures, app/DMG staples and Gatekeeper again.
Signed Mac executable SHA-256:
`64a2fb081ec3dd0ebe2f6c57dc981e6d7ba6b92296c99414e532dc0602a8838c`.

All four downloaded packages match their SHA-256 sidecars and GitHub asset
digests. Their bundled GitHub attestations independently verify against the
exact source, tag and `.github/workflows/release.yml`:

| Asset | Bytes | SHA-256 |
| --- | ---: | --- |
| Windows application ZIP | 65,034,804 | `b5754875a8fecc519f46bee7497cabd863c121664b83c0bcf5be393c981cdc2f` |
| Windows PDB ZIP | 1,376,967,419 | `4c60adf3a23c047971bad90749865fde7e756a8ebff0aaf83d0ed9a363696e7c` |
| Mac application DMG | 124,643,821 | `498073ae09abd50d285cf4e36dd538a33725644bbe275ba3279a460a4c25921b` |
| Mac dSYM ZIP | 1,025,275,158 | `79dd66e95e773de7133814d31d97df268039075461ebc63727b4d92ae2c6e552` |

Evidence is retained in `build/ci-020-evidence/` (CI, signing, symbols,
attestations and physical fixture receipts) and `build/ci-020-download/`
(published bytes). Both temporary runners removed their registrations; zero
remained. No Mac GUI or production Rally calls/writes occurred. Mac free space
remained about 2.3 TiB and games about 3.06 TB; active caches were retained.
