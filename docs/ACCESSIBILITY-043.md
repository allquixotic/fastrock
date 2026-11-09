# AccessKit focus crash — 0.4.3

The reported defect is present in the 0.4.2 source. The bundled Slint 1.18.1
backend is unchanged between the 0.4.2 tag and the pre-fix main branch.
[Slint #10269](https://github.com/slint-ui/slint/issues/10269) remains open and
its maintainer describes the same reentrant-registration failure.

During a partial accessibility update, visibility or focus evaluation can
instantiate a repeater. Its registration callback tries to borrow the AccessKit
adapter already held by the update and silently drops the notification. Later,
focus resolution allocates an ID for the new control and publishes it without
the corresponding node. AccessKit rejects that inconsistent tree with
`Focused ID … is not in the node list`. Fastrock's `panic = "abort"` build
makes this fatal; `catch_unwind` cannot recover it.

The fix retains/coalesces a deferred rebuild when registration or destruction
reenters the borrowed adapter. Rebuilds prune expired identities and invalidate
text caches after deferred destruction. All focus paths check the published
node set (or the current full rebuild), temporarily using the window root until
the new control is available. Accessibility remains enabled.

The report's crash dump was not supplied, so its original stack decoding and
specific work-machine trigger were not independently examined. Source analysis
and tests establish the defect independently; they do not establish which UI
Automation client triggered the original incident.

## Verification

- Windows consumer regression with the original focus policy: one expected-panic
  test passed and three safety cases failed. Two failures reproduced the exact
  AccessKit 0.38.0 `tree.rs:75` panic. Cases cover focus-only updates, partial
  updates/delegated focus, replacement/removal and focus after a full rebuild.
- Pre-fix 0.4.2 fixture UI Automation exercise: 40 settings-page transitions, 322
  tree queries and 322 focus requests across eight distinct controls passed. This is a timing-sensitive defect;
  that run did not reproduce the race. The fixture attaches exclusively to the
  disposable process it starts, with isolated configuration and local providers.
- Fixed consumer regression: all four tests passed on Windows.
- Fixed native fixture: all four sidebar/history/selection/accessibility checks
  passed. Active accessibility completed 40 page transitions and 327 queries/
  focus requests across eight controls, peaking at 98,910,208 process bytes.
- Full Windows suite: 861 Rust tests plus standalone embedded Goose and release
  policy/watchdog checks passed. A transient SSH interruption stopped the first
  attempt; no fixture processes remained and the complete guarded retry passed.
- Both 0.3.4 and the candidate passed 1,000 conversations, eight concurrent
  streams and 10,000 history messages at full scale. Candidate peak process
  memory was 252,391,424 bytes. Host/guest headroom and the 32 GB cap were
  monitored; no memory guard triggered. Frame p95 was 9.388 ms versus 8.681 ms
  on 0.3.4; this fix makes no general performance improvement claim.
- Windows and Mac compilation, formatting and workflow lint passed. No Mac
  application or tests were run. [Local receipt](benchmarks/043-games-2026-11-09.json).
- [Signed release CI](https://github.com/allquixotic/fastrock/actions/runs/42700164850)
  passed 861 Rust tests, all 41 native cases, migration, catalog and scrolling
  checks. The signed accessibility fixture completed 40 page changes and 334
  focus requests across eight controls.
- [0.4.3](https://github.com/allquixotic/fastrock/releases/tag/v0.4.3) was published
  as the latest stable Windows x64 ZIP. Downloaded checksum, package metadata,
  online and bundled attestations, Windows publisher and timestamp were verified.
  No Mac assets were published; no Mac application/tests ran. The temporary
  runner removed its registration. [Release receipt](benchmarks/043-release-ci-2026-11-09.json).
