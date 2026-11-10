# Repository fetch performance

Requested 2026-11-10 after `vwork repo fetch` took several minutes on the work
machine. This is an unreleased source update after Fastrock 0.5.1. No work-machine
trace is available, so the exact cause of that delay remains unconfirmed.

## What fetch does

1. Wait for the repository lock and a bounded transfer slot. A build on the
   same repository or exhausted transfer capacity can contribute to elapsed
   time. Earlier versions held one global transfer mutex for the entire fetch.
2. Create an owned temporary bare Git receiver and expose existing Fjall objects
   through a read-only libgit2 object backend. Seed saved refs for negotiation.
3. Use libgit2 to authenticate, negotiate and fetch **all branches and tags**
   from the saved source (an explicit source overrides and is saved on success).
   It does not invoke Git CLI transport or create a working checkout.
4. Enumerate received packs, import missing objects into Fjall, compute bounded
   Git storage deltas, and publish updated refs. Existing virtual workspaces
   remain pinned; private commits are retained. Pruning is explicit.
5. Remove the owned receiver and retain a cleanup record if removal fails.

This can include server pack generation, network/proxy/authentication latency,
local pack indexing, database work, filesystem overhead and lock waits. It does
not normally re-download or re-import every existing object, but a transfer may
contain duplicates. In particular, libgit2 1.9.7's local transport recursively
packs advertised annotated tags when an update requires a pack, including
already stored tree contents. Fetching again does not make an existing
workspace advance to the new source revision.

## Changes

Fastrock pins vwork `649ace65c89eafd8b401be7d34355071293f0d73`.
The initial optimization at `9b5c268` added:

- Seed negotiation using one validated, sorted `packed-refs` file instead of
  creating a loose file, lock and rename for every ref on every fetch.
- Check immutable object metadata before decoding an incoming duplicate.
  Add repository membership when sharing content from another registration.
  New objects still undergo hashing, bounded delta encoding and normal storage.
- Fetch imports only objects; it no longer repeats the generic registration
  importer’s ref/config scan whose result was discarded.
- Return `timings_ms` for repository/transfer waits, source resolution,
  preparation, transport, import, publication, cleanup and total time. Return
  visited/reused/decoded object counts and decoded bytes. Transport includes
  authentication, negotiation, receiving and libgit2 indexing/ref updates.

The follow-up replaces the global transfer mutex with owned active-transfer
guards. Independent repositories can fetch or clone concurrently, up to four
transfers or the configured temporary-copy limit if lower. Same-repository
operations retain their lock. Excess requests wait for capacity; abandoned
cleanup records still count against the disk-copy limit. Cleanup skips active
receivers through final removal, and the cleanup intent is synced before
creating external files. Allocation/sync time is counted in `prepare`, separately
from `transfer_wait`.
If abandoned receivers fill the available slots, admission attempts owned,
inactive cleanup on the worker before reporting exhaustion. A restart therefore
does not require a separate recovery action for ordinary abandoned transfers;
failed cleanup and mismatched ownership still preserve the files.

Concurrent imports may receive identical Git objects. Imports now collect
bounded OID batches and retry only conflicting database work with fresh delta
preparation. They retain the 256-object/64-MiB batching bounds (one larger object
remains subject to the existing object limit) and never replay network calls.
No extra retained copy of decoded batch bodies is needed for retries.

Fastrock already admits four bulk operations and keeps permits with the actual
worker across cancellation. The old engine mutex prevented those independent
transfers from overlapping after admission. Goose's combined tool-batch
presentation remains separate from actual operation completion.

The tool's successful JSON result now identifies the slow phase. Fastrock's
existing Settings → Diagnostics → Workspace repository transfer additionally
measures store/admission/worker waits outside the engine. A visible Goose tool
spinner can also include waiting for other calls in its tool batch.

For now, avoid unnecessary repeated fetches and fetch the intended registration
once before creating workspaces from its updated ref. Use an explicit local
source only when that checkout already contains the upstream refs you need;
fetching a stale local source cannot retrieve newer server commits. There is no
single-branch fetch option in the current tool. Narrowing ref scope, changing
transport or diagnosing corporate security scanning would require separate
evidence; this change preserves the complete existing fetch semantics.

## Verification

`examples/fetch_performance.rs` in `fastrock-workspaces` compares a synthetic
400-file repository with 2,001 branches and an annotated tag. It measures
unchanged fetch, a one-file upstream update and repeated fetch. Each binary gets
an independent identically seeded source: resetting a ref alone leaves dangling
new objects that registration would import and invalidate the update test.

Compile the fixture against baseline `fb523be0efd118adc819fd6cc4159f97d67dd5f0`
and the pinned candidate. Run `dev/windows-fetch-benchmark.py` on games with
`--baseline`, `--candidate`, `--output` and the current `--host-guard` heartbeat.
The runner contains each process tree, checks host/guest headroom, limits memory
to at most 32 GB and retries at one tenth the workload on a guard stop. It
checks that the candidate decodes three update objects, reuses duplicates,
preserves the old workspace and reads the new file from a new workspace.

The initial `9b5c268` full-size comparison passed on games:

| Operation | 0.5.1 vwork baseline | Candidate |
| --- | ---: | ---: |
| Unchanged fetch | 2,332.4 ms | 147.9 ms |
| One-file update | 5,431.6 ms | 969.6 ms |
| Repeat fetch | 4,006.2 ms | 228.0 ms |

The update received 406 objects in both versions. The candidate reused 403 and
decoded only three (17,077 bytes). Peak process-tree commit was 55,312,384 bytes
baseline and 51,900,416 bytes candidate. Registration measured 267.3/260.4 ms;
these changes do not eliminate first-import or network transfer work. Earlier
valid comparisons showed the same improvement direction with different absolute
times; these are single-host synthetic samples, not work-machine predictions.
Receipt: [guarded Windows fetch comparison](benchmarks/fetch-games-2026-11-10.json).

49 native Windows tests passed: 17 upstream library tests, four Git performance
regressions, three fetch regressions, five object-store integration regressions
and 20 Fastrock workspace unit tests. The duplicate-payload regression fails on
the original importer and passes on the candidate. Upstream strict Mac lint and
Fastrock package/all-target compilation passed; no tests or app ran on Sean's
Mac. Isolated Fastrock test-target compilation requires the application's usual
Tokio `rt-multi-thread` feature. A separate strict Fastrock lint attempt remains
blocked by existing `useless_format` in `memories/mod.rs` and two `collapsible_if`
findings in `session.rs`; these files are unchanged.

[Upstream CI 42893961204](https://github.com/allquixotic/vwork/actions/runs/42893961204)
passed Windows and hosted macOS formatting, strict lint, full tests and repeated
lifecycle race regressions at the initial optimization revision. The initial CI run caught
read-only fixture permission setup on each platform; the corrections affect
only tests, with no production-source changes after the first passing Windows
benchmark. The original 0.5.1 release and its signed assets are unchanged.

## Concurrent network-transfer verification

All fixtures use generated repositories and a loopback smart-HTTP server. No
external repository, service account, credentials or proprietary data is used.
The server holds its reference advertisements until the test releases them,
making actual request overlap observable without inferring it from a spinner.
The old mutex allows only one of four requests to reach the server; the candidate
allows all four. Periodic cleanup runs while responses are held and preserves
every active receiver. Additional cases check a lower capacity, the hard
four-transfer bound, same-repository serialization, failed cleanup ownership,
abandoned guards and overlapping shared-object imports.

A separate fixture contains 25 branches, 12 annotated tags and a 16-MiB binary.
After import, it makes the stored binary unreadable and removes access to the
source. Two unchanged HTTP fetches must still succeed. The server rejects any
pack request and observes exactly two reference advertisements, zero pack
requests and zero downloaded objects/bytes. Both the 0.5.1 engine and candidate
pass: large stored binaries alone do not explain an unchanged fetch delay.
This fixture deliberately does not model HTTPS certificates, proxies,
credential helpers, real-server latency or network stalls. Windows libgit2 uses
WinHTTP; native Git uses a different HTTP implementation, so native Git timing
does not establish that their connection/authentication paths behave identically.

The sanitized receipt records this follow-up's native checks and upstream CI:
[concurrent fetch verification](benchmarks/fetch-concurrency-games-2026-11-10.json).
The concurrency change passed 56 native Windows checks. After the final
abandoned-receiver recovery adjustment, all three HTTP tests, the recovery
guard test and 20 Fastrock workspace tests passed again against the final pin.
Peak process-tree commit across these guarded runs was 148,058,112 bytes.
Mac compilation and upstream strict lint passed without running tests on
Sean's Mac. [Final upstream CI 42906549113](https://github.com/allquixotic/vwork/actions/runs/42906549113)
passed strict lint, the full suite and repeated lifecycle races on Windows and
hosted macOS at `649ace6`. This remains an unreleased source update after 0.5.1.
