# Fastrock improvement suggestions

Review date: 2026-10-09 · Source-only review: no code was edited and no GUI was launched.

**References**
- Codex GUI: `/Users/sean/dev/codex/codex-rs/gui/{ui,src}`, cited as `R ui/...` or `R src/...`.
- Rally reference: `ignore/index.html:N`. Line numbers there are approximate.
- Spec: `ignore/SPEC.md`.
- Fastrock paths are relative to the repository root.

Within each section, the highest-impact items come first. Items tagged **(bug)**, **(data loss)** or **(security)** are defects, not polish.

---

## 1. Fix first: crashes, data loss, security and wrong writes

### Crashes and hangs
- **(bug)** Clicking a conversation while Codex is disconnected panics a worker goroutine. `resumeThread` captures `a.client` without a nil check (`internal/ui/chat.go:176-182`). Guard it before `a.work`.
- **(bug)** `answerApproval` has the same nil-client panic (`internal/ui/approvals.go:186-188`). Clear `a.approvals` when `consume` detects a disconnect (`chat.go:453-473`), as `popout.go:335` already does.
- **(bug)** Approval and dynamic-tool replies go out on whatever `a.client` is current, not the connection that delivered the request (`xtab.go:39-42`, `approvals.go:186`). Reply-wait timers last up to 1800 s (`xtab.go:138-150`), so after a reconnect they answer on the wrong server, and after a disconnect they panic. Store the originating client on each request.
- **(bug)** Async callbacks keep `&s.Tabs[i]` from `State.Current()` (`internal/workspace/state.go:198-205`) and use it after an RPC (`chat.go:961` renames, `chat.go:982` closes). Any intervening Move, Close or Open renames or closes the wrong tab. Capture the tab ID and look it up again.
- **(bug)** Nothing recovers panics. `a.work` is a bare `go f()` (`app.go:276`), as are the broker request goroutines (`broker.go:198`) and the layout workers (`app.go:144-153`). A panic in the primary process also kills the broker that every pop-out depends on. Add a recover wrapper that logs, posts a toast and returns an RPC error.
- **(bug)** One JSON-RPC line over 32 MiB (for example an unpaged long-thread `thread/resume`) fails the scanner, cancels the client, kills app-server and broadcasts `serverStopped` to every window (`codex/client.go:136-137,161-177`, `broker.go:112-113,504-512`). Page history with `thread/read`/`thread/turns/list`, as the reference does with `exclude_turns:true` (`R src/session.rs:78-129`), and reject oversize frames without tearing the connection down.
- **(bug)** Rally-assistant approvals and questions are queued (`chat.go:503-506`) but drawn only when a chat tab for that thread is active (`approvals.go:277-292`). The assistant has no tab, so its turn hangs. Render approvals inside the assistant panel.
- **(bug)** Approvals for sub-agent threads, or for threads that aren't the active tab, wait invisibly forever. Route them to the nearest open ancestor tab with an origin badge, or open the thread or auto-cancel, as the reference does (`R src/approvals/mod.rs:241-311,710-718,800-830`).
- **(bug)** When no window owns the thread, the broker sends server requests to a random peer (`broker.go:480-486`). Route to the window that published the thread, broadcast, or reject.
- **(bug)** Approvals are never pruned. `serverRequest/resolved` is ignored, and nothing removes requests on `turn/completed` or tab close (`chat.go:526-538`, `lifecycle.go:14-56`). Handle all three.

### Credentials and code execution
- **(security)** `StartCommand` sets no `cmd.Env` (`codex/client.go:107`, `process_*.go`). `FASTROCK_RALLY_TOKEN` and `FASTROCK_AUTOMATION` therefore reach app-server and every shell command the agent runs. Pop-outs also keep `FASTROCK_BROKER`/`FASTROCK_BROKER_TOKEN` (`popout.go:266`), which git hooks and `open`/`rundll32` children inherit. Add one `platform.ChildEnv(drop...)` helper, use it for every child process, and unset the broker variables after `Dial`. It replaces the two inconsistent scrub lists at `popout.go:507-516` and `update/install.go:27-35`.
- **(security)** "Open externally" uses `rundll32 url.dll,FileProtocolHandler` / `open` on any path (`internal/ui/files.go:23-46`). Paths can come from agent transcripts (`info.go:212`), so `.exe`, `.bat`, `.app` and `.command` files launch without asking. Mirror the reference's "Run this file?" confirmation with a destructive "Run" button (`R src/files/viewer.rs:1175-1225`).
- **(security)** The automation harness ships in release builds and is enabled by an environment variable (`automation.go:24-33`). It can send chats, save Rally items and take screenshots to arbitrary paths. Put it behind a build tag.
- **(security)** Updates are integrity-checked but not authenticated: the SHA-256 comes from the same release that serves the asset (`update/update.go:190-216`). Sign `SHA256SUMS` (minisign/ed25519 with an embedded public key) and verify the signature before staging.

### Silent changes to model, effort and speed (V2, §C)
- **(bug)** The chat composer runs `c.Effort = efforts[ComboSimple(...)]` every frame (`chat.go:872-874`). When the effort is empty or not in the list (common after resume), it silently becomes `efforts[0]`, the lowest effort, and is sent with the next turn. Assign only when the selection changes, and show "Default effort" for an unset value (`R src/composer/mod.rs:767-771`).
- **(bug)** The Rally assistant forces `Tier:"default"` and the model's default effort, ignoring the user's Codex config (`assistant.go:38`). It also overwrites both from its combos every frame (`assistant.go:78,91`) and sends them explicitly (`:180,:195`). Omit tier and effort unless the user picked one.
- **(bug)** Any `model/list` or `config/read` error sets `a.fatal` and replaces the whole UI with "cannot start / Exit" (`app.go:313-325,501-512`). This includes the reload after a Settings restart or provider change (`popout.go:355`, `session.go:223`). Make it fatal only at first startup; afterwards show a retryable banner and keep the previous catalog.
- **(bug)** Changing the model silently resets an unsupported tier to Standard (`chat.go:852-858`), and the speed combo shows "Standard" for an unadvertised tier (`chat.go:877-889`). Show the Ultrafast/Bedrock explanation inline next to the combo.

### Turns, queues and drafts
- **(bug)** An `error` notification with `willRetry` sets `c.Status="error"` (`chat.go:565-566`). The tab turns red, Stop disappears, and the next Enter starts a second turn while the first is still running. Ignore retrying errors, as `R src/app.rs:884` does.
- **(bug)** Messages sent while a resumed thread is "starting" are queued (`chat.go:333-338`, `workspace/state.go:52`), but resume completion never drains the queue (`chat.go:216-218`). Drain on resume, as `R src/threads.rs:364` does.
- **(data loss)** Draining the queue after `turn/completed` writes the queued text straight into `Editor.Buffer` (`chat.go:365-369`), replacing what the user is typing. Use a nil-checked view and leave the live draft alone.
- **(bug)** Restored queued prompts auto-send after the next completed turn (`chat.go:536-538`), and queues keep sending after a failed or interrupted turn. Pause the queue after error or interrupt and require an explicit resume; the spec says "never auto-submit restored queued work".
- **(bug)** A queued message can be sent twice during pop-out. The source still pops its queue on `turn/completed` after the finalize snapshot (`popout.go:359-370` vs `broker.go:537-560`), and the destination replays the buffered event against that snapshot (`popout.go:389-398`). Freeze queue dispatch once `a.popping[tab]` is set.
- **(data loss)** `/mention` replaces the entire draft with `"@path "` (`slash.go:27-28`). Insert at the caret and open the file popup (`R src/composer/mod.rs:622-633`).
- **(data loss)** "Forward to conversation…" calls `setText` on the target's composer (`transcript.go:348-355`), overwriting its draft. Use a forward dialog with Message, an optional Note, Send now / Queue, and provenance (`R ui/xtab.slint:142-275`).
- **(bug)** `/cmd trailing text` runs the command and discards the text; `/diff looks wrong` runs a diff and clears the input (`chat.go:323-326`, `slash.go:12-28`). Only review, rename, side, plan, new, fork and resume take arguments; send anything else as a normal message (`R src/composer/slash.rs:252-309`). Also pass the arguments through for rename, review, side, new, fork and resume.
- **(bug)** `/plan msg` toggles Plan, so it turns Plan *off* when already on (`slash.go:14-18`). Only enable it.
- **(bug)** With the suggestion list open, Enter sends the half-typed `/mod` instead of accepting the suggestion (`interactions.go:261-284`). Accept on Enter or Tab (`R ui/composer.slint:205-208`).
- **(bug)** Escape is a global binding whose handler always reports "handled" (`interactions.go:30,63-73`, `actions.go:213-228`), so popups, suggestions and editors never see Esc. Consume it only for an active turn after approvals; otherwise close the top popup or drawer first (`R src/app.rs:1286-1300`).
- **(bug)** Any message containing `$` (such as `$HOME` or `$5`) first calls `skills/list`, and if that call fails the message is not sent (`chat.go:406-415`). Use cached skills and never block sending on them.
- **(bug)** Both policy-amendment choices use hotkey `p` (`approvals.go:264-269`). The network rule is also mislabelled: it is a *deny* rule but reads "Apply proposed network rule". Use `d` and "No, and block this host in the future" (`R src/approvals/request.rs:519-541`).
- **(bug)** `/approve` calls `thread/approveGuardianDeniedAction` without the required `event` (`slash.go:62-63`). Record auto-review denials, confirm with a summary, and toast when there is none (`R src/composer/approve.rs:55-226`).

### Destructive actions without confirmation
- **(bug)** `/logout` signs out immediately (`slash.go:64-65`), and Settings › Account › Sign out has no confirmation (`settings_extra.go:94-106`). `/logout` should open Settings › Account (`R src/composer/mod.rs:648`).
- **(bug)** "Return to OpenAI…" always calls `account/logout` (`bedrock_settings.go:63-67`), which can delete stored ChatGPT or API-key credentials. Sign out only when Codex stores the Bedrock credentials; otherwise remove the provider and Bedrock keys (`R src/settings/bedrock/flow.rs:673-697`).
- **(bug)** Closing a busy tab or quitting leaves turns running with no UI (`lifecycle.go:14-56,129-142`, `menu.go:41-42`, `slash.go:66-67`). Add "Close running thread?" with a destructive "Stop and close", and "Quit?" when turns run (`R src/app.rs:1090-1128,1604-1640`). Also send `thread/unsubscribe` on close; Fastrock never does (`R src/threads.rs:555-621`).
- **(bug)** `Client.Close` closes stdin and then immediately cancels the `exec.CommandContext` (`codex/client.go:224-227`), force-killing app-server before it can shut down cleanly and orphaning its MCP and terminal children. Close stdin, wait with a timeout, then kill the whole process tree (process group on macOS, Job Object on Windows).

### Settings that write the wrong thing
- **(bug)** The Add MCP server form writes with `mergeStrategy:"replace"` and no duplicate-name check (`mcp_settings.go:95`), silently overwriting an existing server. It also never calls `config/mcpServer/reload`. Check names, confirm overwrites, and batch the write with a reload (`R src/settings/mcp.rs:579-722`).
- **(bug)** Feature toggles call the process-wide `experimentalFeature/enablement/set` (`settings_extra.go:226`) and are lost on restart. Write `features.<name>` to config.toml (`R src/settings/extensions.rs:496-507`).
- **(bug)** The font-size combo lists 11–16, 18 and 20 only (`settings.go:91-97`). `index()` returns 0 for missing values, so a saved size of 10, 17, 19 or 21–24 is rewritten to 11 just by opening Appearance. Use the full 10–24 range (the reference allows 8–32) and never write unless the user changed the value.
- **(bug)** Bedrock's API token and secret access key share one editor (`bedrock_settings.go:38,41`), so switching credential method carries a secret into the wrong field. Use separate editors and clear secrets on method change.
- **(bug)** The Account device-code toast is immediately overwritten by "Copied" (`settings_extra.go:68-71`), so the code is never visible. Show it in a card.
- **(bug)** Account Cancel sends an empty login ID (`settings_extra.go:94-106`).

### Wrong Rally writes and lost Rally edits
- **(bug)** Defect boards use story states. `StateField("Defect")` returns `State`, but `States()` falls back to the ScheduleState list (`internal/rally/catalog.go:44-60`). Lanes are wrong, and drops and bulk "Complete" write `State=Completed`, which isn't valid. Take lanes from schema AllowedValues; for flow boards use ScheduleState.
- **(bug)** Portfolio Kanban drops and bulk Complete send State as a name string (`board.go:302-315`, `rally.go:331-334`), while the detail editor correctly sends refs (`detail.go:262-273`). Items with no state map to an invented "No Entry" lane (`board.go:65`). Send the State ref, or null.
- **(data loss)** `openArtifact` shows the list projection (empty Description, editable) and then replaces it with the full load without a dirty check (`detail.go:62-93`). Edits typed while loading are lost. Make fields read-only until the full load arrives, or merge.
- **(data loss)** Opening a child or task from a detail replaces it with no unsaved-changes guard, and Back then returns to the board instead of the parent (`detail.go:651-656`).
- **(data loss)** An unsent discussion comment doesn't count in `dirty()` (`detail.go:367-382`). Close, Back and AI view changes discard it silently (`lifecycle.go:15,136`, `assistant.go:252-256`). Saving also recreates the comment editor (`detail.go:506`).
- **(bug)** On create, Save overwrites the chosen Project with the scope project (`detail.go:474-476`).
- **(bug)** Task and Defect details show an Acceptance Criteria editor (`detail.go:52-54,242`), and typing there sends an invalid field. Build editors from the type's schema.
- **(bug)** `v.Selected` is never cleared after bulk actions, filter or scope changes, or eviction (`detail.go:664-669`). Bulk edit and delete therefore hit hidden or filtered items. Clear selection on scope/filter change and after apply, and act only on visible items. Show the count and IDs in the confirmation (`ignore/index.html:2256`).
- **(bug)** `Fields`/TypeDefinition (`internal/rally/client.go:295`), `All("State")` (`detail.go:75`) and ConversationPost (`detail.go:530`) send no workspace, so they hit the user's default workspace and can return the wrong custom fields and states.
- **(data loss)** Unsupported HTML is silently lost after any edit (V9). Parse drops script, iframe and svg, turns images into `[alt]` and flattens tables (`internal/richtext/document.go:96-99,139-147,173-175`), and `HTML()` then rewrites the field. Detect unsupported nodes at parse time, show a banner, and open those fields in HTML-source mode.
- **(bug)** `Document.Restore()` doesn't rebuild `initial` (`document.go:487-495`), so text edited back to the original after a transfer or restart is no longer emitted byte-for-byte.
- **(bug)** "Open web" builds `/#/detail/hierarchicalrequirement/…`; Rally uses `userstory` (`detail.go:127`). It and Delete… also appear as no-ops on unsaved new items.

### Persistence and processes
- **(data loss)** A corrupt `session.json` is silently replaced at the next checkpoint (`session.go:26-33`), losing drafts and queues. Rename it to `session.json.corrupt-<ts>` and toast.
- **(bug)** There is no single-instance lock (`cmd/fastrock/main.go`). A second launch starts a second broker and app-server, and both overwrite `session.json` and `settings.json`. Use a lock file or named mutex and forward the second launch to the running broker.
- **(bug)** `Defaults()` sets `WorkingDirectory` to `os.Getwd()` (`internal/settings/settings.go:49-51`), which is `/` when launched from Finder, the Dock or the Start menu. Default to the home directory.
- **(bug)** The file viewer's find lowercases the whole buffer and reuses those offsets (`files.go:172-204`), so matches drift after characters whose lowercase has a different byte length. Use case-folded comparison on the original offsets.
- **(bug)** If both restart and rollback fail, the deferred cleanup can delete the new copy, leaving nothing at `Target` (`update/install.go:107-135`). Never delete the last runnable copy.

---

## 2. Performance and responsiveness

### Frame scheduling and rendering
- Redraw requests accumulate instead of collapsing. `Changed()` adds 1 (`third_party/nucular/masterwindow.go:146-148`), and the poller removes only 1 per 20 ms tick (`shiny.go:257-260`). `post()` calls `Changed()` per callback (`internal/ui/app.go:267-275`), `transcriptLayout` once per pending block per pass (`transcript.go:311-319`), and the Rally debounce every frame (`rally.go:158-162`). After streaming or opening a long chat, the window keeps doing 50 Hz full passes for seconds or minutes. Make `changed` a flag (`max(cur,1)`).
- Replace polling with event-driven wakeups. The updater sleeps 20 ms per loop (`shiny.go:210-279`), adding 0–20 ms of input latency and 50 wakeups/s per idle window. It forces an update every 10 ms while any mouse button is held (`shiny.go:263-272`), and `ebitenscreen` adds a 30 Hz ticker (`screen.go:80-99`). Use a wake channel signalled by `Changed()` and input.
- Every input event runs two full UI passes because nucular sets `changed=2`. Use one pass and trigger the second only when layout changed (for example the `trashFrame` case).
- "Follow latest" sets `Scrollbar.Y = 100000000` (`chat.go:777-779`). nucular clamps it and sets `trashFrame` (`nucular.go:640-648`), so every following chat renders twice per frame: 8.9 ms and 26 MB versus 4.3 ms and 13 MB measured at 200 blocks. Set it to content height minus viewport height.
- Any change repaints the whole window and copies the frame twice. Detection is all-or-nothing (`masterwindow.go:200-239`); `Upload` copies into a second CPU buffer (`screen.go:114-123`), and `WritePixels` uploads the full texture (`screen.go:318-321`). Compute a damage rectangle from changed commands, redraw under that clip, upload a sub-image, and swap buffers instead of copying.
- Replace the 10 s forced redraw ticker (`app.go:236-246`) with a timer that fires only when something time-based is visible (relative times, polling terminals).
- Replace busy-wait debounces with `time.AfterFunc`, starting with the Rally search debounce that redraws every frame for 300 ms (`rally.go:152-163`).
- Text wrapping rasterizes glyphs only to measure them: `textClamp` calls `Glyph()` instead of `GlyphAdvance()` (`third_party/nucular/font.go:113-125`). It sits under `WrapText`, which runs every frame for labels, Markdown and card titles.
- `ellipsize` trims one rune at a time and re-measures each prefix (`widgets.go:23-37`; about 22 µs cold per label), flooding the 2048-entry width cache. Binary-search the cut point and memoize by (string, width, face).
- `transcriptHit` re-measures every prefix on each drag move (`transcript.go:64-83`), which is quadratic. Accumulate per-character advances once per line.
- Layout workers create a fresh font face per job (`transcript.go:98`), so every measurement misses the cache. They also evict the UI's entries and hold the global `widthMu` lock (`font.go:37-72`). Give each worker a long-lived face and its own cache.
- Resize allocates new full-frame CPU buffers and a GPU image per size step (`screen.go:118-120,311-316`, `shiny.go:244-249`). Grow geometrically and reuse.
- Rounded rects and circles allocate a new rasterizer after each clip change (`shiny.go:402-422`). Cache pre-rendered corner masks per radius. Low priority.

### Per-frame work proportional to data (V4, V13)
- The info panel's "Other conversations" lists every conversation every frame: a full scan, lowercase, sort and one button per chat (`info.go:97-105`). It is on by default. Measured at 100k chats: 26 ms, 13.5 MB and 190k allocations per pass. Remove it (the reference has no such list) or virtualize it with the sidebar cache.
- The New tab page calls `a.state.Sidebar("", false)` every frame to show 8 rows (`app.go:814-820`). Use the cached sorted list.
- The "Choose conversation" popup renders every chat unvirtualized and has no search (`transcript.go:367-377`). Reuse the virtualized sidebar rows and add filtering.
- `State.Sidebar` builds `Title+" "+Cwd`, lowercases it and recomputes `ToLower(search)` per chat, then re-sorts everything (`workspace/state.go:206-223`). It runs on every search keystroke and every `turn/started`/`turn/completed` (`chat.go:571-573`): 23.9 ms at 100k. Precompute lowercase keys on metadata change and keep the order incrementally.
- The transcript calls `transcriptLayout` for every shown block every frame, up to 3000 after "Top", including folded and offscreen blocks (`chat.go:719-736`). `cut(b.Text, 60000)` decodes runes and allocates a ~64 KB string per large block per frame (`transcript.go:289`, `chat.go:93-105`). Use a prefix-sum height index with binary search (as the sidebar does), cache the cut source, and skip layout for folded blocks.
- The file viewer walks the whole buffer every frame (`third_party/nucular/text.go:980-1124,1519-1580`): 59 ms at 256 KB and 236 ms at 1 MB with wrap, which is on by default (`app.go:382`). Build a virtualized line view with a line-start index and a width-keyed wrap cache. The same applies to "Select text" editors and large rich fields.
- Pending approvals are JSON-decoded every frame and on every key (`approvals.go:103,277-292`). Decode once on arrival and store `threadId` and `reason`.
- Rally pickers rebuild their option lists every frame: projects (`rally.go:420-425`), iterations plus releases (`rally.go:180-185`), the detail sidebar's users, iterations, releases and projects (`detail.go:299-321`), and allowed values (`detail.go:201`). Build them once per scope load.
- Planning is O(iterations × items) per frame (`rally.go:645-658`), and the timeline draws up to 2048 rows without virtualization (`rally.go:662-671`). Precompute per-iteration totals and virtualize the rows.
- Team workload rows iterate a Go map every frame (`rally.go:635-639`), so they reorder randomly, flicker, and defeat the unchanged-frame check. Sort the keys.
- Each Rally page load sets `filterSource`/`cardSource` to nil (`stream.go:104-106`), throwing away the worker's search precompute. `filtered()` then recomputes `searchable()` for up to 2048 items on the UI thread (`rally.go:460-471`), and `prepareCards` rebuilds every card and wrapped title (`board.go:40-77`). Merge incrementally and keep cards keyed by ref.
- The select-all checkbox loops over every item each frame (`rally.go:535-543`). Keep a selected count.
- Assistant transcript: `s.Transcript += delta` is O(n) per delta (`assistant.go:211`); `markdown()` re-splits, double-wraps and regex-scans the whole transcript every frame (`markdown.go:12-47`); and the plan preview runs `json.Marshal` per change per frame (`assistant.go:108-112`). Use a builder plus a cached layout keyed by length and width.
- Discussions aren't virtualized, and each post's HTML is parsed into a rich editor during draw (`detail.go:638-648`). Parse on load in a worker.
- `rallySignature` does three `fmt.Sprint` calls per frame (`rally.go:137-139`), and the composer rebuilds its model, effort and tier slices every frame (`chat.go:842-889`). Cache them and rebuild on change.
- A preferences broadcast from any window, even a sidebar toggle, rebuilds the style (`popout.go:402-408`). `font.NewFace` SHA-256-hashes the whole font file on every call (`third_party/nucular/font/font.go:43-64`; 7.5 MB SFNS, 2.45 ms) and invalidates every cached width. Rebuild only when theme or font size changes, and hash once.
- The 1 s checkpoint snapshots every editor's text, deep-clones Rally objects and clones rich documents on the UI thread (`session.go:136-156` → `popout.go:82-125`; 1.9 ms for a 4 MB file). Use per-tab change counters and skip unchanged tabs.
- JSON on the UI thread: `upsertItem` runs `MarshalIndent` on file-change and MCP items (`chat.go:609-637`), and window-transfer payloads (possibly a 16 MB transcript) are unmarshalled in a UI callback (`popout.go:382-388`). Do both in workers.
- Board lane load-ahead triggers only on mouse-wheel scrolling (`board.go:205-210`). Scrollbar drags and keyboard scrolling never load more. Trigger it from the viewport position.

### Streaming and Codex transport
- Coalesce streaming deltas. Each delta is parsed or copied about six times: app-server read, broker `threadId` parse (`broker.go:476`), a fresh encoder per window (`broker.go:41`), the window read, a generic `codex.Decode` map (`chat.go:450`), then one posted closure plus `Changed()`. Forward raw bytes, decode delta methods into a typed struct, and merge consecutive deltas for the same item before posting.
- Route thread-scoped notifications only to the windows showing that thread. Today the broker sends every notification to every window synchronously with a 5 s write deadline (`broker.go:492-502`, `:37-42`), so one slow window stalls all of them, and every window redraws at the busiest window's streaming rate. Give each peer a bounded outbox goroutine.
- `Conversation.Append` calls `TrimTranscript` on every delta, scanning up to 3000 blocks (`workspace/state.go:106-136`). Keep a running byte total.
- `codex.Decode` wraps a `json.NewDecoder` around a bytes reader (`client.go:387-392`). Use `json.Unmarshal`.
- Cancel superseded `@`/`$` suggestion requests. `suggestions.go:79-86` overwrites `SuggestCancel` without calling it, so each keystroke's `fuzzyFileSearch`/`skills/list` runs to its 10 s timeout.
- Propagate cancellation upstream. The broker runs forwarded calls on its own context with a fixed 2-minute timeout (`broker.go:436-442`), and `Client.Call` just abandons on `ctx.Done` (`client.go:209-213`). Derive a per-peer context and send a cancel when the caller gives up or disconnects.
- Each layout job calls `font.NewFace` and `goldmark.New`, converts Markdown to HTML and re-parses the HTML (`transcript.go:97-105`). Reuse one face per worker and one goldmark instance, and walk the goldmark AST directly.

### Rally network and caching
- Bound all Rally requests. The 4-slot limit covers only `CachedQuery` (`cache.go:32,67-70`); `All`, `Get`, `Fields` (one request per attribute, `rally/client.go:309-316`) and mutations are unbounded. Move the semaphore into `request()`.
- Give shared in-flight loads their own context. Waiters receive the leader's cancellation (`cache.go:55-73`), so pressing Refresh during a load cancels the old context and the new request inherits "context canceled" plus backoff (`rally.go:122-124`, `stream.go:47-51`). Don't join cancelled flights. `PurgeCache` also clears in-flight entries and breaks dedupe (`cache.go:114-121`).
- Retry backoff sleeps up to 30 s × 3 while holding one of the 4 slots (`cache.go:67-70`, `client.go:118-139`). Release the slot while waiting.
- The 60 s auto-refresh replaces up to 2048 resident cards with one 128-item page (`rally.go:146-148` → `stream.go:59-64`), losing scroll position. Scrolling back then refetches from the server because the 45 s TTL has expired. Revalidate the loaded range in place (stale-while-refresh), and add jitter to the 60 s timer and the backoff.
- Cache the schema per type in `rally.Client`. Every detail open refetches TypeDefinition, attributes and one AllowedValues request per attribute, sequentially, plus *all* State objects unfiltered (`detail.go:74-75`, `client.go:290-320`), and shows nothing until all of it finishes. Render the `Get` result first and fetch the rest in parallel.
- Load scope lazily and lean. Every window process, including chat-only pop-outs, fetches every workspace, project, iteration, release and user with `fetch=true` at 200 rows per page (`app.go:235,386-472`, `client.go:162-164,211-238`), and refetches projects and users on every project change. Request only Name, `_ref` and dates, run the queries in parallel, page or search users on demand, and share results through the broker.
- Scope changes refresh inactive Rally views too (`rally.go:432-434`, `app.go:434-436`). Mark them stale and refresh when shown.
- Decode each page once into a typed envelope. Today each page is scanned three times and copied twice (`client.go:113-157,196-209`).
- Raise `MaxIdleConnsPerHost` to at least the fetch concurrency; the default transport keeps 2 (`client.go:52-55`).
- The search expression includes `Description contains` (`rally.go:93-95`), which is slow on large Rally workspaces. Make description search opt-in, or search Name/FormattedID first and description second.

---

## 3. Memory

- **Fix the pressure trigger.** It compares process RSS with the per-window *heap* budget (`lifecycle.go:76-85`). With 3+ windows (about 64 MiB threshold each, against ~87 MiB normal RSS per process), it fires on every 10 s sample, evicts all inactive Rally, chat and file state, purges caches and calls `FreeOSMemory`. Compare live Go heap with the heap budget and aggregate RSS with 384 MiB, add hysteresis, and evict least-recently-used first.
- `debug.SetMemoryLimit(256 MiB / windows)` ignores live heap (`lifecycle.go:76-77`). When one window's live heap exceeds its share, the GC runs almost continuously. Allocate the budget by live heap, or set a floor.
- Measure the right metric: phys_footprint on macOS and PrivateUsage on Windows, not resident size or working set (`platform/memory_darwin.go:28`, `memory_windows.go:21`). Create the `NewLazyDLL`/`NewProc` handles once, not per sample.
- Prune `chatView.Layouts` when blocks are trimmed (`transcript.go:285-321`). Each layout keeps the dropped block's text alive, so busy chats grow without bound.
- Delete `c.streams` builders on `item/completed` (`workspace/state.go:85-117`, `chat.go:639-646`); they keep about 2× the text and are ignored by the 16 MiB trim.
- Release `a.infoViews` on tab close (`info.go:29-37`); each can hold 1000 terminal rows. Busy chat views closed via `lifecycle.go:46-49`, and archive from the info pane (`chat.go:981-983`), also skip release.
- Cancel detail loads and collection workers on Back or close (`detail.go:70-92,514-561`).
- File viewer: text is held as `[]rune` (4 bytes per char) plus a cached string, and "Load more" re-concatenates everything (`files.go:59-86`). Editors cap at 2M runes and silently truncate (`theme.go:50`, `text.go:1705-1708`), so the "first 4 MiB" view can lose content. Diffs are stored three times (`diff.go:43-56`). Use a line-indexed, read-only byte store.
- Rich text costs about 36 bytes per character (`richtext/document.go:28-44`), each undo record is a full clone (`:202-216`), and each rich field is held as editor buffer, HTML source editor and original string (`detail.go:42-54`, `richtext.go:39-45`). Use span-based formats, delta undo, and create the source editor lazily.
- Use a typed card projection instead of `map[string]any` Rally objects (`internal/rally/types.go:10`). Divide the 24 MiB cache budget by window count, or host the cache in the broker.
- Fonts: each window process reads and keeps 7.5 MB SFNS plus 1 MB italic (`fonts.go:31-47`). Load lazily, and prefer a smaller UI face.
- The service host keeps the GUI runtime after the first window closes (`main.go:133-149`), which is consistent with the 261 MiB / 465 MiB post-close sample. Run the broker and app-server owner in a dedicated headless process.
- Under pressure, also back off background fetching and release GPU surfaces of hidden windows (V7). `lifecycle.go:83-125` does neither.
- Remove pasted `clipboard-*.png` temp files after send or on exit (`clipboard.go:17`).
- Keep `EditQueue` state during eviction (`lifecycle.go:113-119`). The in-progress edit of a queued message currently becomes a plain draft.

---

## 4. Persistence

- Stop re-serializing every second. The writer marshals the full snapshot to compare strings (`app.go:166-178`), `WriteJSON` re-indents already-encoded bytes and fsyncs (`settings/settings.go:106,120`), and rich drafts serialize at about 55 bytes per character as numeric arrays (`richtext/document.go:472-486`), so a 100K-character description is about 6 MB per write. Use dirty generations, compact encoding, and a span-based rich draft format.
- Don't persist virtual text documents (Changes, JSON views) in the session (`session.go:119-126`). Cap the snapshot size.
- Add a `schemaVersion` to `Preferences` and the session struct (`settings.go:28-46`, `session.go:16-23`) with migration functions.
- Consolidate atomic writes into one helper that fsyncs the directory after rename and retries Windows sharing violations, like `update.replace`. `Store.WriteJSON` (`settings.go:105-130`) and `saveRawConfig` (`configuration.go:296-312`) duplicate this, and several processes write `settings.json` concurrently.
- Persist window geometry (size, position, maximized, sidebar and info visibility per window). Nothing is saved today (`settings.go:28-46`), which contradicts I.distribution's "updates preserve … window state".
- Every preference toggle runs a full `MarshalIndent` plus fsync and broadcasts to all windows (`app.go:326-347`). Debounce it.
- `popout.go:402-408` applies preferences from other windows without rebuilding the italic and navigation fonts or running `Load()` validation, and without re-running `connectRally`/`loadScope` when Rally settings change. Run the same path as `theme()` plus validation.
- Include the Settings tab, in particular the unsaved raw TOML draft, in transfer and session snapshots (`popout.go:82-125`, `configuration.go:255-275`).
- Documents from a closed pop-out return only at the next startup (`session.go:48-78`). Offer them back to a live window immediately.
- Remove the dead `a.writes` channel (`app.go:163,258`).
- `loadSession` reads and parses on the UI thread before the first paint (`app.go:187-189`). Move it to a worker and show a skeleton.

---

## 5. Architecture and code quality

### Layering
- Split the `App` god struct (`internal/ui/app.go:25-98`, about 75 fields) into owned sub-structs (`transferState`, `mailboxState`, `rallyScope`, `persistence`, `chrome`) with narrow interfaces.
- Move thread-state logic out of `internal/ui`: `eventDecoded` (`chat.go:478-575`), `upsertItem` (`chat.go:576-649`, which never uses `a`) and `mergeTranscript` (`chat.go:999`). Create a nucular-free `internal/conversation` package with an `Apply(conv, event)` reducer and table tests; these paths currently have 0% coverage.
- Move pure Rally view logic into an `internal/rallyview` view-model that takes strings, not `*nucular.TextEditor`: `rallyQuery`/`rallySignature` (`rally.go:78-139`), `filtered()` (`rally.go:454-511`), paging and window merge (`stream.go:41-107`), and `detailView.changes()` (`detail.go:383-460`).
- Extract the cross-tab mailbox and reply-wait state machine (`xtab.go:122-231`) into `internal/xtab`.
- Separate DTOs from the domain. `workspace.Conversation` doubles as domain model, `session.json` schema (`session.go:22`) and transfer payload (`popout.go:62`), so renaming a field silently breaks restore and pop-out.
- Replace the broker's 250-line method switch (`broker.go:209-455`) with a `map[string]handler`.
- Move OS helpers into `platform`: `openPath` (`ui/files.go:23-46`) duplicates `platform.OpenURL` (`platform/platform.go:17-30`).
- Unify optimistic concurrency into one `rally.Client.UpdateIfUnchanged`. `saveDetail` checks `LastUpdateDate` only (`detail.go:491-497`), while `assistant.Apply` checks `LastUpdateDate` and `VersionId` (`assistant/tools.go:212-216`), and board moves do a third variant (`board.go:308-314`).
- Give each page in the Rally catalog its own behavior hook (default query, columns, modes) instead of one generic `Spec` (`catalog.go:6-34`); see §7.

### Concurrency and lifecycle
- Capture `a.client` on the UI thread before starting workers. `popout.go:462` (`claimTab`) and `app.go:253` read the field off-thread while the UI thread reassigns it (`session.go:210`, `chat.go:458`).
- `client.go:146-150` uses `break` inside `select`, which doesn't leave the read loop. Use `return` or a labelled break.
- `codex/path_unix.go:29` calls `os.Setenv("PATH", …)`, and every failed lookup spawns an interactive login shell (`-ilc`). Cache the result with `sync.OnceValue` and pass `cmd.Env` to children instead of mutating the process environment.
- `post()` blocks once the 512-slot update channel is full (`app.go:267-275`). Combined with delta coalescing, make it non-blocking for coalescible updates so that a stalled UI can't back-pressure the codex reader.
- Kill pop-out children whose transfer times out, and have pop-outs detect broker death (`popout.go:262-304`).

### Error handling and diagnostics
- Add logging. Nothing uses `log`/`slog`, and app-server stderr is discarded (`codex/client.go:126-128`). Write a size-capped log under `store.Dir/logs` and keep a ring buffer of app-server stderr to show when it dies or fails to start.
- Replace the single overwritten `a.toast` string (`app.go:277-281`) with a queue of typed notices (info/success/warning/error) with optional actions such as Retry or Open Settings.
- Report session write failures (`app.go:174-177`) and the final `a.store.Save` error on exit (`app.go:261`).
- Don't ignore decode errors: `codex.Decode` (`client.go:387-391`) and `json.Unmarshal(m.Params, &a.updateStatus)` (`chat.go:480`).
- `configRequirements/read` errors are swallowed (`client.go:309`), so `Speeds()` treats managed requirements as unrestricted and may offer tiers that policy forbids. Fail closed.
- `openArtifact` discards `Fields`/`States` errors (`detail.go:76-78`), silently losing read-only and allowed-value checks. Show a schema warning.
- `loadScope` overwrites good lists with nil on error and reports only the first of four errors (`app.go:452-469`). Keep previous values per list.
- `connectRally` reports keyring failures as "Connect your Rally endpoint…" (`app.go:391-398`). Show the actual keyring error.
- Rally 4xx bodies are replaced with `http.StatusText` (`rally/client.go:142-143`), discarding WSAPI `Errors`, and responses over 32 MiB are truncated and reported as "invalid JSON" (`:113`). Surface `Errors[]`/`Warnings[]` and the size limit explicitly.
- Add sentinel errors (`ErrServerStopped`, `ErrDisconnected`) and real JSON-RPC codes. `Reject` always uses −32601 (`client.go:222`) and the broker always −32000 (`broker.go:450`).
- Keep error values short and lowercase, and build display sentences in the UI (e.g. `client.go:73`, `detail.go:495`).
- The `last-update.json` error shows on every launch until an update succeeds (`main.go:87-92`; it is removed only at `install.go:136`). Clear it after it has been shown once, as `docs/UPDATES.md:16` says.

### Protocol typing
- Replace about 230 `map[string]any` uses and about 130 scattered JSON-RPC method literals in `internal/ui` with a typed `internal/codex/protocol` package: method constants and per-method param/result structs, ideally generated from the app-server schema.
- Define shared constants for the `fastrock/*` broker methods; today they are duplicated as literals in `broker.go`, `popout.go:305-448` and `app.go`.
- Type the stringly enums: conversation status (`workspace/state.go:52`), `update.Status.State` (`chat.go:481`, `main.go:89`), Rally view modes (`rally.go:238-362`, `assistant/tools.go:102`), plan operations (`tools.go:133`), Theme and BusyInput.
- Use `buildinfo.Version` for `initialize` (`client.go:93`, hard-coded `"1.0.0"`) and `X-RallyIntegrationVersion` (`rally/client.go:105`). Parse `SupportedVersion` instead of hard-coding `minor < 162` (`client.go:63`).
- Remove the unreachable `!c.IndependentSpeedModes` branch (`client.go:313,337`).
- Keep `fastrockSequence` in a broker frame type, not in the JSON-RPC `Message` envelope (`client.go:32`).
- Answer `currentTime/read` instead of rejecting it as unsupported (`approvals.go:78-82`); Fastrock opts into `experimentalApi`. Decline unsupported elicitation modes rather than returning a JSON-RPC error (`approvals.go:48-54`; `R src/approvals/request.rs:228-273`).

### Security hygiene (beyond §1)
- `process_windows.go:13-18` strips quotes but doesn't escape `%` or `^` for `cmd.exe`. It is safe only because the arguments are constants; document that or avoid `cmd.exe`. Replace magic `0x08000000` with `windows.CREATE_NO_WINDOW`.
- The `..` check runs on the raw ref (`rally/client.go:66`), so `%2e%2e` passes the path-prefix check. Unescape before checking.
- macOS builds are ad-hoc signed with `--deep` (`dev/package.go:68-72`), and Go-downloaded updates bypass Gatekeeper. Plan Developer ID signing and notarization, and drop `--deep`.
- Use `golang.org/x/sys/windows` instead of raw `syscall.NewLazyDLL` calls and magic constants (`update/platform_windows.go:16-21`).

### Testing
- Add fuzz targets: `richtext.Parse` with an HTML round-trip, `prepareTranscript` (Markdown), `Client.read` framing, `rally.Quote`/`And`, `update.extract` (zip) and `completeUTF8Page`.
- Cover the 0%-coverage critical paths after extraction: `consume`/`eventDecoded`/`startTurn`/`resumeThread`, `xtab.go`, `transferEvent`/`claimTab`, `serverRequest`/`answerApproval`, `update.Apply`/`copyTree`. Overall `internal/ui` coverage is 18.6%.
- Broker tests: unowned-approval routing, ticket expiry, the "too many pending requests" path, slow peers, and oversize frames.
- `platform_windows_test.go:45-60` writes the real user's Start menu shortcut. Inject the known-folder path.
- Extend automation (Windows only today, `automation.go:24-245`) with approval, injected-request, file/diff, forward, cross-tab, key, pointer and server steps, like `R src/automation.rs:60-130`, so that V11 parity can be fixture-tested.
- Name tests by behavior, not spec IDs (`TestV13…`, `TestV15…`), and put the spec IDs in comments.
- Add regression benchmarks for the per-frame paths in §2 (info panel at 100k chats, New tab, transcript at 3000 blocks, 1 MB file view) with allocation assertions.

### Build and CI
- Add a `CGO_ENABLED=1 go test -race` job (Linux or macOS). Nothing runs `-race` today: neither `go.yml` nor the Makefile.
- Add `staticcheck`, `govulncheck` and `go mod tidy -diff` steps, built with `GOTOOLCHAIN=go1.27.2` so they can read 1.27 export data; the locally installed copies can't.
- `release.yml` skips `go vet` and the `third_party/nucular` tests. Make it run `make check`. Pin actions by commit SHA and add build-provenance attestation.
- Add darwin/amd64 to the CI matrix; today it is built only at release.
- Stamp versions in local `make` builds, and use placeholders rather than whole-file string replacement in `Info.plist` (`dev/package.go:57`).
- Keep a machine-readable patch series for the nucular fork (`third_party/nucular.patches/` against upstream `58b808aa…`) with a CI check that it regenerates.

### Code hygiene
- Replace `index`/`contains`/`remove` (`rally.go:673-697`) with `slices.Index`/`Contains`/`DeleteFunc`. Note that `index` returns 0 for missing values, which causes the font-size and picker bugs in §1 and §7. Return -1 and handle it.
- Sort numeric and date columns numerically (`rally.go:500-507`); "10" currently sorts before "9". Skip the local sort when the server already sorted.
- Name the magic numbers: card stride 206 (`stream.go:73,94`, `board.go:185`), scattered 45/30/20/15/3 s and 2 min timeouts, cache limits 32 / 24 MiB / 45 s / 4 (`cache.go`), and the xtab limits 8192/4/8/64/20000/1800.
- IDs from `UnixNano` can collide (`workspace/state.go:54`, `chat.go:375`, `xtab.go:125,129`). Use a counter or `crypto/rand`.
- Remove dead code: `var _ json.RawMessage` (`approvals.go:195`), `_ = conf` plus a duplicate layer parse on the UI thread (`settings_extra.go:400-416`), the duplicated `len(all) > 100000` check (`rally/client.go:227-236`), the `lower()` wrapper (`rally/client.go:23`), the never-read `v.Rules` (`rally.go:40,343`) and the leftover `Page`/`PageSize` list paging fields.
- `CanonicalKind` allocates a map and slice on every call (`rally/types.go:144-149`). Make the table package-level.
- `moveBoardCard` sets `a.status = "Connected"` after every move (`board.go:305-325`), overwriting the real status ("codex-cli x · Connected"). Restore the previous status, or don't touch it.
- `workspace.State.Open` panics if `crypto/rand` fails (`state.go:158-161`). Fall back to a counter.

---

## 6. Codex GUI parity (`codex-rs/gui`)

### Theme, typography and geometry
- Add the missing tokens `border-strong`, `accent-soft`, `*-soft` and `diff-*` (`R ui/theme.slint:28,38-51`) to `palette` (`internal/ui/theme.go:14-23`). The composer card, popups, user bubbles, selected choices and diffs need them.
- Add the font scale: small (−2), large (+3) and title (+7) (`R ui/theme.slint:54-57`). `title()` currently draws headings at body size (`theme.go:86-89`).
- Load a monospace face (Menlo/Consolas/fontconfig monospace, code at base −1; `R src/app.rs:1765`) and use it for code blocks, commands, diffs, terminal output and the file viewer. Today everything is proportional (`fonts.go:14-49`).
- Load real bold (600/700) instead of the 1px overstrike that is only used in the transcript (`transcript.go:259-263`).
- Buttons: make them at least 26 px tall with 12 px horizontal padding (`R ui/theme.slint:83-99`). Give `primary()` and `button()` hover, pressed and disabled colors; they override only the normal state, so primary buttons turn gray with dark text on hover (`theme.go:66-85`). Add a danger variant.
- Add System to the theme options (System/Light/Dark, following OS changes; `R src/prefs.rs:84`, `src/app.rs:555-565,1544`). `settings.go:96-98` and `app.go:352-354` coerce anything that isn't light to dark.
- Match the defaults: busy input = Steer (`R src/prefs.rs:86`; Fastrock "queue", `settings.go:50`); font range 8–32.
- Make the sidebar 260 + 1 px with a surface-alt background and the info pane 280 + 1 px, with divider lines (`R ui/app.slint:865-866,1044-1065`). Today both are 255 px with no dividers (`app.go:527-545`).
- On narrow windows, show the panes as drawers over the content with a dimmed backdrop (`R ui/app.slint:871-883,1069-1096`). Today fixed widths overflow below about 870 px.
- Implement DPI awareness: `ebitenscreen/screen.go:148-159` uses `PixelsPerPt:1`, `Style.Scaling` is never set, `LayoutAvailable*` values passed back to `Row()` are scaled twice (`app.go:526,543`), and the board stride doesn't scale (`board.go:185`).

### Tab strip, menus, shortcuts, dialogs and toasts
- Tab status dot: 8 px; running = accent, waiting (pending approval) = warning, error = danger, starting = faint, unread = success, idle-and-open = hollow border-strong ring (`R ui/app.slint:258-271`, `src/app.rs:1021-1033`). Fastrock has a 6 px faint/accent/danger dot (`app.go:645-654`, `widgets.go:113`).
- Track unread: mark background tabs unread on item/turn completion or error and clear on activation (`R src/app.rs:757-759,971-976,1714`).
- Strip layout: surface-sunken background, "+" directly after the last tab, a spacer, then text links "Info" (accent when shown) and "Settings" (`R ui/app.slint:396,547-568`). Fastrock pins "+" to the right, uses an icon for info and a bordered Settings button (`app.go:601-602,734-743`).
- Add the overflow chevron that lists every tab with its dot (`R ui/app.slint:326-390,533-599`). Wheel scrolling is the only option today.
- Drag-reorder feedback: accent border on the dragged tab and a 2 px insertion line, with the slot computed from the pointer (`R ui/app.slint:145-148,505-530`). Fastrock only reorders when released over another tab, with no marker (`app.go:657-672`).
- Tab context menu in the reference order and labels: Rename…, Fork, Side chat, Continue in worktree, Recap conversation, Compact context, Review changes…, Create AGENTS.md, Export as Markdown…, View as Text, Copy thread id, Archive, separator, Close other tabs, Close (`R ui/app.slint:150-209`). Put Pop out and Move after a separator so they don't lead the menu (`app.go:674-694`, `interactions.go:87-89`).
- "+" should reuse an existing New tab (`R src/app.rs:1046-1056`). New tabs should open right after the active tab (`:904-927`), and a thread started from the New tab should replace it (`:930-940`). Closing the active tab should activate the right neighbour (`:1143`; Fastrock picks the left, `workspace/state.go:175`).
- Menu bar: Title Case with separators (File: separator before Settings… and before Quit), "Toggle Info Pane", "Export as Markdown…", and Help › About as an overlay with the version (`R ui/app.slint:923-1014`). Fastrock uses sentence case without separators, and About opens Settings (`menu.go:13-16,47-48`).
- Shortcut names: "Open settings", "Toggle info pane", "Interrupt running turn", "Go to tab N", "Go to last tab" (`R src/shortcuts.rs:48-69`; Fastrock `interactions.go:28-31`).
- Toast: a bottom-centre pill that hides after 2.2 s (`R ui/app.slint:1124-1137`, `src/app.rs:67,1396`). Fastrock's toast is a permanent amber row under the tabs that shifts the layout (`app.go:515-521`).
- Add the warnings banner for `configWarning`, thread-less `warning` and `deprecationNotice` (`R ui/app.slint:602-627`, `src/app.rs:761-774`). These are dropped today because events without a chat return early (`chat.go:508-513`).
- Dialogs: centred at one-third height with a dimmed backdrop and 520 px max width; Cancel on the left, the specific action on the right ("Rename", "Delete", "Sign out"), red when destructive; Enter accepts and Esc cancels (`R ui/app.slint:685-761`, `src/app.rs:269-297`). Fastrock's dialogs are movable windows at a fixed (360,240), with a generic primary "Save"/"Confirm" on the left and no Enter/Esc (`actions.go:25-53`).
- Show the link-destination tooltip on hover (`R ui/app.slint:1139-1159`).

### Sidebar
- Header: "Threads" / "Archived threads" in large 600 weight, links "Archived" / "Show active", and a ↻ refresh disabled while loading (`R ui/sidebar.slint:268-297`). Fastrock shows "CONVERSATIONS", a "+", Recent/Archived rows and a bottom "Refresh history" row (`chat.go:229-246,297-304`).
- Search: placeholder "Search threads", 250 ms debounce, *server-side* search plus history search with "Searching conversation history…" (`R src/sidebar.rs:56,640-664`). Fastrock filters only the loaded titles locally (`state.go:206-210`).
- Paging: load more automatically within 120 px of the end, with a "Load more threads" / "Loading…" row (`R ui/sidebar.slint:233-249,340-345`). Fastrock needs a button click (`chat.go:289-296`).
- States: "Loading threads…", "No threads yet. Start one from the new tab page.", "No archived threads", "No threads match your search", and inline red errors with Retry (`R ui/sidebar.slint:348-368`). Fastrock always shows "No conversations" and toasts errors (`chat.go:57-61,286-288`).
- Folder rows: 30 px, ▸/▾ chevron, bold title, thread count, always-visible "+" (tooltip "New conversation in this folder"), and the full path as a tooltip so same-named folders can be told apart (`R ui/sidebar.slint:69-149`; Fastrock `widgets.go:197-216`).
- Folder menu: "New thread in this folder", "Open folder", "Copy path", separator, Expand/Collapse (`R ui/sidebar.slint:74-93`).
- Thread rows: 22 px indent (Fastrock 12, which is less than the folder title), status dot including the open-idle ring, right-aligned relative time (now/5m/3h/2d/4w/5mo/2y) and a title tooltip (`R ui/sidebar.slint:151-231`, `src/sidebar.rs:240-314`). `flatRow` never draws its `detail` argument (`widgets.go:85-108`).
- Thread menu: Open, New thread in this folder, separator, Rename…, Archive/Unarchive, Delete…, separator, Copy thread id (`R ui/sidebar.slint:156-184`). Fastrock shows all 12 tab actions (`interactions.go:217-237`). Export and View as text on an unopened thread produce empty output (`actions.go:81-85`); load the history first.
- Wording: "Rename thread" / "Rename"; "Delete thread?" with "cannot be undone" and a red Delete; toast "Archived “title”" / "Restored …"; confirm "Archive thread?" from the tab menu (`R src/sidebar.rs:795-930`, `src/threads.rs:1033-1043`). Use "thread" consistently instead of mixing conversation and chat.
- Titles: "(no message yet)" for untitled threads, 80-character truncation, and tab titles from name → first preview line (48 chars) → folder (`R src/sidebar.rs:59-94`, `src/app.rs:161-171`). Fastrock falls back to the raw thread id and "New conversation" (`chat.go:85-92,127,343-351`).
- Refresh should replace the top of the list, so threads deleted or archived elsewhere disappear (`R src/sidebar.rs:191-222`). Fastrock only adds or updates (`chat.go:66-80`).

### New tab
- A centred 600 px scrolling column titled "Start a new thread" with the subtitle "Choose a folder or start a conversation without a project folder." (`R ui/newtab.slint:110-137`). The Fastrock page can't scroll because the document group is `WindowNoScrollbar` (`app.go:550`), so lower rows are unreachable.
- Buttons: "New conversation" (folderless, home dir), primary "Choose folder…" that starts the thread right after the trust check, "Open file…", and "Resume a thread" that focuses sidebar search (`R ui/newtab.slint:175-196`, `src/newtab.rs:648-718`). In Fastrock, Choose folder only fills a field, "Folderless chat" sends no cwd (`chat.go:116-118`), and Resume opens an unsearchable popup.
- Show the sign-in card only when sign-in is needed (`R ui/newtab.slint:139-173`); Fastrock always shows "Sign in to Codex…" (`app.go:790-793`).
- Recent folders: a bordered card with 52 px rows (bold name, `~`-shortened path, "N threads"), merging recents with thread folders; menu "New thread here / Open folder / Copy path / Remove from recent"; empty text "Folders you work in will appear here." (`R ui/newtab.slint:35-229`, `src/newtab.rs:105-135`).
- Add the folder trust check before creating a thread, with trust or restricted options (`R src/newtab.rs:1-8,720`).
- Move the Rally shortcuts and copy ("What would you like to work on?") off the Codex new-tab page, or into a clearly separate secondary section, so the page matches the reference.

### Composer
- Input frame: a rounded card with a border-strong edge that turns accent on focus or drop, growing from one line to 220 px (`R ui/composer.slint:95-103,628-663`). Fastrock uses a fixed 76 px frameless editor (`chat.go:839-840`).
- State-aware placeholder, for example "Ask Codex anything. @ to mention files, / for commands" and "Working… type to steer, Esc to stop" (`R src/composer/mod.rs:938-962`).
- Toolbar pickers that open above the input with a heading, ✓ marks and a description per row: Model (hide hidden models; add a "Current model" row when it isn't in the catalog), Effort (Low/Medium/High/Extra high), Speed, **Permissions** (Read only / Auto / Auto-review / Full access), and a Plan pill (`R ui/composer.slint:242-424,720-776`, `src/composer/presets.rs:49-172`). Fastrock uses plain combos with raw IDs and no Permissions picker, and shows the first model when the current one is missing (`chat.go:841-892`).
- Send/Stop: one primary button labelled Send, Steer or Queue by preference, disabled when empty, plus a red Stop (`R ui/composer.slint:812-826`). Remove the inert "Ready" label (`chat.go:893-899,913-938`), which violates V11.
- Context indicator: "NN% context left" (`R src/composer/mod.rs:841-848`) instead of "model · N tokens", which is the cumulative thread total (`chat.go:914`).
- Attachments: 40 px chips above the input with thumbnail, size and ×; accept only PNG/JPEG/GIF/WebP and check model image support (`R ui/composer.slint:426-496`, `src/composer/mod.rs:399-408`). Fastrock's "+" attaches any file as `localImage` (`actions.go:66-68`, `chat.go:370-373`).
- Drag-and-drop with the "Drop to attach images or insert file paths" overlay (`R ui/composer.slint:830-847`, `src/composer/drop.rs`).
- Editor context menu: Cut, Copy, Paste (image-aware), Select All (`R ui/composer.slint:125-157`).
- Suggestion popup: floating, at most 640 px wide and 8 × 28 px rows, with a title and the hint "↑↓ to move · Enter to choose · Esc to close"; wrapping arrows, hover-select, and slash rows with descriptions and highlighted matches (`R ui/composer.slint:498-626,851-856`). Fastrock's inline buttons push the layout (`suggestions.go:131-137`). Quote inserted paths that contain spaces (`suggestions.go:55`).
- Slash matching: case-insensitive fuzzy, plus the aliases approvals, clear, btw, usage, config, apps, features, login and exit (`R src/composer/slash.rs:292-372`). Fastrock uses a case-sensitive prefix (`suggestions.go:72-78`).
- Slash semantics: `/theme` opens Appearance, `/model` and `/permissions` open the toolbar pickers, `/status` reveals the info pane, `/stop` stops background terminals after confirmation, `/resume` focuses sidebar search, and `/copy` with no reply toasts (`R src/composer/mod.rs:556-648`). `/ps` reveals and pins the terminals section instead of dumping JSON (`slash.go:58-59`).
- `!command` runs a shell command after a one-time confirmation (`R src/composer/shell.rs:1-60`).
- Local echo: show the user bubble immediately (muted while pending, red with a caption on failure). Queue on the server with the toast "Queued; sends when the current turn ends", and list queued items in the info pane (`R ui/transcript.slint:685-762`, `src/threads.rs:835-860`).

### Transcript
- Remove the title/status row (raw "idle"/"running") and the toolbar (Top/Latest/Expand all/Collapse all/Copy last reply), and centre content in an 860 px column (`R ui/transcript.slint:111-113,1440`; Fastrock `chat.go:657-687`).
- User messages: right-aligned accent-soft bubbles (radius 12, at most 82% wide) with image chips. Assistant text: no "ASSISTANT" label (`R ui/transcript.slint:685-762`; Fastrock `chat.go:581-594,743` shows `[Image: path]`).
- Per-message actions under finished messages: Copy, Quote (insert at caret), Send to tab…, Select text… (`R ui/transcript.slint:1551-1580`). Today they are only in a context menu, and Quote appends at the end (`transcript.go:322-366`).
- Reasoning: "Thinking" (spinner) or "Thought" with an italic summary and chevron (`R ui/transcript.slint:764-801`), not "reasoning · first 80 chars" (`chat.go:738-741`).
- Command cards: status icon (spinner/✓/✕/⊘/■), "Ran"/"Running", command in monospace, exit code and duration, and a monospace output box with Copy. Compact "Explored" rows for read/list/search (`R ui/transcript.slint:167-234,803-902`). Don't parse command output as Markdown (`chat.go:606-608`); `#`, `*`, `_` and `<x>` currently get mangled.
- File changes: summary "Edited N files +a −r" plus a card per file (clickable path, counts, "Open diff", coloured lines) (`R ui/transcript.slint:904-1041`), not indented JSON rendered as Markdown (`chat.go:609-612`). "Open changes" should open the *turn* diff from the server and toast "This thread has no changes yet" (`R src/info.rs:656,841-853`), not `git diff HEAD` of the whole repo (`files.go:310-330`).
- MCP and web-search cards ("Called …", "Searched the web: …") (`R ui/transcript.slint:1043-1132`) instead of raw item JSON (`chat.go:634-637`).
- Render the remaining item types: plan, context compaction, review mode, image view, image generation, hooks and sub-agent activity (`R src/transcript/render.rs:301,512-694`). Fastrock shows them as empty assistant blocks (`chat.go:578-580,648`) and ignores `turn/plan/updated`.
- Show errors inline as coloured notice rows with Copy (`R ui/transcript.slint:1196-1257`), not only as a toast.
- Markdown:
  - Code blocks: bordered box, language label (or "code"), Copy button, monospace (`R :567-623`).
  - Headings at +7/+4/+2.
  - Tables with borders, a shaded header, measured widths and alignment (`R :625-666`). Fastrock splits on tabs into equal columns (`transcript.go:119-131`).
  - Block quotes with a 3 px bar instead of `│ `.
  - Nested lists indented 20 px per level, numbered lists keeping their start numbers (they restart at 1 today, `transcript.go:165-183`), task-list checkboxes and horizontal rules.
  - Escape stray HTML so `Vec<String>` stays visible (`R src/transcript/markdown.rs:9`). Fastrock drops inline HTML (`transcript.go:103`).
- Directives and links: rewrite `::code-comment`, `:codex-file-citation` and follow-up directives; link bare `src/x.rs:42` paths and `mailto:` (`R src/transcript/directives.rs`, `links.rs`). Fastrock prints directives literally and refuses `mailto:` (`actions.go:69-74`).
- Auto-follow: turn it back on whenever the user scrolls back to the bottom (`R ui/transcript.slint:1769-1774`). Fastrock disables it on any upward scroll or click in text (`chat.go:705-707`, `transcript.go:228-233`).
- History: a floating "Load earlier messages" pill that pages from the server (with loading and retry), and a "Jump to latest" pill only when content overflows (`R ui/transcript.slint:1590-1621,1783-1803`). Fastrock's "Load older messages" only reveals blocks already in memory (`chat.go:712-718`); that is a V11 violation.
- Empty and running states: "What should we work on?" with "Codex is working in <folder>…" and three suggestion buttons; a spinner while loading or starting; "This thread could not be opened"; and an "Esc to interrupt" status line while running (`R ui/transcript.slint:1623-1705,1805-1830`).
- Export: suggest `<title>.md` in the thread folder with a Markdown filter; write "# Codex conversation" with `## User` / `## Activity` / `## Assistant` from the *full* history (`R src/threads.rs:1076-1116`, `src/transcript/export.rs`). Fastrock writes `## you` / `## tool` / `## changes` with raw JSON from loaded blocks only (`actions.go:81-95`).

### Thread actions
- Side chat opens immediately with no dialog, is titled "Side chat — <parent>", adds a "Side chat branched from …" notice, refuses nested side chats, and takes an optional first message (`R src/threads/side.rs:32,155-200`; Fastrock `side.go:12-40`).
- Recap runs in a temporary tool-free thread and shows a card, without posting into the main thread (`R src/threads/recap.rs`). Fastrock posts "Summarize…" and spends a full agent turn (`interactions.go:151-152`).
- Create AGENTS.md sends the bundled `prompt_for_init_command.md` (`R src/threads.rs:71,1024`), not a one-line prompt.
- Review offers a picker: uncommitted changes / base branch / commit / custom (`R src/threads/review.rs`). Fastrock offers only free text (`interactions.go:155-162`).
- Worktree creates a managed worktree with no folder prompt (`R src/threads/worktree.rs`). Fastrock asks for a parent folder and creates a detached `fastrock-<unix>` worktree (`files.go:331-363`).

### Approvals and elicitation
- Card layout (`R ui/approvals.slint:349-630`): status dot, wrapped large title, reason, a selectable monospace code block (up to 150 px) with Copy, detail lines, a diff box (up to 240 px), and option rows with a focus border. Fastrock uses a fixed 140 px group with a one-line button title, the reason squeezed into 35 px, and choices that overflow the 190 px reserved (`approvals.go:96-117`, `chat.go:689-691`).
- Command approval: "Run this command?", `$ <command>` with shell wrappers removed (`bash -lc`, `pwsh -Command`; `R src/approvals/format.rs:42-62`), "in ~/path", and Environment / Permission-rule lines (`R request.rs:667-711`). Special titles: "Allow network access to {host}?", "Send input to the running command?", "Run this command with additional permissions?" (`:668-676`).
- File-change approval: render the proposed diff (cached from `item/started` and patch updates) under "Edited path (+a −r)", capped at 400 lines, or "The proposed changes are not available yet." (`R request.rs:861-889`, `format.rs:465-510`, `mod.rs:883-921`). Today the user approves blind, and the title ends in a bare colon when there's no reason (`approvals.go:20`).
- Permissions approval: "Permission rule: network; write `~/x`", cwd and environment (`R request.rs:949-972`).
- Option labels that show what gets saved: "Yes, proceed", "Yes, and don't ask again for commands that start with `prefix`", "No, continue without running it" (`R request.rs:571-631,830-857,918-947`). Hide save-prefix when the prefix spans several lines. Offer only network options on network approvals (`approvals.go:224-236` adds the command-rule option too).
- Keyboard: Up/Down highlight, Enter to choose, Esc for the cancel option, uppercase key chips coloured by tone, and a hint line (`R approvals.slint:106-127,392-410,622-628`, `mod.rs:1157-1235`). Fastrock requires clicking the title first and lets Esc fire only `n` (`approvals.go:293-317`).
- Badges: "1 of 3" and sub-agent origin (`R approvals.slint:331-347`).
- Attention: mark the tab "waiting" and send a desktop notification when the window is unfocused (`R mod.rs:784-792`).
- After answering, add a transcript notice such as "You approved Codex to run `ls` this time" (`R request.rs:743-827`).
- Remove "View full request details" raw JSON (`approvals.go:104-109`) in favour of structured detail lines.
- User-input card: "Codex has a question" / "Codex has N questions"; Submit / Skip with the "Unanswered questions are sent without an answer…" hint; placeholders "Type your answer", "Add a note (optional)", "Describe what you want instead"; header and wrapped question shown separately (`R src/approvals/user_input.rs:102-270`). Fastrock prints "header: question" on one clipped line (`elicitation.go:84-88`).
- MCP tool-call approvals: "Allow {server} to run {tool}?", up to 6 parameters, Allow / Allow for this session / Always allow (from `_meta`) / Cancel on Esc; tool suggestions "Install X?" (`R elicitation.rs:161-292,548-590`). Fastrock shows an empty form, ignores `_meta`, and sends decline instead of cancel (`approvals.go:47-77,139-181`).
- Elicitation forms: title "{server} needs some information"; Submit / Decline / Cancel; per-field red errors plus "Fix the highlighted fields to continue."; bound hints like "Whole number from 1 to 10" (`R elicitation.rs:328-434,886-929`). Boolean fields print the header twice and lack `*` (`elicitation.go:88-92`).
- URL elicitation: a single "Open the link and continue" choice that opens *and* accepts, plus Decline and Cancel; title "{server} wants you to open a link"; non-http links shown as text only (`R elicitation.rs:233-326`). Fastrock's "Allow / Submit" accepts without opening anything (`approvals.go:119-127,152-165`).
- Warn when closing a tab that has a pending request ("A request in this tab is waiting for your answer…", `R mod.rs:142-170`).

### Info panel
- Add the missing sections: Context meter (percent of window from the last turn, 12K baseline, "X of Y tokens in context", plus thread total in/cached/out), Plan, Changes file list, Queued messages, Running hooks, MCP servers, Usage limits (`R ui/info.slint:392-905`, `src/info.rs:102,676-766,1922-1971`). Rate limits exist only as raw JSON in Settings › Account (`settings_extra.go:107-109`).
- Summary rows: Model / Effort / Provider / Approval ("On request · auto-review") / Sandbox ("Workspace write + network") / Side chat (`R src/info.rs:1621-1790`), instead of muted status/model/effort lines (`chat.go:948-957`).
- Remove Rename/Fork/Archive/Export/"← Move / Move →" from the pane (`chat.go:958-995`), along with the unbounded "Other conversations" list (§2).
- Make every section collapsible with a summary, share collapse state across tabs, and hide empty sections (`R ui/info.slint:180-215`).
- Goal: an empty-state sentence plus "Set goal…", or Edit… / Pause⇄Resume / Clear; one pre-filled prompt; labelled statuses and "1.2M tokens · 2h 5m" (`R ui/info.slint:597-620`, `src/info.rs:1288-1298,1837-1860`). Fastrock prints raw `budgetLimited` and `1.234567e+06` (`info.go:53-85`). Hide the section when the goals feature is off instead of toasting an error on every load.
- Terminals: refresh on process-item and terminal events, not only while the list is non-empty (`info.go:182-184`), so that new terminals appear. "Stop all" should call `backgroundTerminals/clean` after a dialog listing the commands, with a destructive button (`R src/info/terminals.rs:150-168,453-520`). Use relative or `~` folders, inline actions, and Loading / not-available states. Replace the "View details" JSON.
- "Sub-agents" rows: status dot, detail line and status text (`R ui/info.slint:684-735`), not "name · rawstatus" buttons (`info.go:247-268`).
- Cross-tab: a "Let agents in other tabs message this thread" toggle (not the inverted "Disable agent messages"), and "From/To <tab>" rows with times that open the peer, instead of the "Read mailbox" JSON (`R ui/info.slint:805-870`; Fastrock `info.go:89-96`). Turning messaging off should decline pending consents (`R src/xtab/mod.rs:435-458`).

### Files and diffs
- Diff view: old/new line-number gutters, +/− markers, tinted backgrounds, **intraline emphasis**, and status badges (added/deleted/renamed/binary) with +N −N and a summary like "2 files changed · +2 −1", plus Copy diff (`R ui/files_diff.slint:52-178,279-316`, `src/files/diff_view.rs:234-346`). Fastrock colours text by first character, so `+++`/`---` headers get coloured too (`diff.go:57-75`). It also nests each file in a 500 px scroll group (`diff.go:110`). Keep it unified-only; the reference has no side-by-side view.
- Toolbars: show Copy all / Save as… for in-memory text and Expand / Collapse / Copy diff for diffs (`R ui/files_viewer.slint:438-487`). Today Copy path / Reveal / Open externally / Reload act on the document *title* as a path (`files.go:237-270`), and Find / Go to / Wrap act on a hidden editor.
- Find: "3 of 12" / "No results" in red, highlight every visible match, Enter / Shift+Enter / Esc in the field, F3 / Shift+F3, and Ctrl+F focuses the query (`R ui/files_viewer.slint:79-201`). Ctrl+G should be Go to line, not find next (`actions.go:248-257`).
- Go to line: placeholder "Line number (1–N)", a "Go" button, select and centre the line, and toast out-of-range input (`R src/files/viewer.rs:1247-1325`). Fastrock pre-fills "1", uses "Save" and silently jumps to the end (`files.go:241-250`).
- Line-number gutter and a status bar with "12 KB · 340 lines" and "Ln X, Col Y", plus a precise truncation message (`R ui/files_viewer.slint:282-300,590-605`).
- Non-UTF-8 files: offer Open externally / Reveal / "Show as text" (lossy decode) / Retry (`R ui/files_viewer.slint:508-545`). Fastrock dead-ends with "file is not UTF-8 text" (`files.go:157-170,294-297`), so common Windows-1252 files can't be viewed.
- Watch the open file and show "This file was deleted or moved. Showing the last loaded version." and change banners (`R src/files/viewer.rs:285-300,992`).
- Save as: suggest `<title>.md` with Markdown/Text filters and toast "Saved" (`R viewer.rs:1130-1155`).

### Settings (shared)
- Navigation: grouped page headings, open on **Common**, and show Windows sandbox only on Windows (`R ui/settings.slint:72-158`). Fastrock's list is flat, opens on Appearance and always shows Sandbox (`settings.go:66`).
- A subtitle on every page (`R ui/settings_config.slint:52-55`).
- Server banners: "Codex is starting…", "Codex is not running…" with "Edit config.toml", and a restart-needed banner (`R settings_config.slint:12-46`).
- Per-row "Saving…" and red inline errors (`R ui/settings_widgets.slint:415-419,579-583`) instead of a "Saved" toast after every request, errors included (`settings_extra.go:37-45`).
- Remove raw JSON: "View data", the *editable* JSON dump on empty pages and the "Details" dumps (`settings_extra.go:79-81,275-290`). Show empty-state text (`R ui/settings_extensions.slint:219-227`).
- Clear page rows on navigation (`settings.go:68-72`). Feedback currently shows the previous page's toggles under its form.
- Queue config writes and take the expected version from each response, retrying once (`R src/settings/mod.rs:1144-1268`). Fastrock hits false `configVersionConflict` (`configuration.go:115-130`).
- Handle live notifications (sign-in, account, rate limits, MCP status, skills, import, sandbox; `R src/settings/mod.rs:771-807`) so pages update after background work.

### Settings (per page)
- **Common**: add the page (model, effort, context limit, approvals, sandbox, search, summaries, verbosity, notify, filtered by org requirements; `R src/settings/fields.rs:582-780`). Fastrock's "Models" is a read-only catalog with a hard-coded warning (`settings.go:182-209`).
- **All settings**:
  - Show help inline with source badges (`R ui/settings_widgets.slint:385-511`) instead of an "Info" popup and raw layer names (`configuration.go:189-197`).
  - Lock by precedence and requirements and show the reason (`R src/settings/model.rs:193-311`); today profile layers aren't locked (`configuration.go:58`).
  - Save dropdowns consistently, and have empty text remove the key instead of writing `""` (`configuration.go:209-240`).
  - Use a TOML snippet editor for tables and a shell-words editor for `notify` instead of flattened JSON (`configuration.go:79-114`).
  - Use a separate search box from Plugins, group headers, a "No settings match" state, and a project dropdown of open and recent folders (`R src/settings/mod.rs:910-960`).
- **config.toml**: dirty state, a "Discard?" prompt on navigation or Reload, an "Overwrite?" dialog on conflict, and don't force mode `0600` (`configuration.go:255-338` vs `R src/settings/raw.rs:325-640`).
- **Account**: a summary card instead of a JSON dump (`R ui/settings_account.slint:45-84`), respect the allowed login methods, show usage limits as a card, and offer a restart after a provider change.
- **MCP**: rows with status, source and a command subtitle, and an inline Remove (`settings_extra.go:242-252`); structured args/env/headers editors instead of JSON fields.
- **Features**: stage grouping and a "Changed" badge.
- **Plugins / Hooks / Skills**: confirm Uninstall, show status badges and marketplace headers, show hook commands, hide Trust for already-trusted hooks, group skills by scope, and add load-errors and precedence notes (`R src/settings/extensions.rs:130-407`).
- **Memories**: three switches with lock state, a "cannot be undone" reset confirmation, and Learn more pointing to `/codex/memories` (`R src/settings/memories.rs:28,63-170`); not a JSON dump plus Reset (`settings_extra.go:141-148`).
- **Appearance**: System/Light/Dark, the full size range, and switches for sidebar and info pane.
- **Keyboard**:
  - Display shortcuts as "Ctrl+Shift+T" / "⌘⇧T", not `key.Modifiers(Control) + CodeT` (`settings_extra.go:432-435`).
  - Reject bare letters and text-editing combos.
  - Confirm "Reset all", offer an inline "Reassign?" on conflict, and show the action name instead of its ID while recording (`R src/settings/keyboard.rs:34-559`).
- **Import / Feedback / Sandbox / Diagnostics**:
  - Import: grouped sources, progress and results, and Import disabled when nothing is selected.
  - Feedback: categories, validation and a result card (it always sends "bug" today).
  - Sandbox: a structured card instead of JSON plus bare buttons.
  - Diagnostics: honour the "Configuration key" field, which is currently ignored (`settings_extra.go:150-193,185-190`).

### Bedrock and local providers
- Merge them into one page with a "Current provider" card and a "Restart now" note (`R ui/bedrock.slint:162-244`; Fastrock `bedrock_settings.go:17-82`, `providers.go:221-271`).
- Region: a dropdown of the 14 regions with a GovCloud warning (`R src/settings/bedrock/flow.rs:41-105`), not free text. Drive credential methods from what discovery found.
- Validate inputs and resolve the profile on "Check inputs" (`R flow.rs:406-522`); it only tests for empty fields today.
- Apply: a busy guard against double setup; don't call Mantle setup in Runtime mode; clear the old `model`; run the GovCloud check automatically; offer "Restart now / Later" (`R src/settings/bedrock/mod.rs:1122-1258`).
- Add the default-model dropdown, the endpoint description and the "Use Amazon Bedrock" label.
- Local servers: show both servers with Running/Not running, list models with "In use"/"Use", block Ollama older than 0.13.4, show aggregate pull progress, show Pull/Cancel only when relevant, and give human-readable errors (`R src/settings/local.rs:190-247`; Fastrock `providers.go:118-270`).

### Cross-tab messaging
- Show the consent card in the *target* tab: "Agent in tab “X” wants to send this message", a permissions warning, the message in a code block, a caption saying whether the sender waits, and Deliver / "Deliver, and allow “A” → “B” for this session" / Decline (Esc), plus an "Always asks:" reason when the target has more permissions (`R src/approvals/delivery.rs:46-108`, `src/xtab/policy.rs:22-66`). Fastrock shows a modal in whatever window is active with a raw thread ID (`xtab.go:105-121`).
- Match the limits: 5 sends per turn, 10 undelivered per target, 3 unattended hops, 1 h TTL, plus an escalation check (`R src/xtab/limits.rs:13-20`). Reject calls from tabs with messaging off or threads not open in a tab (`R src/xtab/mod.rs:243-261`).
- Use the reference envelope with `from_title` and `reply_expected` (`R src/xtab/tools.rs:357-370`), and render it as a "Message from tab X" card instead of raw XML (`R src/transcript/crosstab.rs:98-110`).
- Persist the mailbox (500 entries in `CODEX_HOME/gui/mailbox.jsonl`, `R src/xtab/mailbox.rs:26-33`); Fastrock keeps 50 in memory (`popout.go:316-317`). Include queued-note and title fields in tool results, and return a note for an empty reply.

### Startup, connection and notifications
- Open the window even when Codex fails, and show an overlay ("Starting Codex…" / "Codex could not start") with the stderr tail, Retry and a link to Raw configuration (`R ui/app.slint:637-683`, `src/startup.rs:420-428`). Fastrock exits through a native alert (`cmd/fastrock/main.go:100-130`), and a bad config.toml is reported as "installed Codex app-server is incompatible; update Codex CLI" (`codex/client.go:96-99`).
- On disconnect, show an in-window banner with a Restart button (`broker.go:511`, `popout.go:318-336`), not just a toast.
- Desktop notifications for approvals and finished turns while unfocused, behind a preference (`R src/notify.rs:38-63`).

---

## 7. Rally parity (`ignore/index.html`)

### Navigation and shell
- Collapse the Rally chrome into one 56 px header: content-sized section buttons (`padding:0 17px`) after the team selector, with Refresh and Ask AI on the right (`index.html:140-146`, `header()` ~1848). Fastrock stacks sections, page tabs, scope and breadcrumb into about 150 px with `Row(54).Dynamic(6)` stretching each section to 1/6 of the width (`rally.go:387-443`).
- Navigate inside the current Rally tab (`go()` ~1791). Every navigation click currently opens or focuses a separate document tab (`rally.go:404,416`, `workspace/state.go:151-157`). Use middle-click or a menu item for "Open in new tab".
- Drop the "Fastrock / Title" breadcrumb (`rally.go:443`) and draw the page title at 24 px/700/#5c6573 (`index.html:160`).
- Show List/Board/Charts only on board pages and timeboxes, with ☷/▦/▥ icons and a solid #3272d9 active state (`pagebar()` ~1867, css ~420). Include planning and timeline modes so that picking a mode doesn't permanently lose them (`rally.go:238-246`, `catalog.go:11,23,24`).
- Add Comfortable/Compact density (~2221, css 545-551) and the ⚙ page settings: WIP limit, card color, age threshold (~2219).
- Add Home → Recycle Bin with Restore and Delete Permanently via WSAPI `RecycleBinEntry` (~1746, ~2244). The delete prompts already reference it ("cannot be undone here", `detail.go:674`).
- Keyboard: `/` focuses search, Alt+B opens Team Board, Escape closes the detail, Enter opens the focused card (~2458-2469). Register them in `shellActions` so they are rebindable (`actions.go:236-246`, `interactions.go:63-73`).
- Show a spinner, "updated N s ago" and a disabled state on Refresh (`rally.go:444`). Make Refresh in a detail reload the item while keeping edits (I.refresh); today it reloads only the background list (`rally.go:444`, `actions.go:241`).
- Set the tab title to the open item's ID (~2083); today it stays the page name (`app.go:365-373`).

### Team Board and cards
- Lane headers: display labels (Idea/Backlog, Define/Ready, In-Progress, Review/Deploy, Accepted) with `n/WIP` (~1754, ~1925), a #ebeff5 fill, a 1 px #c4cdd7 bottom border and bold #454e5a text (css ~237). Fastrock shows `"State   N"` in state colours with no separator (`board.go:119,173-180`).
- Lane collapse to 45 px with a vertical label (~1926, css 471-473).
- Per-lane dashed "＋" quick add that pre-sets the state (~1927, css ~469, `"add-state"` ~2237).
- Drop feedback: dashed blue outline plus blue-soft fill on the whole lane, the source card at opacity .45 (css ~241, ~246), and a ghost card under the cursor. Fastrock fills only the 30 px header (`board.go:174-178`).
- Optimistic move: show the card in its new lane immediately with a pending marker, lock it against a second drag, show a toast with Undo, and patch the single item instead of reloading the whole board (`board.go:302-325` calls `refreshRally`). Report in the toast, not `a.status`, which lives in the hidden footer.
- Cross-swimlane drops should set the group field, and drops onto a card should rerank (~2449-2455). Fastrock changes only state (`board.go:219-223`).
- Show stories, defects, test sets and defect suites on the Team Board (~1843) by querying Artifact with a type filter; it is HierarchicalRequirement only (`catalog.go:14`). Give cards a type icon.
- Card type icon and colour: a circle from DisplayColor, ◆ for defects, and a left accent (~1902, css 256-259). Add DisplayColor to the fetch list (`stream.go:15`).
- Card actions: hover toggles for ready (✔/●) and blocked (⬟) plus a ⋮ menu (~1903, ~2242-2244). Move selection to Ctrl/Cmd-click instead of the checkbox square in the action area (`board.go:250-259`).
- Ready/Blocked styling: ready #e0faeb/#36af13/#2a9f16; blocked #ffefef/#d85252/#c24949 with a 3 px #e46a65 left border (css 276-278). Fastrock fills both with `p.Selected` (`board.go:280-288`).
- Card footer: task % complete, to-do hours and discussion count (~1909-1910). Fetch TaskStatus, TaskRemainingTotal and Discussion. Pluralize correctly; "1 tasks" today (`board.go:65`).
- Show "—" or hide the pill for unestimated cards instead of "0" (`board.go:65`).
- Owner block: an "Owned By:" label and "—" initials for no owner (~1906, css 269-271). Fastrock shows "U".
- Ellipsize the third title line and show the full title on hover (`board.go:265-270`).
- Use compact, content-sized cards. The fixed 196 px card (`board.go:194`) leaves large gaps on short cards; with density modes, use two fixed strides to keep virtualization.
- Keyboard: focusable cards (`tabindex=0`) and Enter to open (~1900, ~2468).
- Swimlanes: ▾/▸ toggle and count badge (~1921, css ~425), sized to content. Fastrock uses fixed 420 px lanes with nested scrollbars inside the scrolling page (`board.go:136-142`).
- Remove "Show Work Rules based on State" (inert; `v.Rules` is never read, `rally.go:40,343`) and "Show Exit Agreements" (invented text, `board.go:181-183`), or source them from real data (V11).
- Add card-field toggles to Show Fields (~2229); today it changes only table columns (`rally.go:295-307`).
- Minimum lane width 220 px (css ~236); Fastrock uses 190 (`board.go:148`).
- Show an empty board state ("Based on your active filters… [Clear filters]", ~1963).

### Lists, columns, grouping and bulk actions
- Use schema DisplayName for column headers and the field picker instead of `FormattedID`, `ScheduleState`, `PlanEstimate` and similar (`rally.go:297,545`).
- Show ↑/↓ sort indicators (~1959). Make collection columns (Tasks, Discussion) non-sortable (`rally.go:544-552`).
- Per-kind default columns that are always editable (~1935-1939). Today Task, Project, User, Iteration and Release pages hard-code columns (`rally.go:514-518`), Defects lack Severity/Priority/State, and Test Cases get ScheduleState/PlanEstimate (`rally.go:73`).
- Build the fetch list from the visible columns (`stream.go:15`). Discussion is always 0, timebox dates are always "—", Users show ObjectIDs, and TestCase LastVerdict/Method are missing.
- Grouped tables: sort by group key first and put counts in the group headers (~1953, ~1961). Fastrock repeats headers (`rally.go:556-561`).
- Label Group By as "Swimlanes"/"Group By" with None/Owner/Feature/Iteration/Project (~1881). Drop raw "ScheduleState", and preserve unknown groups instead of resetting them to None every frame (`rally.go:255-256`); that reset breaks saved-view and AI groups.
- Inline editing: numeric Plan Estimate, a Blocked checkbox, a row ⋮ menu, a Rank column and a totals row (~1955-1961).
- "Page X of Y" plus page buttons (~1964). Previous/Next mixes local pages with server windows and shows no position (`rally.go:365-383`); "Previous" at page 1 skips about 103 items. Prefer one virtualized scrolling list, consistent with the board.
- Render ID and Name as link-style cells that wrap, not bordered buttons cut at 58 characters (`rally.go:576-579`).
- CSV: export *all* matching items with progress, map collections to counts, use display headers and suggest `<page>.csv` (~2199; Fastrock `actions.go:96-136` exports only the loaded window).
- Bulk "Edit selected" for state, owner and iteration using schema values (~2254), replacing Block/Unblock/Complete/Delete (`rally.go:322-338`).

### Filters, search, saved views and scope
- Show active filters: "⚑ Show N Filters" plus removable chips like "owner is X ×" (~1880, ~1891-1896). With the panel closed, owner/state/blocked/ready and the WSAPI query are invisible today (`rally.go:252`).
- Source owner options from all users (~1888). They come from loaded items, so after filtering the list shrinks to the selected owner (`board.go:46-76`).
- Apply the advanced WSAPI query only on Apply or Enter. It is part of `rallySignature`, so every 300 ms typing pause sends a half-typed expression and errors flash (`rally.go:138,152-163,282`).
- Add a filter builder with field/operator/value rows (is / is not / contains) for Iteration, FY Quarter, Team, Tags and Type (~1891-1895).
- Saved views should snapshot mode, group, columns, sort, filters, search and timebox (~2163-2171). They store only Query/Group/Mode (`settings.go:16-22`, `rally.go:223`). "Standard View" should reset everything (`rally.go:202-206`).
- Saved-view management: a dirty "Save changes" / "Revert" pair and a ⌄ menu with Add, Edit, Copy, Delete and Manage (~1869, ~2230-2235). Fastrock overwrites same-named views silently, and the "Custom Views" page lists user stories (`catalog.go:30`).
- Timeboxes: separate Iteration and Release pickers sorted by date, with dates and a current marker; deduplicate same-named timeboxes across child projects and filter on `Iteration.Name` so sibling teams aren't excluded; show the pickers only on board and plan pages (~1865; Fastrock `rally.go:83-92,180-191`).
- When the selected timebox or project isn't in the loaded list, show it anyway instead of displaying "All timeboxes"/"All teams" while the filter stays applied (`rally.go:186-191,426-427`).
- Parents/Children toggles must call `loadScope()` as well as `reloadRally()` (`rally.go:436-441`).
- Scope picker: show the team name prominently with a team menu and a workspace menu (~1852, ~2210-2211). Make the project and user pickers hierarchical and searchable (`rally.go:419-441`, `app.go:458`).
- Search: add the global "Search all work items" across types on `/` (~1857, ~2422), include Tags (~1834), and fetch enough fields for the local fallback to match descriptions (`rally.go:451-453`).
- Page presets: Iteration Status defaults to the current iteration and shows its summary metrics (~1795-1797, ~2090); My Work Items is limited to the current user (~1753); Backlog is unscheduled items. Backlog and User Stories are identical today (`catalog.go:8-9,33`).

### Detail view and properties
- Header: navy #12356c bar with type icon, ID, ⋮ menu, inline title input, "Unsaved changes" chip, "Save *", Show Fields, Templates and × (~1985, css 500-506). Fastrock has Back / ID / Open web / Save / Delete and no dirty indicator anywhere, including the tab (`detail.go:115-139`).
- Title new items "Create User Story" (~2136), not "New HierarchicalRequirement" (`detail.go:124`).
- Tabs with icons and counts ("Tasks (3)"), including Defects and Test Cases, and "Revision History" (~1984-1987). Hide Tasks/Children on Tasks. Read `UserStories` for Feature children (`detail.go:541-545`); today they never appear.
- Ready/Blocked as "✔ Ready" / "⬟ Blocked" status buttons with green/red active fills (~2011, css 528-531). Show Blocked Reason only when blocked (`detail.go:290-298,332`).
- Build the properties sidebar from the schema in reference order: Color, Owner, Team, Schedule State, Flow State, Plan Estimate, Priority, Task Roll-up, Parent, Severity, Tags, FY Quarter, Iteration, Milestones, Expedite, custom fields, then Created/Updated/Submitted By (~1988-1999). Use DisplayName, so workspaces see "FY Quarter" and "Team" instead of the hard-coded "Release" and "Project" (`detail.go:299-332`). Draw the Parent/Feature pickers; their editors exist but are never shown.
- Layout: a 72/28 split with the sidebar at least 300 px and Description at least 360 px tall; custom HTML fields inline, not under "More fields" (css 513, 521; Fastrock `detail.go:214-216,240-243`).
- Add Defect "Steps to Reproduce".
- New items: default owner to the current user, iteration to the current timebox and state to the originating column (~2135-2136).
- Leaving with unsaved changes: Keep editing / Discard / Save (~1813-1817). Fastrock offers only Discard (`detail.go:116-121`).
- On a conflict, offer "Reload item" that keeps local edits, instead of "Reopen it" (`detail.go:495`).
- Reference fields under "More fields" (WorkProduct, Requirement, PortfolioItem) should be reference pickers, not text boxes holding WSAPI URLs (`detail.go:714-718,217`). Feature/Parent hold display names that fail validation when edited (`detail.go:414-419`).
- Report single deletes and bulk edits in plain language. They currently use AI-plan wording, "Applied 0 of 1 changes … Refresh before retrying" (`detail.go:694-699`). Close the detail only after the delete succeeds (`detail.go:686`).

### Rich text editor
- Specific toolbar tooltips such as "Bold (Ctrl+B)"; all five buttons currently say "Toggle formatting" (`richtext.go:150-154`).
- Render discussion and revision posts read-only, without Edit/Preview/HTML toggles (`detail.go:643-648`, `richtext.go:128-137`).
- Links: Ctrl+K, remove link, URL validation, and feedback when nothing is selected (`richtext.go:169-172`).
- Comment placeholder "Add a comment..." (~2016). Size posts to their content instead of a fixed 120 px (`detail.go:648`).
- See §1 for the unsupported-HTML disclosure (V9).

### Tasks, discussion, attachments and revisions
- Tasks tab: a table of ID/Name/State/Estimate/To Do/Actuals/Owner with × to remove and a full task form (~2015, `taskForm` ~2329). Fastrock lists "ID  Name" buttons and uses a name-only dialog with a "Save" button (`detail.go:618-657`, `actions.go:31`).
- Discussion: avatar initials, author *and* date chip, and Delete (~2016). Fastrock shows user *or* date (`detail.go:640`).
- In-flight guards and progress for Add comment, upload and task creation, which can currently post twice (`detail.go:567-616`).
- Attachments: a Name/Size/Added/Remove table, multi-file upload and real content types (~2017). Uploads always send `application/octet-stream` (`detail.go:604,651`). Suggest the filename on download and confirm success (`actions.go:137-152`).
- Revisions as a table of Revision #, Date, User and Changes (~2018), not a 120 px rich box per revision (`detail.go:638-648`).
- Tab loading: clear errors on a later successful load, and ignore stale responses that clear Loading for another tab and show a false "No tasks…" (`detail.go:552-559`).

### Charts, reports, planning and timeline
- Give each page its own content. My Rally, Team Status, Quality Management, Reports, Custom Reports and Insights all render the same `charts()` (`catalog.go:7,15,21,27-29`). The reference has welcome/recent panels, per-user task hours and a filterable report catalog (~1437, ~1470, ~1610).
- Compute charts over all matches using WSAPI aggregate queries or paged counts, or label them "loaded items (N of M)". Today they use the loaded window while claiming "selected scope" (`rally.go:612-640`). Render metric tiles at 36 px (`.metric`, css ~309).
- Quality Management buckets test cases by ScheduleState, which TestCase lacks (`rally.go:615-617`). Use LastVerdict.
- Timeline: Gantt bars, date range, zoom, dependencies and milestones (~1552-1574, css ~322), not text rows (`rally.go:662-672`).
- Planning: points versus capacity with progress bars, expandable iterations and a "Plan" action (~1449-1468); add a Releases toggle on Timeboxes (~1484).

### Ask AI (usability; the reference has no AI)
- Dock the assistant as a right-side panel in the Rally tab instead of a fixed 760 × 690 popup at (350,100) that hides the board it changes (`assistant.go:43`).
- Show the change preview as a before→after table with per-row checkboxes and red delete rows, not raw JSON in a 70 px box (`assistant.go:107-115`).
- Show an "AI view" chip when the assistant applies a view, validate the requested group, and label the saved-view combo (`assistant.go:257-264`, `assistant/tools.go:52`).
- Confirm before a scope change discards the conversation, or keep one session per scope (`assistant.go:28-34`).
- Show tool activity (which Rally queries ran) in the transcript, rendered as Markdown blocks rather than string concatenation with "You:" prefixes (`assistant.go:176,210-211`).
- Add Ctrl/Cmd+Enter to send and a "New conversation" action. Label the model/effort/tier combos and lock them mid-turn (`assistant.go:51-91,142-152`).
- Highlight "Ask AI" only while the panel is open (`rally.go:447`).

### Visual tokens
- Add a Rally sub-palette for Rally tabs. Light: bg #f5f7fa, line #d4d9e0, ink #293442, muted #748090, blue #3272d9, blue-soft #eaf2ff, lane #eaf0f6. Dark: #17212d / #202a37 / #3c495a / #83afff (css 17-34, 552-563). Fastrock uses neutral grays (`theme.go:19,21`).
- Primary hover #2465cd (css ~198); see the §6 button fix.
- Danger style for destructive actions: a red "Delete" (css ~199, ~2248) instead of a blue "Confirm" (`actions.go:40-53`).
- Active segments: solid #3272d9 with white text (css ~420).
- Toasts: a dark #35495f bottom toast that clears after 4 s (~1764-1768, css ~433).
- Make Selected and Hover distinct. They are #dde4f3 vs #e6e8ec in light and #3a3f4b vs #33363c in dark (`widgets.go:217-232`), which weakens V14.
- Card ID in 14 px semibold #2673ed (css ~249). Don't colour "Defined" amber like a warning (`rally.go:712-713`).

### States and errors
- Distinguish "Connecting to Rally…" from "Connect to Rally". While connecting, the tab shows the connect prompt with a blank message (`rally.go:164-172`) while Settings already says "Connected". Disable Refresh and Ask AI with no client.
- Make errors actionable: map 401/403 to "Update your API token in Settings" with a button, and add Retry next to `v.Error` (`rally.go:312-315`). Surface WSAPI `Errors[]` instead of "(200)" or a bare status text (`rally/client.go:129-144`).
- Don't show "No work items match…" during the initial load (`rally.go:589-591`).

---

## 8. SPEC conformance and documentation accuracy

- **V4 / V13**: the per-frame O(N) paths in §2 (info panel, New tab, conversation chooser, Rally pickers, planning, timeline, transcript, file view) contradict T5 and T8 being marked done. Fix them, add regression benchmarks at 100k conversations, then re-mark the tasks.
- **V5 / I.refresh**: the 60 s refresh replaces loaded data instead of stale-while-refresh; there is no jitter; and Refresh inside a detail doesn't reload the item (§2, §7).
- **V6**: scope metadata and all `State` objects are fetched eagerly, up to 100k objects each (`app.go:451-455`, `detail.go:75`, `rally/client.go:211-238`). Page them or search on demand.
- **V7**: memory pressure doesn't back off fetching or release GPU surfaces, and the RSS-versus-heap comparison makes it fire constantly with 3+ windows (§3).
- **V10**: no `thread/unsubscribe`, and app-server is force-killed (§1). Busy closed chats and `infoViews` are never released (§3).
- **V11**: inert or placeholder controls: Work Rules, Exit Agreements, the "Ready" label, "Load older messages", the Diagnostics configuration key, "Custom Views", and Rally pages that share generic behavior (§6, §7). Implement them or remove them and document the exclusion.
- **V12**: add darwin/amd64 to CI, and vet plus nucular tests to the release workflow (§5).
- **V14**: add meaningful tooltips to the tab close ×, folder chevrons, queue ↑/↓, attach "+" and the rich-text buttons (`widgets.go:115-142,197-216`, `chat.go:817-832,900`, `richtext.go:150-154`). Replace the "▾/▸" text glyphs in the terminals disclosure with drawn chevrons (`info.go:186-192`). Make Selected and Hover fills distinct (§7).
- **I.windows**: share Rally client, cache and metadata through the broker instead of loading them in every window; re-run `connectRally`/`loadScope` on preference sync; return a closed pop-out's documents to a live window immediately; and add a single-instance lock (§1, §4).
- **I.persistence**: persist window geometry; keep snapshot construction off the UI thread and bounded; and never auto-send restored queues (§1, §4).
- **I.operations**: actionable Codex errors with the stderr tail (§6). `README.md:21` promises an "Exit button", but startup errors use a plain OK alert (`main.go:102-106`). Make them match.
- **T2 (still open)**: unsubscribe and stop on close or quit; paged history; typed attachments; readable shortcut labels; a warnings list; assistant approvals; approval routing and pruning; the inert Rally controls and pages.
- **T7 (still open)**: intraline diff; desktop notifications; System theme; the reference renderer choice (Auto/Software/GPU) or a documented exclusion; approval card layout; DPI scaling.
- **`docs/INTERACTIONS.md` overstates coverage.** It claims "Usage/status" in the info panel, model/effort/speed/plan in the composer, and search, refresh and pagination in the sidebar. It omits that the Permissions picker, `!` commands, server-side search, unread and waiting marks, the warnings banner, busy-close confirmations and the file-change approval diff are missing. Several rows described as "layout differences" are behavior bugs (feature toggles, forced sign-out, draft loss, MCP overwrite, `/approve`). Rewrite it as a per-callback checklist with Implemented / Partial / Missing / Excluded status, so V11 can be audited mechanically.
- **`docs/ARCHITECTURE.md`**: "four concurrent requests per client" (`:79`) is true only for `CachedQuery`. "JSON decoding … run in workers" (`:86-88`) is contradicted by `upsertItem` and transfer decoding on the UI thread. Update the docs or fix the code (§2).
- **README Rally section**: "Charts compute counts and estimates from the current selection" should state that they use only the loaded window. "Boards display real WSAPI results" should note that Defect and Portfolio boards currently use hard-coded state lists (§1).
