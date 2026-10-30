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


## Goose ACP migration — 0.3.0-alpha.1

The default backend launches installed Goose ACP over stdio. Source inspection of
Goose v1.56.0 confirms independent sessions and prompt tasks within one process;
`goose serve` uses the same ACP handler. See [architecture](goose-acp.md).
No Goose crate was added to Cargo.lock. Both platforms use the fast dev profile.

Windows verification on `games` covers the full Rust suite (681 passed; the one
window-runtime unit test is excluded in favor of native UI acceptance), including
11 Goose adapter tests. Three release-policy tests pass; the Apple-only symbol
fixture is skipped on Windows. Native automation uses a separate Python ACP
process and loopback Rally/MCP fixtures, with no real inference or Rally writes.
Two conversation tabs stream independently; stopping the first does not stop the
second. Screenshots verify the native Goose option controls. The Rally suite
covers 82 HTTP requests and 13 explicit fixture writes, proposal review/Apply,
drafts, restart and window transfer. `--ci` uses native board callbacks: no
physical mouse test was possible because no interactive user was signed in.

CI repeats the Rust and native Goose/Codex UI checks on Windows, on the exact
signed release executable for the UI runs. The Mac only compiles and performs
signing/notarization verification; it runs no tests or GUI. CI evidence includes
`goose-chat-smoke`, `goose-rally-smoke`, `rally-smoke`, Windows test output and
signature receipts. A live provider account has not been exercised.

Release verification completed 2026-10-26:

- Source commit: `b611ffe3c2eb87c30cab1ecd8ce77cfa6121a73e`.
- [Signed CI run 40267148011](https://github.com/allquixotic/fastrock/actions/runs/40267148011): build, Windows and macOS jobs succeeded.
- [Published v0.3.0-alpha.1](https://github.com/allquixotic/fastrock/releases/tag/v0.3.0-alpha.1): Windows x64 ZIP and Apple Silicon DMG, each with checksum and attestation.
- Windows: 681 Rust tests passed; Goose chat, Goose Rally and Codex Rally UI
  acceptance passed. Every UI receipt matches signed executable SHA-256
  `b88a380f5703ef6764fb7013733607f6cf7e56583bbee2d74157abcffe438a36`.
  Authenticode publisher is Sean McNamara; Microsoft RFC 3161 timestamp verified.
- Mac: app and DMG notarization both Accepted; signing, stapling and Gatekeeper
  verification passed without launching the app. Both temporary CI runners were
  removed after completion. Local evidence is retained under
  `build/goose-verification/`; CI artifacts retain the published-source receipts.

## Stable 0.3.0 virtual workspaces — 2026-10-26

- [Stable release v0.3.0+build.1](https://github.com/allquixotic/fastrock/releases/tag/v0.3.0%2Bbuild.1)
  is public, latest and not a prerelease. Windows x64 ZIP only; Mac compilation
  passed without running Mac tests or GUI, and the Mac publication job was skipped.
- [Signed CI run 40382229798](https://github.com/allquixotic/fastrock/actions/runs/40382229798)
  succeeded from `3002f35fb14bb00690930db4384efd0d12585cec`. The temporary build
  runner was removed. Fast build settings remain opt-level 0, debug 0, no LTO.
- Windows: 690 application tests, five provider tests and four release-policy
  tests passed. One live-vwork test remains opt-in/ignored; one existing window
  runtime test is skipped in the headless test executable. Native Goose chat,
  Goose Rally, Codex Rally and virtual-workspace UI acceptance all passed on the
  exact signed executable. Hosted Rally uses callback-based board interaction.
- Virtual workspace evidence verifies the native window, preserving text on a
  revision conflict, a successful revision-checked save, and hidden controls with
  vwork disabled. The UI screenshot was inspected after downloading CI evidence.
- Azure Artifact Signing `allquixotic-signing/windows-public` produced valid Sean
  McNamara Authenticode signing and a Microsoft RFC 3161 timestamp. Certificate
  SHA-1: `0063631B695A60361BF66A861649BD0CB7F91CB2`. Signed executable SHA-256:
  `61dfb942633dc414bafd8f68423bafdc455965767b021bcab5b34a5841e27067`.
- ZIP: `fastrock-0.3.0-windows-x64-fast.zip`, 145,777,592 bytes, SHA-256
  `c830f129682c798ff0a91e9b429ffa40e68157d60490d781ba7309d917b33200`.
  Downloaded bytes match the checksum, GitHub asset digest and embedded signing
  receipt; GitHub provenance verification passed for the exact source/tag/workflow.
- [S3 fastrock.zip](https://fastrock-dl-582961286906-us-east-1-an.s3.us-east-1.amazonaws.com/fastrock.zip)
  contains identical bytes. The explicitly invoked ship-to-aws helper verified
  anonymous HEAD and full-download SHA-256, REDUCED_REDUNDANCY storage and denied
  anonymous bucket listing. Failed tag v0.3.0 remains unchanged; B72 records the
  runner PSGallery registration fix included in the successful build tag.

## 0.3.1 vwork maintenance — 2026-10-26

- [Release v0.3.1](https://github.com/allquixotic/fastrock/releases/tag/v0.3.1)
  and [signed CI 40425525614](https://github.com/allquixotic/fastrock/actions/runs/40425525614)
  succeeded from `58fa5da5014d120b6e45e57d32f5bee12d874da1`. Windows x64 fast ZIP;
  Mac compatibility compilation passed with no local Mac tests/GUI. The temporary
  build runner was removed and its incremental cache preserved.
- 691 application tests, six workspace-provider tests and four release-policy
  tests passed. The existing live-vwork opt-in test remains ignored and the
  headless-incompatible window-runtime test remains skipped. Signed native
  Codex Rally, Goose Rally/chat and virtual-workspace acceptance passed.
- The compact action calls the selected store in the background, shows measured
  reclaimed bytes or skip/error reasons, and preserves unsaved drafts. Native
  fixture evidence and the inspected screenshot show the button, successful
  compaction report and preserved draft; disabled vwork hides the feature.
- Authenticode publisher: Sean McNamara; Microsoft RFC 3161 timestamp present.
  Certificate SHA-1 `F65E5B72C3747B4450FC367E1AD28B15F1418729`.
  Executable SHA-256:
  `d8e6044139aa229ea200ae2464f2a5e65273ee728e4001e2e5cb72cf2e9db750`.
- Downloaded ZIP `fastrock-0.3.1-windows-x64-fast.zip`: 145,959,360 bytes;
  SHA-256 `10648b312c46ec89993aa4dbd8a81db44f66742303813651c6362f36031e1304`.
  GitHub asset digest, checksum and signing receipt match. GitHub provenance
  verified against the exact source, tag and release workflow.
- Explicit ship-to-aws upload overwrote
  [fastrock.zip](https://fastrock-dl-582961286906-us-east-1-an.s3.us-east-1.amazonaws.com/fastrock.zip)
  with identical bytes using REDUCED_REDUNDANCY. Anonymous HEAD size/class and
  full-download SHA-256 passed; anonymous bucket listing remained denied (403).

## 0.3.2 scrolling performance — 2026-10-26

Development benchmark ran in the signed-in console session on `games` (Windows 11
26100, software renderer). Identical local ACP fixtures and native wheel events
were used before and after; 360 input ticks per scenario at 1440×1000/100%,
120 at 2560×1600/200%. First ten samples were excluded. Renderer timings include
rasterization and presentation; event-loop intervals additionally expose layout
and dispatch stalls. The 16.667 ms input timer is a pacing target, not an input-to-
photon latency measurement. No inference or real Rally requests were used.

Baseline is c7374dd behavior plus in-memory diagnostics, all opt-level 0 and the
original full-window invalidation. Candidate retains application opt-level 0,
optimizes 17 rendering/text dependencies at level 2, disables all LTO (including
local ThinLTO), retains a private Windows framebuffer, and virtualizes tables in
eight-row chunks and tool/command/diff output in sixteen-line chunks.

| Scenario | 100% frame p95, before → after | 200% frame p95, before → after | 100% event-loop interval p95, before → after |
|---|---:|---:|---:|
| prose | 63.90 → 7.45 ms | 120.51 → 13.20 ms | 72.49 → 17.24 ms |
| table | 56.93 → 6.65 ms | 81.45 → 9.94 ms | 140.62 → 18.23 ms |
| code | 41.00 → 5.43 ms | 81.69 → 9.55 ms | 42.29 → 17.26 ms |
| tools | 45.36 → 6.79 ms | 87.66 → 9.13 ms | 47.17 → 17.39 ms |
| streaming | 68.76 → 7.75 ms | 125.28 → 12.83 ms | 81.84 → 17.20 ms |

Baseline executable SHA-256: `8d6fe39c2193d9ca7d7fe9e1becf0c1c7ead025a033f835be1ee5bcb731c0f43`.
Candidate executable SHA-256: `6604f3a25e7c5b04e7aae484a31b44ba29c10043e91c0f0d98d4fd72964a004a`.
All baseline frames dirtied the entire window; candidate scrolling typically
dirties only the conversation region. Presentation still copies every pixel to
Windows so RDP buffer loss cannot expose stale pixels. Private storage costs
approximately four bytes per physical window pixel.

Raw frames, timing summaries, snapshots and local binaries are retained in
`build/scroll-benchmark/` and `games:C:/Users/SeanMcNamara/fastrock-scroll-032/`.
The reusable fixture is `dev/windows-scroll-benchmark.py`. These are development
measurements; hosted CI runs a shorter acceptance fixture on the signed executable,
without imposing machine-independent timing thresholds.

Windows regression evidence: 695 application tests plus the isolated rendering
test passed; six provider fixture tests passed, one live-vwork test stayed ignored.
Three release-policy tests passed; the Apple symbol fixture was skipped on Windows.
Rendering tests corrupt the presentation buffer, verify unchanged content needs
zero rasterization, verify a small visual change remains a partial repaint, and
check resize storage, links, clipping and selection. Native benchmark exercises
minimize/restore. Actual RDP reconnect was not driven; surface loss is injected
deterministically. Mac compatibility compilation passed; no Mac tests or GUI ran.

Native acceptance on the candidate also passed both Codex and Goose Rally fixtures
(82 local requests and 13 explicit fixture writes each, real Windows mouse board
moves), concurrent Goose chat/cancel/config controls, and virtual-workspace
conflict/save/disabled-controls checks. All receipts bind to the candidate hash.

### Published CI artifact

[Signed release CI run 40441558745](https://github.com/allquixotic/fastrock/actions/runs/40441558745)
passed and published [v0.3.2](https://github.com/allquixotic/fastrock/releases/tag/v0.3.2)
on 2026-10-27 UTC from `d38db9e890025cc514a244d6e7e89310f9ff60eb`.
Windows ran 695 application tests, the isolated renderer test, six provider tests,
and release-policy checks. The live-vwork test stayed ignored. Mac compatibility
compilation passed; Mac publication was skipped. The temporary build runner was
deregistered after completion.

The exact signed executable passed CI's Codex/Goose Rally, concurrent chat,
virtual-workspace and five-scenario scrolling fixtures. Hosted Rally board moves
used callbacks; the pre-release games checks above exercised native mouse moves.
CI scrolling p95 render-plus-present times ranged from 6.64 to 12.53 ms at
1440×1000/100%; these hosted-runner figures are separate from the games comparison.

Downloaded `fastrock-0.3.2-windows-x64.zip` SHA-256 matched both the published
sidecar and GitHub asset digest:
`aa4d3e2f0ccee342f2bef205a231f9134c73b8d24f86a30ccba9c95134398106`.
GitHub attestation verification passed with repository, release workflow, tag and
source commit constraints. BUILD.json confirms a clean source tree, all 17
authorized dependency optimizations and `lto="off"`; signing and acceptance
receipts match the extracted executable SHA-256:
`44767d7917a65c5bd24cdbf00288d58b9037727f00cedcc8f446678113bdeb10`.
Independent Authenticode verification of that downloaded executable on games
returned `Valid`, publisher Sean McNamara, with Microsoft's timestamp authority.
Downloaded assets, CI receipts and local verification evidence are retained in
`build/scroll-release-0.3.2/`.

## Conversation opening investigation — 2026-10-29

`dev/windows-thread-benchmark.py` measured the published, signed 0.3.2 executable
with SHA-256 `44767d7917a65c5bd24cdbf00288d58b9037727f00cedcc8f446678113bdeb10`.
The same executable ran on games (Windows 11 x64 26100.9711, active console session)
and [hosted Windows CI run 40879417319](https://github.com/allquixotic/fastrock/actions/runs/40879417319).
CI independently verified the release checksum, attestation, executable hash,
Authenticode publisher and timestamp. All twelve samples per host passed.

Each sample starts a fresh native application process, waits for backend readiness,
then opens a conversation at 1440×1000/100% with software rendering. The local ACP
fixture has no provider, extensions, inference, credentials or external services.
New conversations use the normal folder-trust path in an isolated folder outside
Git. This measures conversation opening after application startup, not cold launch.

| Scenario | games request → thread response | games history availability upper bound | CI request → thread response |
|---|---:|---:|---:|
| new conversation | 18.5–23.7 ms | n/a | 8.0–9.0 ms |
| 10 saved turns, 26 KB replay | 5.3–6.5 ms | 104–308 ms | 3.3–3.5 ms |
| 2,000 saved turns, 5.3 MB replay | 225–287 ms | 411–655 ms | 165–166 ms |
| 200 turns with tool output, 15.3 MB replay | 288–516 ms | 662–907 ms | 287–294 ms |

Ranges cover three samples per scenario. Response timings use the existing
`thread-started` / `thread-resumed` milestones. The transcript export is sampled
every 100 ms until its latest-message sentinel is present. That history measure
includes polling and asynchronous export overhead; it is neither a first-pixel
measurement nor input-to-photon latency. Fixture delivery timing includes Python
serialization and pipe backpressure, so it cannot isolate Rust parsing CPU time.

Source inspection identified these optimization candidates:

- `src/goose/sessions.rs`: new conversations wait for `session/new`; resumed ones
  wait for the entire `session/load` reply before exposing history. Every history
  page clones all loaded items before taking at most 100; turn metadata also clones
  item payloads before discarding them. The shared session mutex covers that work.
- `src/goose/model.rs`: replay constructs presentation events, including cloned
  tool output, then clears them because replay only needs stored history.
- `src/transcript/store.rs`: first-page Markdown parsing and row conversion run
  on the UI thread. Row virtualization does not remove this initial preparation.
- `src/app.rs`: `save_prefs` writes synchronously, including the new-thread path.
  The measured new-thread fixture was still below 24 ms, so this alone does not
  establish the cause of the reported severe delay.

The [Goose ACP implementation](https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/acp/server.rs)
starts extension leases during session activation before returning setup responses.
Its [load handler](https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/acp/server/load_session.rs)
also replays stored conversation content. Provider/extension setup is therefore a
candidate shared by new and existing conversations. The installed-backend experiment
below confirms the dependency, but does not reproduce the user's configuration.

Evidence: `build/thread-benchmark-ci/`, `build/thread-benchmark-games-summary.json`,
and `games:C:/Users/SeanMcNamara/fastrock-thread-032/fixture/`. The manual
`thread-performance.yml` workflow permits repeating the same test against a signed
published release without rebuilding the application.

### Installed Goose on games

After explicit user authorization on 2026-10-29, installed the official
[`@aaif/goose-acp@1.57.0` package](https://github.com/aaif-goose/goose/tree/v1.57.0/ui/goose-acp)
through Bun 1.4.2 as the signed-in Windows user:
`bun add --global @aaif/goose-acp@1.57.0`. Both `where goose` and `goose --version`
passed in that user's session. The command shim is
`C:\Users\SeanMcNamara\.bun\bin\goose.exe`; the package's native x64 executable is
`C:\Users\SeanMcNamara\.bun\install\global\node_modules\@aaif\goose-binary-win32-x64\bin\goose.exe`,
SHA-256 `ee39aee30805fda80ed996a4e2e9940d04ec855436315b8deb96950231145242`.
No normal Goose provider configuration or credentials were changed.

`dev/windows-goose-benchmark.py` drives that installed command with a separate
`GOOSE_PATH_ROOT` per sample. A loopback OpenAI-compatible fixture answers model
discovery and produces one fixed response; no real inference or external tools run.
Four variants each have three fresh-process samples and two create/prompt/close/load
cycles. The empty-config variant retains Goose's default platform extensions;
it does **not** mean zero extensions. The Developer variant explicitly enables
the builtin Developer entry. The other two add the same local HTTP MCP server,
with either immediate initialization or an intentional 1,000 ms delay.

| Goose ACP variant | First new session | Second new session | Reload one saved turn |
|---|---:|---:|---:|
| Empty config / default platform extensions | 118–143 ms | 16–22 ms | 10–15 ms |
| Explicit builtin Developer | 129–140 ms | 18–20 ms | 10–33 ms |
| Default + immediate local MCP | 133–170 ms | 22–47 ms | 17–27 ms |
| Default + MCP delayed by 1,000 ms | 1,110–1,117 ms | 1,025–1,037 ms | 1,019–1,046 ms |

These direct ACP timings include no native UI. Reloads occur after `session/close`
in the same Goose process. The script also launches the same signed Fastrock 0.3.2
binary used above, with software rendering at 1440×1000/100%. Every GUI sample
starts a fresh application/backend process and waits for backend readiness before
opening. New conversations use Fastrock's normal folder path, including its default
cross-tab MCP bridge. Resumes load the saved fixture turn; MCP variants retain the
saved external MCP configuration. All eighteen GUI opens passed in the final run.

| Native Fastrock + installed Goose | New-thread response | Saved-thread response | Saved history availability upper bound |
|---|---:|---:|---:|
| Default platform extensions | 69–115 ms | 40–47 ms | 202–300 ms |
| Explicit builtin Developer | 63–72 ms | 43–54 ms | 201–302 ms |
| Saved immediate local MCP | n/a | 54–70 ms | 201–301 ms |
| Saved MCP delayed by 1,000 ms | n/a | 1,064–1,070 ms | 1,105–1,206 ms |

History availability uses the same 100 ms transcript-export sampling described
above, not first-pixel timing. In the delayed MCP direct-client samples, all saved
text arrived after only **3.2–6.8 ms**, while the final `session/load` reply took
**1,019–1,046 ms**. Fastrock's adapter buffers that replay but waits for the reply
before allowing history queries. This confirms an opportunity to show received
history while activation finishes, with generation, failure and send-readiness
guards preserved. It does not establish which extension/provider, if any, causes
the user's reported severe slowdown. Clean defaults and builtin Developer did not
reproduce a multi-second opening delay.

The other concrete optimization candidates remain page-sized cloning instead of
whole-history cloning, avoiding discarded replay presentation events, and moving
initial Markdown preparation off the UI thread. Their CPU contribution has not
yet been isolated. No Fastrock runtime code or build profile changed in this
benchmark step; no new application release was produced.

Benchmark validation checks extension activation results, actual MCP initialize
requests, matching-session text replay, normal app exit and exported sentinel
content. Early fixture runs were discarded after fixing unknown `server/discover`
handling (must return method-not-found for fallback) and allowing Goose's cached
tool catalog on reload. The final full run exited zero with all checks passing.

Final evidence: `games:C:/Users/SeanMcNamara/fastrock-thread-032/goose-final/`,
local `build/goose-final/`, and `build/goose-install.json`. Raw ACP replies,
request timings, HTTP method logs, isolated config, GUI logs and snapshots are
retained. To repeat on Windows with an already installed backend:

```powershell
python dev/windows-goose-benchmark.py C:\path\to\goose.exe --fastrock C:\path\to\fastrock.exe --output C:\path\to\new-results-directory
```

The benchmark never installs or updates Goose. The existing hosted CI benchmark
uses an ACP fixture; these installed-Goose measurements ran on games.

## Shared vwork transport and whole-database compaction (2026-10-29)

Fastrock 0.3.3 replaces eligible installed vwork stdio definitions at Goose
session activation with same-name Streamable HTTP routes. Both listeners bind
127.0.0.1; Host/Origin checks, bearer authentication on the daemon hop, and
proxy/redirect rejection remain enforced. vwork 0.3.3 runs its existing MCP
handler with request-local workspace binding; Fastrock carries no vwork tool
implementation or redb handle. Restricted sessions do not receive vwork.
Session sidecars preserve bindings across Fastrock restarts. Older daemons and
customized extension definitions retain stdio.

The native Virtual window now selects a database separately from workspace
selection. Compact targets the entire redb, including all repositories and
workspaces, works with an empty catalog, and preserves editor drafts.

On `games` Windows x64, installed Goose 1.57.0, signed vwork 0.3.3, software
rendering at 1440×1000, and an isolated local model fixture:

| Three samples each | Signed Fastrock 0.3.2 | Development Fastrock 0.3.3 |
| --- | --- | --- |
| New conversation response | 109.6–115.5 ms, median 113.6 | 94.0–118.1 ms, median 95.1 |
| Resume seeded conversation | 87.4–95.2 ms, median 92.0 | 63.4–79.0 ms, median 65.1 |
| Resume after Fastrock restart | Not included | 75.3–89.3 ms |
| vwork clients per GUI open/resume | 1 | 0 |

The owner stayed alive through each run. The process monitor observed four
clients for the standalone-Goose control in every sample, and zero clients
across all nine new Fastrock GUI operations. Actual `vwork_workspace_info` calls
returned the original workspace before and after app restart. Origin and forged
Host probes returned HTTP 403. Timings measure the existing native response
milestones, not end-to-end perceived pixel latency; three samples do not
establish a universal percentage improvement. This machine did not reproduce
the user's severe work-machine latency. The eliminated process launches are
verified; their benefit on a machine with expensive process creation remains
unmeasured. Live inference and unrelated user MCPs were not exercised.

Reproducible command: `dev/windows-goose-benchmark.py <goose.exe> --variants vwork
--repeats 3 --fastrock <fastrock.exe> --vwork <vwork.exe> --require-shared-vwork`.
The script creates isolated configuration, repos and databases, cleans up its
own daemon, and uses no real provider credentials or Rally writes. Omit the
last flag for the 0.3.2 baseline. Machine-readable measurements and executable
hashes: [benchmark receipt](benchmarks/vwork-startup-games-2026-10-29.json).

Windows tests: 699 application + 7 workspace tests passed; one opt-in live
workspace test skipped. Three release-policy tests passed, Mac-only symbol
test skipped. Mac compatibility check passed without executing tests or GUI.
The daemon dependency passed Windows/macOS CI 40895378344 and signed release
40896550504; ZIP attestation, checksum and Windows publisher/timestamp were
independently verified. Application optimization remains 0, the existing
render/text allowlist remains 2, and LTO remains off.

A separate cold-owner check stopped only the fixture daemon, then opened a new
conversation through Fastrock. It started the installed owner successfully,
completed in 245.3 ms, served the real bound MCP tool, and retained the
Host/Origin guards. The test stopped its owner afterward. The native database
window smoke also passed compaction before workspace selection, draft retention,
conflict handling, revision-checked save and disabled-provider hiding.

Signed application release [40899046673](https://github.com/allquixotic/fastrock/actions/runs/40899046673)
passed both platform builds, 706 Windows Rust tests, release-policy checks,
Authenticode/timestamp verification, and native Codex/Rally, Goose conversation,
Goose/Rally, vwork and software-scroll acceptance on the exact signed executable.
Mac publication was skipped and the temporary build runner deregistered.
Source: `98075c028691bba085f0814cf4e1ed675d1d078c`.
ZIP SHA-256: `1d110a2483f0ee157000061e47769f067d94da9db53f45602ebbbc4a69c0fce9`.
Signed EXE SHA-256: `c3e5cac2218aef354e457de610de2bacc0c1783153120f35710e87cca300e1bd`.
Downloaded checksum and source/tag/workflow-bound attestation passed; games
independently verified Sean McNamara's Authenticode signature and Microsoft
RFC 3161 timestamp. README and release notes were refreshed after publication;
release assets remain the unchanged CI-built bytes.

The downloaded signed executable passed the installed-stack check on games:
new 97.5 ms, seeded reload 69.8 ms, and reload after app restart 83.5 ms. All
three operations used HTTP, preserved the original workspace and rejected
Origin/forged-Host probes, with zero vwork clients. Standalone Goose control
launched four clients as expected. The receipt includes this separate final run.
## Conversation selection, 2026-10-29

Fastrock 0.3.4 adds 12-logical-pixel selection gutters beside rich/native text
(table cells use half their inter-column spacing). Tool results use selectable
native styled text. Expanded calls retain full arguments/commands and Copy All
retains details plus output. Native input focus preserves a new glyph anchor
when switching from a gutter selection. No vwork source or protocol changes.

- Both new V37 Rust regressions failed on the 0.3.3 implementation, then passed.
  All **708 Windows tests** passed on `games` (one existing live-vwork integration
  test remains ignored). `FASTROCK_TEST_PYTHON` points to the installed Python.
- `dev/windows-selection-smoke.py` passed on `games` at 100% and 200% scaling:
  17 exact clipboard cases, three native Copy menus, margin/glyph transitions,
  second-line code, UTF-8/graphemes, link drag versus click, real header expansion,
  and selection across a virtual tool-row boundary. Copy All retains the full
  arguments and all 24 fixture result lines. Screenshots inspected.
- Mac compatibility compile, `cargo fmt --all -- --check`, build-policy audit
  and `git diff --check` passed. No Mac GUI or tests ran. Additional Clippy audit
  fails on 36 existing denied diagnostics in unchanged Rally/Goose/lib code;
  each diagnostic's source line was checked against the committed baseline.
  No diagnostics point to this change's transcript files. This is not a clean
  Clippy claim; release CI's build, tests, policy and signed fixture gates remain.
- Software-renderer comparison on `games`, 1440×1000, scale 1, 120 requested
  frames per scenario, same fixture and signed 0.3.3 baseline. Frame p95 in ms:

  | Scenario | 0.3.3 | 0.3.4 development build |
  |---|---:|---:|
  | Prose | 7.375 | 7.150 |
  | Table | 6.114 | 7.316 |
  | Code | 6.050 | 6.261 |
  | Tools | 6.030 | 6.833 |
  | Streaming | 8.744 | 8.163 |

  These are single-run measurements, not a speedup claim. All measured p95
  frame times remain below 8.2 ms with selection enabled. Restore smoke passed.
  Development executable SHA-256:
  `5843b144881475953420345ff01de9f351043758f7b6ce6854e70018d5bfc1b8`.

[Signed release CI 40917749947](https://github.com/allquixotic/fastrock/actions/runs/40917749947)
passed from source `19e94e62276d14d4246abadbc710db90923f4eba`: both platform
compiles, 708 Windows tests, release policy, Azure signing/timestamp verification,
and signed-byte Rally, Goose, vwork, selection and scrolling fixtures. The
temporary build runner was removed. [Fastrock 0.3.4](https://github.com/allquixotic/fastrock/releases/tag/v0.3.4)
is published as a Windows x64 ZIP; the Mac publishing job was intentionally skipped.

Downloaded ZIP SHA-256:
`89fc853834acfa8d631ab940a246dadd55135e68bdd7adda943a8990887f426a`.
GitHub digest, checksum file and embedded binary receipt agree; bundled provenance
verified against the exact source, tag and workflow. Independent verification on
`games` confirmed Valid Authenticode, Sean McNamara publisher and Microsoft RFC3161
timestamp. Published executable SHA-256:
`dd0b29630e62ade319575c48c7417013a356e79d3aff2ca350b5cae62526b7a9`.
vwork remains unchanged at `c53cea4879bc0e12735ae1c89bd7d5ca3e9b1910`.

## 0.4.0 development: durable queue recovery

On 2026-10-29, `games` passed 708 application tests, 16 workspace tests and
the standalone embedded-Goose fixture. The native renderer test also passed.
Mac compatibility compilation, formatting and the optimization-policy audit
passed; no application or tests ran on the Mac.

`dev/windows-summary-smoke.py --recovery` restarted embedded Goose during a held
local Bedrock request. It verified no work before Resume, one retained uncertain
send, native review/Copy, a single visible replayed prompt, and exactly one queued
send after Resume. The queued turn generated one Luna search summary; search and
settings caused no additional model calls. Explicit Quit retained the unresolved
record paused for the next launch. Unit fixtures also cover legacy-file migration,
invalid records, attachments, edits, close/reopen, reviewed-ID dismissal and failed
completion writes. Closed chats release their cached recovery state.

Final native receipt:
`C:\Users\SeanMcNamara\fastrock-040-policy\recovery-native-4\acceptance.json`.
Development executable SHA-256:
`745b206fe98adca24c235ab464547d0d05e2302aac1e6c38d0cd90332c905f4b`.
Peak process memory was 149,364,736 bytes under a 2 GB fixture cap.

This is queue/backend-restart acceptance, not complete process/draft recovery or
the large-workload benchmark. No live model credentials or production Rally
endpoint were used. No 0.4.0 release was published.

## 0.4.0 development: account quota removal

Account quota panels, notification handling, snapshots, formatting and polling
were removed on 2026-10-29. Conversation context information remains. All 702
remaining application tests passed on `games` (six obsolete quota tests were
removed), along with 16 workspace tests, the embedded-Goose fixture and native
renderer test. Mac compatibility compilation and formatting passed.

The combined native fixture passed three main-window turns, a second-window
turn, summary replacement/search, independent Rally navigation and draft handoff,
close-to-tray background completion, restore and native-menu Quit. Both windows
belonged to one process. The receipt is
`C:\Users\SeanMcNamara\fastrock-040-policy\no-quota-native-1\acceptance.json`;
peak process memory was 183,476,224 bytes under the 2 GB fixture cap. Executable
SHA-256: `e4155727cbcd460d63bcdcb50af174f7b5192bea5b1eee70661ecd305b5eb0f9`.
All eight provider requests and all Rally reads used local fixtures.

## 0.4.0 development: Rally model release

On 2026-10-29, switching Rally documents clears obsolete Slint editor models,
and closing the displayed document releases its rows, nested cards, editors and
assistant buffers. Open documents retain their own drafts. The native fixture
verified empty detached editor models, zero retained models after the final
close, reopening, and the existing multiwindow draft-transfer/tray behavior.

Games passed 702 application tests, 16 workspace fixtures, the embedded-Goose
fixture and the native renderer regression. Mac compatibility compilation and
static audits passed; no Mac execution. The combined native fixture made eight
local provider calls and only local Rally reads, with 184,406,016 peak process
bytes under its 2 GB cap. Receipt:
`C:\Users\SeanMcNamara\fastrock-040-policy\rally-release-native-1\acceptance.json`.
Executable SHA-256:
`2570f2dcae2f4a4a87132b9ec9b912572fa77e86d17fe6f75a1fee4c2319de30`.
Full 0.4.0 feature/performance acceptance remains open; no release tag.

## 0.4.0 development: process and draft recovery

On 2026-10-29, Fastrock began saving open persistent chats, unsent composer text,
attachment paths, model overrides and local recovery records through one atomic,
coalescing background writer. Quit flushes the latest snapshots. Relaunch reloads
history but requires Resume before sending; stale local submissions are retained
for review and never automatically replayed. Unknown/malformed recovery files are
preserved with a visible warning and saving disabled. Draft attachments are no
longer removed solely because of age. Backend-paused queues also retain the draft
when Send is pressed before Resume.

Games passed 706 application tests, 16 workspace fixtures, the embedded-Goose
fixture and the native renderer regression. New tests cover Unicode/image paths,
model/Plan settings, interrupted image preparation, uncertain local input,
multiple window snapshots, closed tabs, invalid files and failed-write recovery.
Mac compilation, formatting and build-policy checks passed; no Mac execution.

`process-recovery-native-5` forcibly killed only the fixture process after its
draft had been persisted. Relaunch restored the Unicode draft and image,
review/Copy exposed a simulated stale local submission, no provider request ran
before Resume, and the draft was sent exactly once afterward. The uncertain
submission was absent from history. Both completed turns had Luna summaries;
the fixture now waits for the background worker rather than a fixed delay.
Four local provider requests; 145,489,920 peak process bytes under a 2 GB cap.

`restore-regression-native-1` passed three main turns, a second-window turn,
summary/search, Rally document release and draft handoff, background completion
while hidden, restore and native tray Quit. Both windows belonged to PID 3576;
eight local provider requests and only local Rally reads. Peak process memory:
183,062,528 bytes. These receipts live under
`C:\Users\SeanMcNamara\fastrock-040-policy\<fixture>\acceptance.json`.
Both used executable SHA-256:
`c66a72c9660ababbe0adb2a9b7451446e2ed97c632c0c8d6e017c2c8c0c5830c`.
The backend-restart draft-refusal fixture `recovery-native-5` also passed, on
SHA-256 `4f2b8e84cbc4135876d7bdc7a3d19829400df9f7d9d9187eee45dd1d8c259739`,
before the automation-only summary wait was added.

Persistent chats from all windows currently reopen together in the main window.
Original window layout and temporary side-session recovery remain under T43.
Full-scale performance comparison and the remaining 0.4.0 features remain open;
no release tag or live provider/Rally calls.

## 0.4.0 development: embedded release fixtures

On 2026-10-29, the Windows release fixtures stopped emulating an external ACP
backend. The actual embedded Goose crate now receives loopback Bedrock Converse
and AWS event-stream responses, executes configured fixture MCP tools, and calls
Fastrock's Rally/vwork adapters. Isolated Goose/AWS configuration uses fake keys;
no live provider or Rally service is contacted. Every native smoke has a 2 GB
process cap. Two Windows watchdog tests verified that timeout and memory-limit
failures terminate only the owned process, leaving an unrelated process alive.

Games verified executable SHA-256
`c66a72c9660ababbe0adb2a9b7451446e2ed97c632c0c8d6e017c2c8c0c5830c`:

- Concurrent streamed conversations and isolated cancellation: 142,073,856 peak
  process bytes (`goose-chat-smoke-2`).
- Full Rally list/board/rank/undo/detail/create/relations/rich text/discussion/
  saved views/bulk/proposal/Apply/restart/window transfer: 82 loopback requests,
  13 explicit fixture writes, two model calls, 155,328,512 peak process bytes.
  CI mode exercises the move callback; native picker, Apply and window controls
  remain real UI interactions.
- Real embedded vwork: register/import committed source, create workspace, edit,
  compact, conflicting model tool edit, reject stale save, reload and save with
  the new revision. UTF-8 BOM/CRLF and executable mode preserved; source checkout
  untouched; no installed Goose/vwork or daemon connection file. Default embedded
  database also available without legacy extension configuration. Peak process
  memory: 155,504,640 bytes.
- Selection: all 17 clipboard cases, three native menus, Unicode, links, tool
  arguments/output and selection across virtual rows passed; 156,835,840 bytes.
- Five 120-frame scroll scenarios passed, including tool output and active
  streaming. Frame p95: prose 6.973 ms, table 5.029 ms, code 4.406 ms, tools
  5.597 ms, streaming 7.172 ms. Peak process memory: 158,982,144 bytes. These are
  small smoke workloads, not the required 1,000-chat/10,000-message comparison.
- Three-turn summary/search plus two-window Rally navigation passed: eight local
  model requests, both native windows in PID 8336, 171,483,136 peak process bytes.

Evidence is under
`C:\Users\SeanMcNamara\fastrock-040-policy\ci-fixtures\build\` in each fixture's
`acceptance.json` (scrolling uses `summary.json`). Backend queue recovery also passed at 148,451,328 peak process bytes; forced
process/draft recovery passed at 146,235,392 bytes. Both required explicit Resume
and retained uncertain sends for review without replay. CI now includes these
recovery cases and summary/search on the signed bytes.
No Rust/application change in this milestone; prior 706 application tests,
16 workspace fixtures and Mac compatibility compile remain applicable. Full
performance comparison, remaining 0.4.0 features and live-profile acceptance
remain open; no release tag.
