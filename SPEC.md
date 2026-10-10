# SPEC

## §G GOAL
Replace Go Fastrock with codex-gui Rust/Slint; preserve Rally inventory and external installed Codex; fast prerelease iteration.
Broader GUI contract: repository `GUI.md`.

## §C CONSTRAINTS
- Fastrock derives from pinned allquixotic/codex-gui; Apache-2.0 and Slint attribution preserved.
- Installed external Codex over stdio; no embedded runtime/core/V8. Protocol types pinned to stable baseline.
- Every build prioritizes compile speed; no optimized dependency overrides, LTO, stripping or packing without explicit user authorization.
- New Rally UI uses native Slint controls/layouts; horizontal documents, conversation-only sidebar.
- Track Mac/games free space; clean identified stale build junk only when needed.
- Native Slint 1.18.1; bound transcript memory; no idle repaint polling.
- Fastrock alpha: signed Windows x64 fast ZIP and signed/notarized/stapled Apple Silicon Mac DMG via CI. Imported upstream universal release records remain historical.
- Commit/push main directly; no draft PR workflow. CI builds/signs/verifies/publishes releases with persistent incremental caches; no unsigned fallback.
- Never run GUI tests or launch this project's GUI on Sean's Mac unless the user explicitly says "test the GUI on this Mac"; use Windows hosts.
- Publish only after fast development verification passes; never add an optimized build pass.
- Verify GitHub asset digest before deleting local build outputs.

## §I INTERFACES
- I.scroll: wheel, scrollbar, Jump to latest in conversation pane.
- I.paint: Windows restore, remote desktop exposure, existing redraw events.
- I.questions: synchronous server-request forms; asynchronous message forms and ordinary send/steer/queue replies.
- I.speed: provider-catalog speed picker; thread/settings/update and turn/start.
- I.summaries: sidebar titles/tooltips, resize handle; GUI-only persistent cache, ephemeral fast-model requests.
- I.distribution: Azure Authenticode, Developer ID Application, Apple notarization/stapling, GitHub artifact attestations.
- I.links: glyph hover, Copy link, Open in browser, Open file.

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

V15: All Cargo profiles ! opt-level=0, debug=0, LTO=false, codegen-units=256; build scripts/CI ! no optimization overrides. Verify: dev/check-build-policy.py.
V16: Startup ! launch installed external Codex after first frame; inherited env/config; process cancellation/restart/disconnect ! retain unsent drafts. Verify: transport::tests plus Windows mock-process smoke.
V17: Rally tokens ! OS vault only, no logs/prompts/settings; refs ! same-origin WSAPI; HTTP ! loopback only; mutation ! never auto-retry. Verify: rally::client::tests.
V18: Query/search/filter/preset/types/export ! retain selected scope, paging and loaded-only totals; metadata ! per-workspace/type. Verify: rally::view::tests and fixture integration.
V19: Writes ! review/revision/scope guards; dirty/conflict drafts retained; batch failure ! exact completed count; no claimed atomic transaction. Verify: rally::editor::tests, rally::assistant::tests.
V20: Every inventory capability R01–R22 ! Rust implementation plus named fixture/Windows evidence before marked complete; native Slint UI only. Verify: docs/RALLY-INVENTORY.md ledger and dev/windows-rally-smoke.py.
V21: Personal settings/views/session ! migrate legacy names and independent snapshots; vault service ! fastrock. Verify: rally::store::tests.
V22: Windows prerelease ! fast profile, exact source/asset metadata; tests ! fixture-only, software renderer; Mac GUI ! never launched. Verify: workflow/policy audit and Windows smoke.
V23: Release ! commit/push main, tag, CI only; exact source/run/unsigned input hashes verified before signing; Windows ! expected publisher + RFC3161 timestamp + fixture acceptance of signed bytes; Mac ! pinned Developer ID + Accepted app/DMG logs + stapling + Gatekeeper including mounted app. Cache ! persistent, no per-release archive wait; no PR. Verify: release.yml, signing receipts and GitHub attestations.

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
T24|~|Land port directly on main; fast cached CI signs/verifies/publishes Windows and Mac prerelease|V15,V22,V23,I.distribution

Port acceptance: v0.2.0-alpha.3, source 97a2ef394b1591ba9e0421a2cf376055899ece30; 676 Mac headless tests; games native mouse/picker, 82 fixture requests/13 writes, late drafts/restart/window transfer; both OS binaries unoptimized; no Mac GUI or real Rally writes. ZIP SHA-256 a6f56551249d377e292db78735622e4af59d0e9c2947f79478cba299ab5d0d75. Hosted run 38036468741: 667 Windows tests + callback UI/package/cache passed on alpha.2 source. Detailed evidence: docs/PORT-VERIFICATION.md.

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
