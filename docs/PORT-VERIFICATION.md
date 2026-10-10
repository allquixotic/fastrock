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
