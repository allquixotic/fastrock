# Fastrock 0.4.2 performance work

Requested 2026-11-09 after workspace tools took approximately eight to nine
minutes on the work PC. Windows x64 publication through signed GitHub Actions
is authorized. T75 runtime acceptance is complete; signed CI publication and
independent asset verification are the final distribution gates.

## Confirmed code paths

The affected vwork revision is `7bad2919054452a2017de64200e7f98ff2c3b453`.
`git::log` selects libgit2 topological sorting before applying the result limit.
libgit2 drains reachable history during that sort, including for a three-entry
preview. `git::prepare_build` exports every repository object through the loose
filesystem ODB, creating a file for each historical object.

Goose 1.57.0 `ToolExecutionOperation::run` receives results individually but
emits the combined response after all streams finish. Four visible spinners
therefore do not prove that four engine operations ran for eight minutes.
Goose is unchanged; per-call engine measurements distinguish this presentation
delay from actual operation latency. No work-PC timing capture is available.

## Implementation

- Use an explicit commit-date priority queue for limited logs. Expand parents
  only while more results are needed and deduplicate shared ancestors. Preserve
  both merge parents. Topological ordering is no longer promised by the preview.
  Merely changing libgit2's revwalk sorting to TIME was insufficient: that mode
  also enables whole-history preprocessing. CI run `42677218736` caught this on
  both platforms through the missing-ancestor regression; commit `77b688a` fixes
  the traversal itself.
- Write full temporary Git history through libgit2 streamed packs. Bound batches
  to 4,096 objects or 64 MiB of decoded content, except for a single larger
  object subject to the existing storage limit. Limit packing to one worker and
  a 16 MiB delta-search window/cache. Keep the complete Git history, tags,
  overlays and independent checkout ownership. Reusing a lease skips objects
  already present. New leases still read repository history; this change removes
  loose-file amplification, not the cost of producing a complete Git database.
- Admit at most four bulk calls before acquiring the eight general worker slots.
  Waiting bulk jobs cannot occupy the other four slots. Started workers retain
  both permits after caller cancellation. Repository locks and revision guards
  are unchanged.
- Expose bounded categories for store wait, admission wait, blocking-worker wait
  and execution in Diagnostics. Separately time new-session workspace recovery
  persistence. Do not record paths, arguments, contents or credentials.
- Authorize 0.4.2 in the existing release policy and skip Mac publication.
  Preserve feature-completion gates, Windows signing, timestamp verification,
  attestations and signed-executable acceptance.

## Verification and remaining release work

The modified Fastrock library and workspace test targets compile on macOS.
Workspace test targets also pass a Windows x64 cross-check using workspace-local
copies of the installed SDK cache and unchanged blake3 source for assembler
scratch files.
The vwork candidate and all its test targets compile and pass strict Clippy.
Formatting, Python syntax compilation and build-policy validation pass. Strict
workspace Clippy reports only the two previously recorded `collapsible_if`
findings in `session.rs` (B284); they are outside this change. No
application or test executable ran on Sean's Mac.

The vwork source is committed and pushed to upstream `main` at
`77b688a80e06fd05a630058dce043adbf445d5a4`, with Fastrock pinned to that exact
revision. The working checkout is `/tmp/fastrock-vwork-042`. No local-path
dependency patch is used by the current builds. CI run `42678235015` passed
Windows and hosted macOS formatting, strict lint, full tests and repeated
lifecycle races. Receipt: `docs/benchmarks/042-vwork-ci-2026-11-09.json`.

`crates/fastrock-workspaces/examples/workspace_performance.rs` supplies the same
synthetic workload for both versions. Compile it against this checkout and
the detached `b3cd4ff` baseline at `/tmp/fastrock-041-performance`.
`dev/windows-workspace-benchmark.py` runs both on games with the existing
hypervisor heartbeat and guest/process memory limits. It starts with 2,000
files and 4,000 commits, including merges and an annotated historical tag;
guard aborts retry at one tenth the scale. Per-operation durations, process-wide
I/O counters, peak memory, object-file counts, store sizes and `git fsck` results
are preserved. I/O counters cover whole processes because concurrent calls
overlap. Fixtures contain no private source or provider calls.

The full-size games comparison passed: temporary checkout 28,867.7 → 6,258.0 ms,
three-entry log 23.8 → 0.6 ms, Git object files 14,040 → 12, and process-wide
write bytes 206,474,690 → 8,400,309. Candidate peak process memory was
33,054,720 bytes; both histories passed `git fsck`. Initial repository import
remains substantial (13.7 seconds baseline, 15.2 seconds candidate); these fixes
target subsequent commands and do not eliminate first-import work. This is one
synthetic comparison, not a work-PC latency measurement. See
`docs/benchmarks/042-workspace-games-2026-11-09.json`.

Fastrock Windows validation passed 857 Rust tests, the standalone embedded
Goose and clean-store fixtures, release-policy checks and watchdog checks.
Receipt: `docs/benchmarks/042-tests-games-2026-11-09.json`. The Mac application
compiled successfully; no application or tests executed on Sean's Mac.

Nine native cases passed: vwork, projects, checkpoints, compaction, relaunch,
history, stalled history startup, queue recovery and forced process recovery.
The final local executable SHA-256 is
`64f2d7d33cdf6d82bc4133d6fb53ae48a7d172acf535d5baf0cf6b51aa331272`.
Receipt: `docs/benchmarks/042-native-games-2026-11-09.json`.

The native virtual-project New chat path passed against stable 0.4.1 build 4:
baseline cold/warm/warm 14,758.9/802.3/803.5 ms; candidate
13,864.3/802.3/603.5 ms. These include explicit fixture waits, project opening
and 100 ms idle observation; the cold case imports the repository. Ordinary
folder observations are preserved separately. The initial virtual fixture
failed Git ownership validation because SSH and the desktop use different
accounts; only the generated repository root/.git ownership was corrected.
Receipt: `docs/benchmarks/042-sessions-games-2026-11-09.json`.

A fresh 0.3.4 baseline and candidate both passed 1,000 conversations, eight
simultaneous streams and 10,000 history messages, with no reduced-scale retry.
Candidate peak process memory was 248,135,680 bytes (baseline 183,173,120),
history-open observation 208.1 ms (baseline 1,405.8), and scroll frame p95
7.989 ms (baseline 8.123). Host/guest reserves and the 32 GB stop limit remained
active. Receipt: `docs/benchmarks/042-scale-games-2026-11-09.json`.

Acceptance checklist (steps 1–4 complete; step 5 is publication):

1. Execute vwork's full suite on Windows and hosted macOS, including `v39_*`:
   a short preview succeeds when a distant commit is unavailable, merge parents
   remain visible, all exported Git object bytes match, `git fsck` passes,
   historical objects occupy pack files, and lease reuse preserves existing
   packs while adding only missing objects.
2. Execute Fastrock workspace/cancellation tests, release-policy tests and native
   project, history, temporary checkout, recovery and durability fixtures on
   `games`. Read its AGENTS.md and check free space first.
3. Measure baseline versus candidate on a local synthetic monorepo. Record
   actual per-operation times, process read/write counters, object-file counts,
   peak memory and store size. Exercise two creates, a three-entry log and a
   temporary checkout concurrently, plus cold/warm new conversations. Include
   deep history, many files and merge commits; use no private repository or live
   provider. Retain evidence even when a run aborts.
4. Run the required 1,000-conversation/eight-stream/10,000-message games workload
   against the retained 0.3.x baseline with the host/guest headroom monitor and
   32 GB stop limit; use the required tenfold fallback if it trips.
5. Commit and push Fastrock to main, complete T75 using measured evidence,
   then tag `v0.4.2`. The existing Actions
   pipeline must build, sign, verify and publish the Windows x64 ZIP. Verify
   release assets and CI acceptance before reporting publication.

## Environment blockers

The resumed Codex session has working GitHub DNS/API/push access and writable
Git metadata. Initial default SSH failed because the Mac aliases selected only
RSA while both servers accept the existing Ed25519 key. This repeated the
November 4 client-configuration omission, previously masked by agent state.
Following the user's investigation/fix request, the Mac `games` and
`aorus-ubuntu` entries explicitly select that existing Ed25519 identity. Both
passed with the agent disabled. Server keys/policy and VM lifecycle were
unchanged. Games verification is unblocked; no additional release approval is
needed. The installed tailnet skill records the permanent correction.

### Original chat environment

The local shell cannot resolve `github.com` or `games.tail5e018.ts.net`.
Earlier direct SSH to games also returned `Operation not permitted`; do not
route around the restriction. The Windows cross-check first failed because
the global cargo-xwin cache was read-only. A workspace-local cache resolved that
issue. The blake3 assembler also required writable temporary `.asm` files next
to its source; a workspace-local copy of that unchanged crate allowed the
Windows workspace cross-check to pass. These SDK/crate overrides are validation
inputs only, not release dependency changes. Git index writes in the original
Fastrock checkout are also denied, so source commits are preserved in isolated
checkouts and bundles without altering its protected Git metadata. Neither
runtime acceptance nor publication is claimed.
