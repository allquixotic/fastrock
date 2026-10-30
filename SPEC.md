# SPEC

## §G GOAL
Fastrock 0.4.0: stable T3 UI/orchestration in native Slint/Rust, unmodified embedded Goose, embedded compressed vwork and full Rally support.
Broader GUI contract: repository `GUI.md`.

## §C CONSTRAINTS
- Fastrock derives from pinned allquixotic/codex-gui; Apache-2.0 and Slint attribution preserved.
- 2026-10-29 authorization supersedes external-backend rules: Goose/vwork/core orchestration in-process; configured MCPs/hooks/credential helpers and user/LLM tools may spawn. No Goose fork, Codex backend or remote agent servers.
- Preserve Goose configuration/OS credentials. Bedrock/direct OpenAI/direct Anthropic, tools/hooks/recipes/code mode; no local inference or voice. Required model routes block final release; unsupported effort levels do not.
- Application/dependencies opt=2, debug=0, LTO="off" including local ThinLTO, 256 codegen units, incremental, no stripping/packing; build scripts opt=0. Explicit 2026-10-29 authorization.
- Native Slint controls/layouts. Repository projects; permanent Rally virtual project, closable horizontal documents/boards, no conversation binding. On-demand temporary Rally assistant retains human Apply.
- All applicable stable T3 features including scheduling; exclude every quota feature. Native diff/review/files/terminal/palette/checkpoints; browser web previews.
- Exclusive embedded vwork ownership; no standalone-client compatibility requirement. Preserve existing stores, compressed source, tool leases, hooks, revisions and explicit exports.
- Close-to-tray; explicit Quit stops work. Restart restores drafts/queues but requires Resume; no uncertain tool replay. Isolated delegated workspaces; explicit integration.
- Named independent provider/endpoint/AWS profile/region/credential/model/effort profiles where unchanged Goose permits. Display supported reasoning levels; accept custom saved levels only through supported Goose APIs.
- Codex histories untouched on disk, absent from UI. Goose history retained; one current search summary backfilled per existing conversation.
- One latest compaction-length keyword-focused summary per conversation; each completed user-facing turn replaces it asynchronously. Compressed summary shares vwork redb; no historical summaries; original conversation retained. Settings identify cheap profile, else Bedrock GPT-6 Luna; unavailable means deterministic search only.
- Search performs immediate case-insensitive title/current-summary matching, then bounded multithreaded fuzzy search over current summaries. No model call per query; cancel obsolete searches.
- Track Mac/games free space; clean identified stale build junk only when needed.
- Native Slint 1.18.1; bound transcript memory; no idle repaint polling.
- 0.4.0: signed Windows x64 ZIP via CI, Mac compatibility compile only. Usable signed prereleases authorized; final requires full acceptance. Imported distribution records historical.
- Commit/push main directly; no draft PR workflow. CI builds/signs/verifies/publishes releases with persistent incremental caches; no unsigned fallback.
- Never run GUI tests or launch this project's GUI on Sean's Mac unless the user explicitly says "test the GUI on this Mac"; use Windows hosts.
- Publish only after development verification passes. Explicit 0.2.0 exception: official profile optimizes at level 1 with full debug info, packed symbols, LTO off; fast defaults remain.
- Stable 0.3.0 is authorized with the fast profile and Windows x64 ZIP publication only; compile the Mac build but do not publish it.
- Verify GitHub asset digest before deleting local build outputs.
- Games benchmark: 1,000 stored conversations, eight active streams, 10,000-message conversation; compare 0.3.x memory/scroll/switch latency. Stop fixture process at 32,000,000,000 bytes, reduce workload tenfold; low host/guest headroom also stops tests. Never risk host OOM.

## §I INTERFACES
- I.scroll: wheel, scrollbar, Jump to latest in conversation pane.
- I.paint: Windows restore, remote desktop exposure, existing redraw events.
- I.questions: synchronous server-request forms; asynchronous message forms and ordinary send/steer/queue replies.
- I.speed: provider-catalog speed picker; thread/settings/update and turn/start.
- I.summaries: sidebar titles/tooltips, resize handle; GUI-only persistent cache, ephemeral fast-model requests.
- I.distribution: Azure Authenticode, Developer ID Application, Apple notarization/stapling, GitHub artifact attestations.
- I.links: glyph hover, Copy link, Open in browser, Open file.
- I.goose: embedded stable Goose public Rust APIs; session lifecycle/streaming/permissions/profiles/extensions, no HTTP/stdio hop to external Goose.
- I.vwork: embedded vwork Engine and public tool API, exclusive existing redb owner; workspace browser/editor/compaction and compressed current search summaries share database.
- I.orchestration: persistent threads/runs/queues/lineage/mailboxes/schedules; delegate/message/wait/interrupt/fork/model switch/explicit merge/recovery; no quota features.
- I.search: immediate deterministic substring plus bounded parallel fuzzy search; background cheap-model current-summary replacement and backfill.
- I.lifecycle: close-to-tray, explicit Quit, restart Resume and safe cancellation.

## §R REFERENCES
R1: T3 Code v0.1.5, 6d450a4284852af137b2f3407fddd89eb76e7644; UI/orchestration-v2 reference, MIT attribution; https://github.com/pingdotgg/t3code/releases/tag/v0.1.5.
R2: Goose v1.57.0, 30bba95105e59bc3e7df624cf6b4d433690cfe12; unmodified public Agent/extension APIs; https://github.com/aaif-goose/goose/releases/tag/v1.57.0.
R3: Ghostty v1.4.0, 83fb95624b0439ee660c2679b43b38c8b63769cd; idle incremental LZ4 page compression/on-demand restoration; Windows requires portable adaptation; https://github.com/ghostty-org/ghostty/blob/v1.4.0/src/terminal/compress/Page.zig.
R5: vwork 0.3.4, c53cea4879bc0e12735ae1c89bd7d5ca3e9b1910; public Engine/protocol, redb 4.4, zstd level/config retained; https://github.com/allquixotic/vwork.
R6: Goose 1.57.0 hooks accept command actions only; Agent hook manager is private. Authorized hook exception permits Fastrock helper mode using vwork protocol::hook, no separate vwork install.
R7: Goose Bedrock construction uses process-global config/environment; independent concurrent AWS credential/region profiles need a supported upstream API. ACP supports upstream effort values, not arbitrary custom strings. User authorizes fixture-only development and will validate live profiles on own machine.
R4: AWS Runtime supports Converse/InvokeModel and OpenAI-compatible APIs; Runtime/Mantle routes differ; exact model/profile must preserve installed working configuration; https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints.html.

## §V INVARIANTS
V1–V14 retain the imported upstream behavior/history. V10 describes its historical signed distributions; current Fastrock prereleases follow V15/V22.
V1: Upward user scroll ! detach tail-following before virtual row measurement; small idle scroll ! persist; automatic height/offset adjustment ! preserve live tail following.
V2: Lost Windows surface pixels ! repaint next frame without resize; idle ! schedule no polling frames.
V3: Link hit ! match shaped glyph and clipping; wrapped web/file links ! retain exact destination.
V4: Copy/tooltip ! preserve file line and resolve relative path; browser action ! encode file URL.

V5: Speed choices and actual request tiers ! follow provider catalog and feature requirements; Standard ! send explicit default; model switch ! reset unsupported tier; resume/fork ! preserve reported tier.

V6: Async questions ! show choices + free text; no send before Submit; exact message/index reply identity; turn end ! retain unanswered fields; rejected send ! preserve drafts; history replay ! deduplicate; transcript ! render question once.

V7: Purpose inference ! consume only completed user requests, never assistant/tool context; one turn ! return title + plain-text tooltip; completed request ! also satisfy initial cache-missing scheduling; ephemeral ! no listed thread/rollout; fast model failure ! retry normal model.

V8: Cache ! survive restart under resolved CODEX_HOME; any manual rename ! permanently protect title; tooltip ! remain generated; resize ! debounce ten seconds, visible rows only, cached tooltips only; title ! fit actual row without ellipses; stale result ! never replace newer context/width/name.

V9: Conversation selection ! match rendered rich/plain glyphs and UTF-8 graphemes; pointer drag + Shift/arrows ! select; Ctrl/Cmd+C and menu Copy ! selected plain text; links and markdown ! preserve; tab change ! clear; selected scene ! remain bounded.

V10: Mac native payload ! contain arm64 + x86_64 slices, verified by actual Apple tools before signing; dynamic dependencies ! Apple system libraries only. Release ! sign every native payload with expected identity + secure timestamp; Mac ! harden runtime, restrict V8 entitlements to code-mode helper; execute IPC on both slices, staple accepted app before DMG construction, accept both Apple logs, validate DMG + mounted app. Attestation ! bind final package bytes to exact successful source/workflow; temporary Mac runner ! manual main only, no GUI, remove after job.

V11: Composer Enter ! queue after entire active turn; Shift+Enter ! steer as soon as possible; Alt+Enter ! newline; explicit Queue and Steer coexist while busy; per-message intent ! survive startup and image preparation.

V12: Every sidebar tooltip ! remain fully inside conversation-pane bounds at all window sizes and pointer positions; wrapped text ! remain readable without blocking sidebar rows. Generated titles ! use natural spaced words, representative font-width budget and measured final fit without ellipses. Tooltip purpose ! directly describe work, never narrate the requester.

V13: Viewing/resuming/reading ! never advance thread activity; user/agent/tool changes ! advance it. Read state ! persist independently of activity; blue filled ! unread or running, empty ! visited/read idle, absent ! unvisited idle, warning ! needs input, red ! failure.

V14: Pending-message pencil ! edit or delete local unsent input and server queue entries; preserve non-text attachments; queue update/delete ! atomic backend boundary; consumed input ! never silently rewrite history; failed/racing edits ! retain draft.

V15: Default dev/release/test application ! opt-level=0, debug=0, LTO="off", codegen-units=256; explicit rendering/text dependency allowlist ! opt=2 only; build scripts/CI ! no environment overrides. Verify: dev/check-build-policy.py.
V16: Startup ! launch installed external Codex after first frame; inherited env/config; process cancellation/restart/disconnect ! retain unsent drafts. Verify: transport::tests plus Windows mock-process smoke.
V17: Rally tokens ! OS vault only, no logs/prompts/settings; refs ! same-origin WSAPI; HTTP ! loopback only; mutation ! never auto-retry. Verify: rally::client::tests.
V18: Query/search/filter/preset/types/export ! retain selected scope, paging and loaded-only totals; metadata ! per-workspace/type. Verify: rally::view::tests and fixture integration.
V19: Writes ! review/revision/scope guards; dirty/conflict drafts retained; batch failure ! exact completed count; no claimed atomic transaction. Verify: rally::editor::tests, rally::assistant::tests.
V20: Every inventory capability R01–R22 ! Rust implementation plus named fixture/Windows evidence before marked complete; native Slint UI only. Verify: docs/RALLY-INVENTORY.md ledger and dev/windows-rally-smoke.py.
V21: Personal settings/views/session ! migrate legacy names and independent snapshots; vault service ! fastrock. Verify: rally::store::tests.
V22: Windows prerelease ! fast profile, exact source/asset metadata; tests ! fixture-only, software renderer; Mac GUI ! never launched. Verify: workflow/policy audit and Windows smoke.
V23: Release ! commit/push main, tag, CI only; exact source/run/unsigned input hashes verified before signing; Windows ! expected publisher + RFC3161 timestamp + fixture acceptance of signed bytes; Mac ! pinned Developer ID + Accepted app/DMG logs + stapling + Gatekeeper including mounted app. Cache ! persistent, no per-release archive wait; no PR. Verify: release.yml, signing receipts and GitHub attestations.
V24: Rally AttributeDefinition AllowedValues ! preserve all enum choices with shared literal "null", JSON null or absent refs; complete first unpaged catalog ! normalize StartIndex=0. Persisted duplicate identities and partial/unexpected pages ! still fail closed. Verify: rally::client::tests::v24_* and realistic Windows metadata fixture; Broadcom KB 57584.

V25: Explicit stable 0.2.0 ! official profile opt=1, debug=2, packed symbols, LTO="off" (including local ThinLTO), strip=none, 256 codegen units, incremental; build scripts ! fast. Stable tag ! GitHub official/latest release, matching PDB GUID/age and dSYM UUID, symbol archives with hashes/source/run/attestations. Prereleases ! retain V15/V22. Verify: dev/test-release-policy.py, CI receipts, native signed Windows fixture and symbol checks.

V26: Default ! one installed Goose ACP process; concurrent sessions ! independent prompt/cancel/permission routing; prompt ! no short RPC deadline; shutdown/disconnect ! stop owned tasks/process and retain drafts. No Goose dependencies. Verify: goose::tests::concurrent_sessions_and_cancel, goose::tests::disconnect_fails_pending, build policy.
V27: ACP history/config/permissions ! server capabilities and values; session IDs ! backend namespace; missing feature ! explicit error, never fabricated success; unknown notifications ! tolerated. Rally tools ! scoped loopback MCP bridge, no credentials, human Apply unchanged. Verify: goose::tests and dev/windows-goose-smoke.py.
V28: Virtual controls ! absent without configured vwork; multiple stores/repos/workspaces, bounded lists and UTF-8 editor. IO ! background tasks. Saved bytes/mode ! retained, revision conflict/stale callbacks/config removal ! preserve drafts. Extension rename ! stable store identity. Daemon credentials ! never GUI/log/persistence. Verify: virtual_workspaces::browser_tests and fastrock-workspaces HTTP/config fixtures.
V29: Stable 0.3.0 ! GitHub non-prerelease/latest, retained fast profile, signed/verified/attested Windows ZIP. Provider and app tests ! run on Windows; no Mac GUI or release binary. Verify: release_config, CI receipts and public downloads.
V30: Vwork compaction UI ! selected store only, native menu/button, background authenticated daemon call; progress/outcome ! accurate; drafts/provider-removal guards preserved; no direct DB access or daemon restart. Verify: virtual_workspaces::browser_tests::v30_compaction_preserves_drafts, provider HTTP fixtures, Windows vwork smoke.

V31: Windows software frames ! retain private framebuffer; unchanged scene ! no rerasterization; every existing redraw ! fully restore presentation pixels despite false buffer age; resize/scale/recreation ! correct; idle ! no polling. Verify: window_runtime::tests and games scroll/recovery fixture.
V32: Large transcript tables/tool output/diffs ! bounded rows in native ListView; shared column widths, source order, copy, links, expand/collapse ! preserved. Verify: transcript virtualization tests and games fixtures.
V33: 0.3.2 ! games before/after frame timing with same fixture, Windows tests, Mac compile, signed/verified CI Windows ZIP. Rendering optimization ! exact dependency receipt, LTO off.
V34: Installed-Goose timing ! isolated config/local provider, successful extension activation and observed MCP initialization; unknown fixture RPC ! method-not-found; replay timing ! matching session text events. Reload tool catalog may be cached. Verify: dev/windows-goose-benchmark.py on games.

V35: Shared vwork MCP ! 127.0.0.1 listeners only; authenticated daemon, exact Host, no Origin/proxy/redirect; preserve per-request workspace, installed tools, timeout, restricted-session boundaries and restart recovery; warm conversation open ! zero vwork client spawns. Custom transports retain stdio. Verify: v35 fixtures + installed stack on games.
V36: Compact ! entire selected redb, independent of workspace/catalog selection; multiple stores ! explicit selector; drafts and stale-response guards preserved. Verify: v36 browser fixture + Windows native smoke.
V37: Conversation selection ! accept drags beginning in adjacent horizontal padding, preserve shaped offsets, links, keyboard and Copy; expanded tool/command details ! selectable full arguments/command and visible results, including no-output calls; Copy All ! retain complete details across virtual chunks. Verify: v37 render regressions and dev/windows-selection-smoke.py on games/CI.

0.4.0 overrides conflicting historical transport/storage/build/summary/distribution clauses in V7/V8/V15/V16/V22/V26–V30/V33–V35. Other correctness and attribution requirements remain.
V38: Core ! one process, unmodified pinned Goose/public APIs plus vwork; no external Goose/Codex/vwork dependency or loopback core-service hop. Configured tool/extension/hook/credential subprocesses allowed. Verify: embedded backend/provider/extension fixtures and process-tree smoke.
V39: Required Bedrock Runtime Sol/Luna and US cross-region Bedrock Opus routes ! preserve configured IDs/endpoints/auth; effort values ! Goose-supported/custom-public-API only; profiles ! isolated concurrently; no secret logging. Verify: route/request-body/stream/tool/cancel/resume fixtures, controlled working-profile smoke.
V40: Vwork ! single embedded Engine, preserved source compression/revision/lease/compaction behavior. Summary ! same redb, atomic replace by monotonically newer completed turn; no summary history or secret material. Verify: embedded store reopen/conflict/summary-replacement/stale-generation tests.
V41: Orchestration ! durable queues/lineage/messages/schedules, isolated child workspaces, explicit merge; restart ! awaiting Resume, uncertain tools ! never automatically replay. No quota feature. Verify: orchestration lifecycle/crash recovery/delegation/schedule fixtures.
V42: T3 visual/interaction parity ! native Slint; project ! repository with workspace-bound chats; Rally project ! permanent, independent closable docs/boards, no conversation context. Last closed Rally document ! release retained row/card/editor/assistant models. Rally assistant ! temporary/on-demand/human Apply. Web previews ! system browser. Verify: native games UI fixtures plus Rally inventory.
V43: Huge history ! lossless compressed cold pages, bounded resident/render caches, asynchronous prefetch/restore, stable anchors/selection/copy/export; no whole-history layout or UI-thread IO. Verify: compressed-history roundtrip/random-jump/selection/stream-boundary fixtures.
V44: Search ! deterministic local immediate case-insensitive title/summary matches, bounded parallel fuzzy continuation and cancellation. Every user-facing completed turn ! cheap-model compaction-length keyword summary replaces old summary off UI thread; no recursive summarizing of summary jobs. Backfill ! one latest per old Goose conversation. Cheap profile visible; no expensive fallback. Verify: search ordering/cancellation/budget and summary scheduling/replacement/backfill fixtures.
V45: Games workload ! 1,000 threads/eight streams/10,000 messages; benchmark watchdog stops fixture at 32 GB process memory or inadequate host/guest headroom, records abort, retries at tenfold lower scale. Kernel memory cap ! include owned child processes; unavailable/stale host monitoring ! fail closed. Never terminate unrelated app/VM or exhaust host. Verify: watchdog fixtures and measured games comparison.
V46: 0.4.0 application/dependencies ! opt=2, debug=0, LTO="off", 256 codegen units, incremental, no strip; Windows signed CI ZIP only, Mac compile only. Prerelease ! usable verified milestone; stable ! all acceptance and required routes. Verify: build/release policy tests, signed receipts and Windows acceptance.
V47: Last window close ! tray continuation, explicit Quit ! cancel/save/exit, restart ! restore chats/drafts/settings without executing pending work. Uncertain local sends ! review-only; malformed session snapshot ! preserved without overwrite; draft attachment files ! retained. Verify: native close/tray/Quit and recovery fixtures.

V48: Native windows ! one embedded backend/store owner; callbacks, timers, async results and server requests route to owning window. Rally moves preserve drafts/recovery without starting a process. Verify: scoped routing tests and native two-window transfer/interaction fixture.

V49: Explicit workspace integration ! review source/target changes, preserve source and checkout, guard both reviewed heads under exclusive owner lock; stale review/conflicts ! no merge writes. Incomplete review ! Apply disabled; successful dispatch consumes review. Verify: v49 integration/browser tests and native vwork merge fixture.

V50: Bound conversation ! persisted workspace default and separate scratch cwd; native open ! clean draft/current response. Fork ! unchanged Goose public storage, isolated vwork revision, exact message boundary, preserved parent/history/tools, no inherited schedule or automatic run. Partial fork ! hidden until bindings saved; restart ! original bindings. Verify: v50 workspace/browser/fork-storage tests and native fork/restart fixture.

V51: Project ! registered repository; workspace chats group by repository while retaining actual scratch cwd. Native repository open ! in-process import of committed HEAD, reuse registration, preserve dirty/untracked source. Empty registered projects ! remain visible outside search. Verify: v51 repository/sidebar tests and native registration/grouping fixture.

## §T TASKS
id|status|task|cites
T1|x|Fix small-scroll tail feedback|V1,I.scroll
T2|x|Recover Windows retained surface; Auto uses CPU in RDP|V2,I.paint
T3|x|Shared link menus and hover tooltips|V3,V4,I.links
T4|x|Verify, commit, push, publish Windows package, clean outputs|V1,V2,V3,V4; GUI.md §16

T5|x|Add catalog-driven inference speed and verify upstream migration|V5,I.speed

T6|x|Extract standalone GUI, pin latest stable, verify Windows CI, commit/push/release|GUI.md §19

T7|x|Fix asynchronous question forms, reply delivery, history replay and Windows interaction verification|V6,I.questions

T8|x|Persistent background purpose summaries, manual-title protection and resizable sidebar; verify and ship next release|V7,V8,I.summaries

T9|x|Selectable user/assistant conversation text, keyboard/clipboard/context menu; verify and ship with T8|V9,I.links

T10|x|Build, sign, notarize/staple and attest 0.3.1 Windows/Mac packages; verify exact bytes, publish and remove temporary runner|V10,I.distribution; GUI.md §25

T11|x|Integrate stable 0.162.0; replace Astra patches with minimal upstream Sol catalog backport|V5
T12|x|Explicit queue/steer input and key routing, including delayed input|V11,I.questions
T13|x|Bound sidebar tooltips, readable summaries and persistent activity/read state|V7,V8,V12,V13,I.summaries
T14|x|Verify Windows software/GPU interactions; sign, attest and publish 0.4.0 for Windows/Mac|V10,I.distribution

T15|x|Edit/delete pending messages from transcript, preserving delivery intent and attachment data; verify Windows races|V14,I.questions

T16|x|Inventory Go Rally and import pinned codex-gui source without losing history|V20
T17|x|Enforce fastest build profiles/scripts and prerelease pipeline|V15,V22
T18|x|Replace embedded runtime with installed Codex stdio transport|V16
T19|x|Port WSAPI/security/vault/preferences/query/schema services|V17,V18,V21
T20|x|Native Rally pages/boards/tables/filter/saved-view UI|V18,V20,V21
T21|x|Native details/rich fields/relations/attachments/conflicts/bulk editing|V19,V20
T22|x|Port scoped assistant tools/proposals/human Apply|V17,V19,V20
T23|x|Complete fixture/headless/Windows interaction checks and prerelease|V15,V16,V17,V18,V19,V20,V21,V22
T24|x|Land port directly on main; fast cached CI signs/verifies/publishes Windows and Mac prerelease|V15,V22,V23,I.distribution
T25|x|Fix enum metadata mistaken for duplicate Rally records; verify and publish signed prerelease|V17,V18,V23,V24
T26|x|Publish official optimized/debuggable 0.2.0 with LTO off and matching debug symbol archives|V15,V22,V23,V25,I.distribution
T27|x|External Goose ACP default, concurrent sessions, native conversation/config/permission UI, Rally MCP bridge, fixture verification; retain Codex option|V6,V11,V14,V16,V17,V19,V20,V26,V27,I.goose
T28|x|Verify Windows/Mac builds, Windows tests and signed CI prerelease; document Goose architecture/capability limits|V15,V22,V23,V26,V27,I.distribution,I.goose
T29|x|Port vwork provider/browser onto current Goose/Rally architecture; verify and publish stable 0.3.0 Windows ZIP, mirror with ship-to-aws|V15,V23,V28,V29,I.vwork,I.distribution
T30|x|Native vwork storage compaction control, preserve drafts, verify Fastrock 0.3.1 Windows fast CI release and explicit S3 upload; Mac compatibility compile|V15,V23,V28,V30,I.vwork,I.distribution

T31|x|Retained Windows presentation, optimized render/text dependencies, finer transcript rows, games benchmark and 0.3.2 CI release|V15,V31,V32,V33,I.scroll,I.paint,I.distribution
T32|x|Measure signed 0.3.2 opening on games/CI; installed Goose 1.57.0 local-provider/extension timing on games; confirm activation gates history, record unprofiled copy/UI candidates and real-config limits|V27,V33,V34,I.distribution,I.goose
T33|x|Shared localhost vwork MCP, database-level compaction, actual Goose/games comparison, signed 0.3.3 CI release|V15,V27,V34,V35,V36,I.vwork,I.goose,I.distribution
T34|x|Widen text selection hit areas, selectable tool/command details, Windows interaction/scroll checks, Mac compile and signed Fastrock 0.3.4; no vwork changes|V9,V15,V23,V32,V37,I.distribution
T35|x|Record accepted 0.4.0 contract, optimized profiles and Windows-only release policy; policy audit passed, games ReleasePolicy 3 passed/1 Apple-only skipped|V46,I.distribution
T36|~|Embedded session/prompt/close and three Runtime routes verified on games fixtures; live profiles require user validation|V38,V39,I.goose
T37|~|Replace external backend/core bridges with embedded Goose; remove Codex UI/connection, preserve Goose configuration|V38,V39,I.goose
T38|~|Embed vwork Engine/tools/browser/compaction; bound conversations and reviewed explicit integration; compressed current-summary table in same redb|V40,V49,V50,I.vwork
T39|~|Port applicable T3 V2 orchestration, delegation/workspaces/scheduling/durable recovery; isolated conversation forks; no quotas|V41,V50,I.orchestration
T40|~|Native T3 main UI/sidebar/panels, registered repository projects and permanent independent Rally project|V42,V51,I.questions,I.links
T41|x|Background per-turn summary replacement/backfill and immediate bounded deterministic search|V40,V44,I.search,I.summaries
T42|~|Lossless compressed transcript cache, incremental rendering and efficient arbitrary scrolling|V1,V9,V32,V37,V43,I.scroll
T43|~|Tray lifecycle, explicit Quit and restart Resume|V41,V47,I.lifecycle
T44|~|Watchdog-protected games benchmarks, full Windows verification, Mac compile and signed CI milestones/final|V23,V38–V47,I.distribution
T45|x|In-process Rally windows and scoped UI/backend routing|V38,V42,V47,V48

Port acceptance: v0.2.0-alpha.3, source 97a2ef394b1591ba9e0421a2cf376055899ece30; 676 Mac headless tests; games native mouse/picker, 82 fixture requests/13 writes, late drafts/restart/window transfer; both OS binaries unoptimized; no Mac GUI or real Rally writes. ZIP SHA-256 a6f56551249d377e292db78735622e4af59d0e9c2947f79478cba299ab5d0d75. Hosted run 38036468741: 667 Windows tests + callback UI/package/cache passed on alpha.2 source. Detailed evidence: docs/PORT-VERIFICATION.md.

Signed release acceptance: v0.2.0-alpha.4, source 32ab8aeb93dcd7401c32fc3392a19178daf01286, CI 38048069773 all jobs passed. Build 5m26s; 676 headless tests; signed Windows callback + games physical mouse each 82 requests/13 fixture writes; expected Authenticode publisher/timestamp; Mac pinned Developer ID, both Apple logs Accepted, app/DMG stapling and mounted Gatekeeper passed. Published checksums + attestations independently verified. Direct main push; temporary runners removed; no Mac GUI or real Rally writes. Full receipt: docs/PORT-VERIFICATION.md.


Embedded foundation verification, 2026-10-29: games Rust application 689 passed plus one isolated native rendering test; workspace 5 embedded and 7 historical HTTP/config fixture tests passed; legacy real-daemon test skipped. Standalone unchanged Goose fixture passed initialization, session creation, prompt, assistant text, close and Runtime Converse routes for all three required model-name fixtures. No live provider/Rally calls. Mac `cargo check --locked --lib` passed, no Mac tests/GUI. Build policy and formatting/diff audits passed. T36–T38 remain open for live profile acceptance, graphical configuration, recipe/session migration and full workspace integration. No release tag.

Search verification, 2026-10-29: games application 693 tests passed; workspace 2 bounded-search, 5 embedded-store and 7 historical fixture tests passed. Standalone Goose/Bedrock fixture verified Luna-only summary backfill/replacement, original-message preservation and restart deduplication. Native Windows smoke `dev/windows-summary-smoke.py` found a case-insensitive keyword present only in compressed summary, rejected absent keyword, saved visible cheap profile; exactly one Sol conversation request plus one Luna summary request, no query-time model call. Peak process memory 155,545,600 bytes; smoke stopped above 2 GB, not full V45 workload. Binary SHA-256 `48568ac6d90cbd8c721c91d777041e019bdab89110b4ab48a9c89c8588a1121b`; receipt on games `C:\Users\SeanMcNamara\fastrock-040-policy\summary-native-1\acceptance.json`. Mac library check, Windows app build, formatting and build-policy checks passed; no Mac execution or live model/Rally calls. T42/T44 performance acceptance remains open.

Scrollback foundation verification, 2026-10-29: cold LZ4 pages, 16-page/8-MiB decoded-cache budget, direct item/turn indexing and append-stable cursors. Games application 694 tests passed; workspace 4 search/scrollback plus 12 embedded/historical fixtures passed. Roundtrip/random-page fixture covers 10,950 items, UTF-8/tool arguments, live append, empty turns and oversized cache bypass. Native three-turn export retained every question/answer exactly once, no search-summary leakage; six local provider requests, 147,087,360 peak process bytes. Binary SHA-256 `95b1a7c1590a679c765813909d04d5eccb302fae597460927621b4113dce5e34`; games receipt `C:\Users\SeanMcNamara\fastrock-040-policy\scrollback-native-1\acceptance.json`. Mac compile passed. T42 remains open for full large-history GUI acceptance and T44 comparison; this smoke is not the full V45 workload.

Navigation foundation verification, 2026-10-29: native T3 sidebar/header adaptation, separate Rally document strip and permanent Rally project; same-area bulk close preserves dirty Rally documents. Games 694 application tests, one rendering regression, 16 workspace fixtures and standalone embedded Goose fixture passed. Native three-turn smoke also verified independent Rally selection/return/close/reopen, draft retention and navigation through an injected backend failure overlay; only GET requests to local Rally fixture, six local model calls, 162,045,952 peak process bytes. Binary SHA-256 `156cf30c6935f41c21463455f10bf433ae8bf4176196c2ac00ba37d67be47d91`; games receipt `C:\Users\SeanMcNamara\fastrock-040-policy\sidebar-native-2\acceptance.json`. Mac compile and static audits passed. T40 remains open for repository/workspace binding, composer/panel parity and complete orchestration UI; no release tag.

Core startup audit, 2026-10-29: removed automatic login-shell PATH probing and renderer subprocess relaunch. Mac compile passed; Windows library compiled and 693 application tests passed (obsolete relaunch test removed), workspace and embedded Goose fixtures passed. Static audit finds no process launch in src/lib.rs or src/platform.rs. Rally window process migration remains open under T37/T40/T43.

Native-window foundation verification, 2026-10-29: one shared embedded backend, explicit UI ownership for native callbacks/timers/async and blocking tasks, single-target tool/permission routing, per-window input queues, one ordered mailbox writer, and durable same-process Rally handoff. Games 699 application tests, one renderer test, 16 workspace fixtures and standalone Goose fixture passed. Native three-turn plus second-window conversation smoke preserved separate exports, found both current summaries, moved unsaved Rally name/filter drafts out and back, and observed both owner records under PID 1580. Eight local provider calls, local Rally GETs only; peak process memory 176,660,480 bytes. Binary SHA-256 `1c0dab9c60888b7c785a481716871dd9b1360d5994ce5628a65c558efe3602c5`; games receipt `C:\Users\SeanMcNamara\fastrock-040-policy\windows-native-2\acceptance.json`. Mac compile/static audits passed; no Mac execution. Last-window tray continuation and restart recovery remain T43/T45 acceptance work; no release tag.

Tray lifecycle verification, 2026-10-29: native Slint tray, last-window close preserves active work, restore retains conversation state, native menu Quit flushes asynchronous preferences and stops the shared runtime. Games 700 application tests, one renderer test, 16 workspace fixtures and standalone embedded Goose fixture passed. Combined three-turn/two-window/Rally navigation and draft-transfer fixture passed with eight local model calls, local Rally GETs only and 179,818,496 peak process bytes. Binary SHA-256 `9d00955e020a3d93ce9a75f6a23d94cc77e831d1a5a17e8c2d38fcc493b5ebfb`; games receipt `C:\Users\SeanMcNamara\fastrock-040-policy\tray-native-8\acceptance.json`. Both document owners share PID 6468; native window count includes the tray popup. Mac compile, build policy, formatting and diff checks passed. T43 remains open for restart Resume/draft recovery; T44 full-scale comparison and release remain open. No Mac execution or release tag.

## §B BUGS
id|date|cause|fix
B1|2026-10-06|32px tail threshold and height-estimate callbacks pull upward scroll back to bottom|V1
B2|2026-10-06|Retained surface pixels assumed valid across Windows remote-display exposure; missing full invalidation|V2
B3|2026-10-06|Slint overwrites link hit with each wrapped-line rectangle instead of accumulating hits|V3; pinned core patch
B4|2026-10-06|Treating automatic content-y corrections as user gestures disables following during streamed row updates|V1; detach on input, never on offset alone
B5|2026-10-06|TouchArea moved fires only during dragging; stopping on hover movement without restarting loses tooltips|V3; handle pointer move and reset the stationary delay

B6|2026-10-07|Upstream protocol migration adds required fields and replaces skill PathBuf with LegacyAppPathString; clean Git merge still fails compilation|Adapt typed paths and protocol constructors; one-time migration, no new invariant

B7|2026-10-07|Development-main protocol fields and path wrappers differ from stable 0.161.0|Adapt release constructors and optional wire capabilities; one-time migration

B8|2026-10-07|Async question metadata rendered as duplicate passive markdown; no input form or ordinary reply route|V6; retained form and upstream question reply envelope

B9|2026-10-07|Extraction omitted workspace Clippy test settings; copied deny lints reject existing test assertions|Own minimal Clippy config retains test assertions and lock-guard checks; one-time migration

B10|2026-10-07|Windows Cargo checkout contains a 266-character upstream snapshot path even when TUI is not linked|Short Cargo cache path plus Git core.longpaths in Windows CI; environment constraint, no new invariant

B11|2026-10-07|Stable Bedrock catalog normalization clears all speed tiers; GUI-only fixtures miss loss of native Astra Ultrafast|V5; minimal provider catalog backport and native Mantle/Runtime/custom Sol regression

B12|2026-10-07|Nine inherited Windows test failures use Unix-only absolute paths, file URLs or displayed separators|Use upstream native test paths and platform-correct URL/display expectations; existing path invariants suffice

B13|2026-10-07|Stable core request builder independently discards all Bedrock tiers even after catalog normalization is fixed|V5; minimal core expression backport and actual HTTP-body regression for Mantle/Runtime, native/future models and unsupported tiers

B14|2026-10-07|Global/posted keys fail to activate guest native menus; GW_OWNER does not identify their owning window|V9; GetGUIThreadInfo ownership + IAccessible default action; standalone native-popup regression

B15|2026-10-07|Pristine stable workspace Cargo.lock requires resolution changes when building helpers; --locked stops packaging after GUI build|Consumer helper manifest compiles unchanged stable entrypoints with the GUI root lockfile and exact Git dependencies, retaining setup asInvoker metadata; stable pin/package invariant covers recurrence

B16|2026-10-07|Mock model catalog omitted the canonical instruction template required by stable Codex; real GUI server never became ready|Add minimal model_messages.instructions_template, parse fixture with upstream schema before Windows smoke, and retain startup logs; existing stable API migration policy covers recurrence

B17|2026-10-07|Rich shaping gave every paragraph range 0..0, clamping cursor geometry and suppressing paint; native user-text right clicks were grabbed before the parent menu; synthetic Ctrl chords used uppercase text|V9; assign rich plain-text byte ranges, explicitly open native text menus, normalize unshifted primary chords, and assert real cursor progression/round-trip plus drag/clipboard/menu behavior

B18|2026-10-07|Completed-turn summary scheduling did not mark the initial missing-cache attempt, allowing sidebar refresh to invalidate and duplicate in-flight inference|V7; share queue bookkeeping and verify exactly two purpose calls for two processed requests on software and GPU renderers

Release evidence: v0.3.0, commit e3494e5943458e61b5209ba7761bdbd02f4922e1; hosted workflow 37697823119 passed all 629 Windows tests and interaction/package gates. The exact ZIP passed software/GPU and question-form smoke tests on games, helper dispatch/startup/manifest checks, and GitHub digest verification. Mac: 638 headless tests and clean Clippy; no GUI launch. See GUI.md §24.

B19|2026-10-07|Installed Apple lipo rejects combined two-architecture -verify_arch invocation even with input-first order|V10; verify slices independently and add real clang/lipo universal-positive plus single-slice-negative headless packaging gate

B20|2026-10-07|allow-jit alone works on arm64 but signed x86_64 V8 traps during code-range setup; unsigned control passes and stable upstream already includes allow-unsigned-executable-memory|V10; reuse exact upstream helper entitlements, keep GUI unentitled, force both architectures through signed IPC/JIT regression before notarization

B21|2026-10-07|Native lzma-sys pkg-config discovery links MacPorts liblzma and adds its search path, also selecting MacPorts libiconv in the ARM GUI|V10; use supported LZMA_API_STATIC build switch, check dependencies before packaging, and test rejection with a real temporary dylib

Release evidence: v0.3.1, source 084b9f8271a61abee6e22630df91f3c8265062dc; Windows run 37721517380 passed 629 tests, interaction gates, Azure signing and attestation. The exact signed ZIP passed signatures/timestamps, helper checks, question forms and software/GPU purpose/selection tests on games. Mac run 37721517143 passed 638 headless tests, dual-architecture signed helper IPC/JIT, both Accepted notarizations, staples and Gatekeeper including the mounted app. Both online/bundled attestations and all eight GitHub upload digests verified. Temporary Mac runner deregistered and removed; no Mac GUI launched. See GUI.md §25.

B22|2026-10-08|Sidebar tooltips use native cursor-relative popups that can leave the screen and cover neighboring rows|V12; single bounded conversation-pane overlay
B23|2026-10-08|Widest-glyph character budget severely underfills normal prose; summary prompt permits compressed labels and requester narration|V12; representative width, measured final fit and explicit natural-language prompts
B24|2026-10-08|Sidebar uses generic update time and open-tab phase as activity/read status; resume writes look like conversation progress|V13; meaningful turn timestamps and separate persistent read state

B25|2026-10-08|Stable 0.162 adds turn lineage, subagent model telemetry and string skill paths|Adapt typed constructors and skill paths; stable API migration, no new invariant

B26|2026-10-08|New pencil tooltip passed plain string to styled-text Slint property|Use explicit markdown conversion; compile-time regression

B27|2026-10-08|Glob protocol import shadows std Result and fixture uses obsolete image field|Explicit imports and current image/detail shape

B28|2026-10-08|New automation measurement borrows an existing item reference; activity read condition nests a collapsible branch|Remove redundant borrow and collapse condition; mechanical lint cleanup, no new invariant

B29|2026-10-08|Preparation validates restored generated-crate provenance before replacing an older upstream cache|Validate source pins first, regenerate, then enforce provenance; verified with deliberately stale cache

B30|2026-10-08|Pending-message smoke used the legacy automation send default, which steers; its first edit scenario did not exercise Queue|Drive the actual composer Queue action for queue edit/delete/race coverage; existing V11/V14

B31|2026-10-08|Independent certificate extraction passed codesign an optional prefix as a separate argument|Use --extract-certificates=PREFIX; exact downloaded certificate and both architectures verified, no product change

B32|2026-10-08|Helper consumer retained windows-sys 0.52 after stable upstream adopted 0.61.2 and pointer handles|Inherit workspace bindings, refresh the lockfile and validate inheritance before builds; existing V10 stable-helper invariant

B33|2026-10-08|Automation inspected only styled-text items, so compiled plain sidebar labels and pencil glyphs were invisible to hit testing|Handle Slint simple, complex and styled text; require a measured thread label in every tooltip fixture; existing V12/V14

B34|2026-10-08|Narrow sidebar drawer was painted after the tooltip, putting its dimming layer above the popup|Paint the pane-bounded tooltip after drawers; existing V12

B35|2026-10-08|Steer-edit fixture assumed slow streaming retains pending input, but native steering immediately interrupts inference; reruns also inherited request logs|Hold a real tool open, require the original steer editor, clear request logs before each run; existing V14

B36|2026-10-08|Legacy upstream recency_at can equal rollout file mtime, which resume advances without conversation work|Use creation plus actual turn timestamps and live work notifications; Windows reopen regression records both caches; existing V13

B37|2026-10-08|Resume emits goal snapshots, including goal-cleared for threads with no goal; GUI treats snapshots as live mutations|Use goal updated_at and ignore clear without a known goal; snapshot/update/clear unit regression and Windows reopen check; V13

B38|2026-10-08|Hosted runner shared IP exhausts anonymous GitHub API quota before stable-version check|Use workflow read-only token only for api.github.com requests; stable check retained

B39|2026-10-08|GPU editor opens after fixed 400ms fixture delay, so automation types before the real form exists|Wait for settled editor open/closed state instead of elapsed time; V14

B40|2026-10-08|Narrow-hover fixture keeps pointer inside a recreated drawer row and assumes its tooltip is ready after 800ms|Enter from outside the row and wait for actual tooltip content before bounds assertions; V12

Imported upstream release evidence (does not verify this Rust/Rally migration): v0.4.0, source 1b512fecad7e28826ef98538a69dd9b05327275a; Windows run 37874908459 passed 633 tests, interaction gates, Azure signatures and attestation. Exact signed ZIP passed question forms, software/GPU purpose/selection and conversation tests on games, including pending edits/deletes/races and stable reopen activity; all signatures/timestamps and native helper IPC verified. Mac run 37874969609 passed 642 headless tests, signed helper IPC/JIT on both architectures, both Accepted notarizations, staples and Gatekeeper including mounted app. Strict Clippy passed; no Mac GUI launched. Online/bundled attestations and all eight public asset digests verified. Temporary Mac runner deregistered and removed. See GUI.md §26 and docs/releases.md.

B41|2026-10-10|Inherited CI/Mac scripts override root codegen units with 16|V15; audit every entrypoint, remove optimization overrides

B42|2026-10-10|External transport migration retained embedded facade/config field assumptions|V16; replace transport types and readonly config snapshot, compile all targets

B43|2026-10-10|New Rally UI assumed unsupported dynamic markup macro and Slint properties|V20; use Rust StyledText projections, supported selection offsets, and compiler-check native controls

B44|2026-10-10|Feedback fixture hardcoded imported client name after Fastrock rebrand|V16; fixture checks Fastrock external client identity

B45|2026-10-10|New proposal test omitted required summary; quit guard reached private migration fields|V19/V20; valid tool fixture, controller-owned quit/write guard, headless + Windows checks

B46|2026-10-10|Automation sampled Rally before queued native callbacks ran, bypassing settled write waits|V20/V22; yield a native event-loop turn after each Rally command; assert dirty/save and rich-write evidence

B47|2026-10-10|Saved-view search used an unsupported nested repeater/conditional expression|V20; conditional child layout and typed Rust predicate callback

B48|2026-10-10|Keyboard FocusScope wrapper inherited a Rectangle background declaration|V20; keep visual properties on Rectangle and compile native focus controls

B49|2026-10-10|Expanded automation was copied before the matching executable finished building|V22; serialize build completion and deploy, record executable hash with fixture evidence

B50|2026-10-10|Board restore compared a partial later page's length to the collection total|V18/V20; compare absolute page end and retain the loaded working set across refresh/restore

B51|2026-10-10|User picker assumed Name; fixture conflated current-user GET and paged User query, hiding an open empty picker|V18/V20; DisplayName/UserName labels/order/search, distinguish fixture endpoints, assert picker closes and no view errors

B52|2026-10-10|Create fixture expected the restored board working set to grow automatically to its new total|V18/V20; wait for retained loaded window, assert create payload/state and updated server totals separately

B53|2026-10-10|Bulk form was declared after its reference picker and intercepted picker clicks|V20/V22; place picker above parent form; real native User-button click must close it

B54|2026-10-10|Changing board group retained later-page cursor, so reload appended duplicate records; screenshot check caught missing error assertion|V18/V20; reset cursor before grouped reload, retain resident working set, assert settled swimlane errors and row count

B55|2026-10-10|All toolbar rows plus fixed 260px narrow assistant exhausted the document height at 900x700|V20/V22; bound toolbar height with native ScrollView and size bottom dock to viewport, inspect narrow document/assistant screenshot

B56|2026-10-10|Selected Rust cache action exports CARGO_INCREMENTAL=0 and discards incremental artifacts despite workflow env=1|V15/V22; use ordinary Actions cache for dependencies and full target, retain incremental setting, policy gate checks runtime environment

B57|2026-10-10|Save replaced entire editor with mutation response, losing omitted fields, complete collections and edits typed while request ran; reload omitted selection hydration|V19/V20; merge acknowledged snapshots, preserve late field/comment drafts, hydrate reload selections, fixture sparse responses and delayed writes

B58|2026-10-10|Generic validation/session error cleared busy during an unrelated in-flight mutation|V19/V20; preserve busy until owning write callback finishes; delayed-write invalid-input fixture checks busy and retained draft

B59|2026-10-10|Fixture created lower-case _type, hiding new artifacts from its case-sensitive mixed query|V18/V20; use canonical payload type, page beyond retained window, assert created row is present

B60|2026-10-10|Hosted OS mouse gesture failed to reach DragArea although Rust tests/build and remaining native callback scenarios ran|V20/V22; hosted callback mode with honest receipt; retain actual mouse requirement on games and upload failure evidence

B61|2026-10-10|Save-and-continue consumed pending navigation after acknowledgement even when a later draft remained dirty|V19/V20; continue only after clean acknowledgement; retain draft/intent and re-show guard; delayed-navigation fixture asserts editor remains open

B62|2026-10-10|Delayed-navigation fixture reset Name to its acknowledged baseline, so no dirty Save-and-continue path existed|V22; use a distinct delayed title and assert later draft remains in the open editor

B63|2026-10-10|GitHub runner config contains UTF-8 BOM; registration succeeded but plain JSON decode stopped listener startup|V23; read utf-8-sig, verify/resume this offline runner, remove registration after its one job

B64|2026-10-10|Generic collection guard treats AllowedAttributeValue _ref="null" as shared object identity; documented unpaged catalog also uses StartIndex=0; simplified fixtures omitted both real WSAPI forms|V24; narrow metadata normalization, retained object guards, headless and Windows regressions

B65|2026-10-10|Symbol fixture linked and deleted temporary object before dsymutil; UUID-only check accepted dSYM without debug sections|V25; retain fixture object, require nonempty DWARF info/line sections and reject empty matching-UUID symbols

B66|2026-10-10|Cargo keeps hash-suffixed link filename inside dSYM after exposing public fastrock binary; filename-based symbol check rejects valid matching UUID|V25; resolve sole DWARF file by UUID and verify sections, hashed-filename regression; preserve failed tag and use stable +build.1 packaging tag
B67|2026-10-25|Goose queue drain and prompt completion formed recursive opaque async Send types|V26; boxed queue scheduling boundary, concurrent/queued prompt fixture and Windows compile

B68|2026-10-25|ACP live item projections omitted mandatory UI lifecycle timestamps; replay-only coverage missed event deserialization|V26; stamp millisecond item lifecycle events and deserialize every live event kind in Windows regression

B69|2026-10-25|Queue persistence referenced an undeclared hash crate|V26; reuse existing base64 path encoding; no new hashing dependency

B70|2026-10-25|UI summary reused Codex permission labels and unfiltered child-thread lookup showed the parent as its own sub-agent|V27; Goose-owned permission summary, live model/provider refresh, reject unsupported child enumeration

B71|2026-10-26|Rally child window dropped explicit CLI backend selection and defaulted to Goose during Codex acceptance|V27; forward connection address/token-variable name to child; native fixtures exercise both backends
B72|2026-10-26|Hosted Windows runner lacked PSGallery registration, preventing Azure signing module installation|Register default repository before signing; preserve failed tag and use stable +build.1; existing V29 signing gate retained
B73|2026-10-26|Setup guidance omitted virtual/host file and skill/instruction boundaries, allowing deliberately disabled Developer to be mistaken for missing ACP capability|Document vwork-only workflow, host-side global instruction setup and 1.56.0 hint loading; no runtime defect established, no new invariant

B74|2026-10-26|Unconditional Windows full-window invalidation, opt=0 raster/text dependencies and monolithic table/tool rows amplify scrolling cost|V31,V32,V33; retained framebuffer, authorized dependency optimization, bounded native rows, same-fixture benchmark

B75|2026-10-26|Replacing shorthand padding while retaining explicit left override produced duplicate Slint binding|Use one binding per edge; compile-time regression, existing V20
B76|2026-10-29|Goose history pages clone whole store; replay builds then discards presentation events; UI waits for activation despite received history|T32; signed 0.3.2 Windows baselines and installed-Goose delayed-MCP reproduction recorded in docs/PORT-VERIFICATION.md; optimization pending, user's severe delay not reproduced with clean config
B77|2026-10-29|Startup benchmark returned empty success for unknown MCP discovery, masking extension failure; reload check assumed uncached tool-list fetch|V34; method-not-found response, activation/initialize/replay assertions, cached catalog allowed; repaired full Windows benchmark passed

B78|2026-10-29|Each Goose activation launches stdio vwork bridge despite persistent database daemon|V35; forward same-name MCP extension through localhost Fastrock route to authenticated daemon
B79|2026-10-29|Compaction targeted whole database but UI required selected workspace|V36; independent database selector and explicit whole-database action
B80|2026-10-29|Manual Windows test launch omitted FASTROCK_TEST_PYTHON; eight embedded Goose fixtures failed before startup|Use installed Python path, matching CI setup; rerun passed 706 tests
B81|2026-10-29|End-to-end MCP probe used internal workspace_info operation instead of advertised vwork_workspace_info tool|V35; use native MCP name; daemon correctly rejected unknown tool
B82|2026-10-29|Selection touch areas stop at text bounds; adjacent layout padding never starts a drag|V37; horizontal selection gutters with unchanged glyph geometry and Windows pointer regression
B83|2026-10-29|Tool results use passive labels; full arguments and single-line commands are absent from expanded details, and copy retains only results|V37; selectable detail text, complete call copy source, retain bounded virtual rows
B84|2026-10-29|Shared-to-native selection focus cleanup resets the TextInput anchor after its pointer event|V9,V37; clear shared state while preserving the newly focused native selection; gutter-to-glyph Windows regression
B85|2026-10-29|0.3.x lock pins shared chrono/uuid below embedded Goose 1.57.0 minimums; individual updates expose next locked conflict|T36; refresh compatible dependency resolution with upstream Git revisions pinned; migration-only, no new invariant

B86|2026-10-29|Optional Goose symbol parser and retained Codex UI protocol link incompatible tree-sitter ABIs|T36; disable optional Goose tree-sitter feature; tools/code mode retained, Goose unchanged
B87|2026-10-29|Broad resolver update chose transitive Rama crates requiring newer Rust than available toolchain|T36; declare Goose minimum Rust 1.94.1 and use compatible-version fallback
B88|2026-10-29|Cargo strips host proc-macro debug symbols despite parent strip=none; rust-objcopy emits macOS 27-invalid LINKEDIT alignment|V46; explicit build-override strip=none, retain unoptimized build scripts; Rust issue 157750
B89|2026-10-29|Embedded workspace fixture compared line-oriented MCP preview with byte-exact file content|T38; verify round-trip via editor read API; fixture-only correction, no new invariant
B90|2026-10-29|Bedrock fixture selected optional Axum OriginalUri extractor absent from minimal build|T36; use standard HTTP Uri extractor; fixture API correction, no new invariant

B91|2026-10-29|Temporary Windows runner used invalid PowerShell variable colon syntax and lost completed process exit codes|Use explicit variable delimiters and Process handle before waiting; fixture harness only
B92|2026-10-29|Historical virtual UI test forbade redb after authorized embedded storage migration|Replace obsolete dependency assertion with queued background compaction contract; V38,V40
B93|2026-10-29|Customized legacy vwork entries could silently retain external stdio fallback|Reject unmappable vwork configuration before activation; embedded discovery fixture; V38
B94|2026-10-29|Session restore passed owned cwd into new borrowed discovery parameter|Borrow cwd at call site; compiler mechanical fix
B95|2026-10-29|Graceful-close request made shutdown fail after a known disconnect|Always abort/join owned tasks after bounded close attempt; disconnect_fails_pending regression; V38,V47

B96|2026-10-29|Shared Slint ContextMenuArea collided with inherited close function, then compiler inlining panicked with Menu plus children slot|Keep native menus inline and pair accessibility labels with roles; retain compile oracle; V1
B97|2026-10-29|Shared Close other tabs crossed Rally/conversation areas and bypassed Rally write/draft guards|Scope bulk close to same kind; reject bulk close while Rally writes/drafts need handling; native navigation fixture; V19,V42
B98|2026-10-29|Backend status overlay covered the permanent Rally sidebar project|Limit overlay to conversation pane; native injected failure still permits Rally navigation; V42
B99|2026-10-29|Inherited startup code launched a login shell and Linux renderer replacement process|Remove startup probes/relaunch; inherit environment and report renderer failures in process; V38

B100|2026-10-29|Multiwindow compile used immutable method as mutable controller callback and lacked ComponentHandle import|Use closure adapter and import trait; V48
B101|2026-10-29|New native windows exposed singleton callback routing, duplicate mailbox writers and Rally assistant registration/close races|Window-scoped callbacks/tasks/input, shared ordered mailbox writer, attach-before-prompt and temporary-session cleanup; V48,V42
B102|2026-10-29|Weak mailbox registry could replace an actor while its last writes drained|Retain sender through runtime shutdown; dropped-handle reattachment regression; V48
B103|2026-10-29|Window-close path saved UI preferences synchronously on the Slint thread|One coalescing background writer, flush on explicit shutdown, latest-save/error regression; V47
B104|2026-10-29|Slint tray attempted its one-time native initialization before the async icon arrived, leaving no tray|Construct tray only after decoding its initial icon; native close/background/restore/menu-Quit fixture; V47
B105|2026-10-29|Tray fixture treated Winit's transparent WS_VISIBLE event target as a user window and destroyed callback delivery with WM_CLOSE|Exclude internal event window from native window enumeration; retain timeout diagnostics; V47,V48
B106|2026-10-29|Synthetic End/Return did not activate the native tray menu, repeating B14's modal-menu driver limitation|Use process-owned IAccessible default action on the verified Quit Fastrock item; V47
B107|2026-10-29|Queue dequeue and dispatch evidence used separate files; restart could expose an uncertain send as pending and a later add could drain it|One atomic queue/dispatch record; recover pending work paused and quarantine uncertain sends; v41 queue crash-boundary fixtures and explicit Resume UI; V41,V47
B108|2026-10-29|Native fixture captured clipboard before its deferred Copy callback executed|Yield after native Copy before capture; recovery fixture verifies copied record; V41
B109|2026-10-29|Goose replay changes turn/user IDs; retained live projection duplicated interrupted prompt|Replace server projection on reattachment; retain pending/unsent input and notes; v41_goose_reload_replaces_live_ids_and_keeps_pending_and_unsent plus native visible-history export; V41,V43
B110|2026-10-29|Failed completion persistence retained a dispatch claim that Resume could not clear|Quarantine the in-memory record; disk evidence remains recoverable; v41_failed_completion_save_remains_reviewable_and_resumable; V41,V47
B111|2026-10-29|Recovery fixture expected live unpaused state after explicit Quit had closed the session|Assert persisted unresolved evidence is paused for next launch; native recovery fixture; V41,V47
B112|2026-10-29|Slint globals retained rows/cards/editors after document switches and closing|Clear obsolete document models and assistant buffers; native navigation fixture checks detached editor models, zero retained rows after close and successful reopen; V42

B113|2026-10-29|Open chats/drafts existed only in memory; backend reattachment dispatched pre-start inputs automatically|Atomic coalesced session snapshots, explicit Resume, review-only recovered local sends and durable attachment paths; V41,V47
B114|2026-10-29|Snapshot capture moved non-Copy reasoning choices from borrowed state|Clone optional reasoning choices; compile oracle; no new invariant
B115|2026-10-29|Native recovery fixture decoded UTF-8 dumps with Windows locale encoding|Read JSON explicitly as UTF-8; source/recovered files preserved identical Unicode; V47
B116|2026-10-29|Composer cleared input before backend rejected a send to a recovered paused queue|Check local/backend recovery pause before changing the draft; native recovery send-refusal check; V41,V47
B117|2026-10-29|Process-recovery fixture used a fixed delay for asynchronous summary completion|Wait on the shared worker becoming idle after turn completion; V44,V47

B118|2026-10-29|Release GUI fixtures still selected removed external ACP backends|Exercise embedded Goose against isolated local Bedrock/MCP fixtures; preserve native Rally/selection/scroll coverage and memory caps; V38,V39,V42,V46
B119|2026-10-29|Local provider fixture compared prompts including Fastrock turn-context metadata|Strip the appended turn-context before choosing deterministic fixture responses; native embedded streaming/cancellation fixture; V39

B120|2026-10-29|Workspace project module inherited sha2 from nonexistent workspace dependency entry|Declare matching direct dependency; manifest/compile oracle; no new invariant
