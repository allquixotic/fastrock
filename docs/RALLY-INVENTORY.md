# Rally functionality before the Rust port

Audited baseline: Fastrock `f523c6b7ae6f9b6a12ca951845b56d81cb76a879`
(v0.1.5), 2026-10-10. Source retained in Git history and the separate local
`fastrock-go-baseline-f523c6b` checkout. This records implemented behavior,
including deliberate limits; it does not claim full Rally web-app parity.

## Workspace and screen layout

Rally opens in horizontal document tabs alongside chats, files and Settings.
The left application sidebar lists only Codex conversations. Rally has its own
navigation inside its document and does not consume conversation-info-panel space.
Rally tabs can be popped out or transferred between windows, preserving filters,
selection, detail drafts, rich edits, collection paging, assistant proposals and
scroll positions. Pending mutations block close/transfer. Dirty editors require
save/discard/cancel before navigation. Independent tabs keep independent views.

The normal page stacks these rows above its content:

1. Section buttons: Home, Plan, Track, Quality, Portfolio, Reports.
2. Page buttons for the selected section; selecting opens a Rally document.
3. Team selector plus Parents/Children scope checkboxes.
4. Section/page breadcrumb, Refresh, Ask AI.
5. Update/loading/error status and last-refresh time.
6. Standard/private saved-view selector, Save view and changed-view controls.
7. Iteration and FY Quarter selectors, including Current iteration.
8. View actions: create, edit selected, export and saved-view actions.
9. List/Board/Charts and page-specific Planning/Timeline mode selectors.
10. Comfortable/Compact density; board Page settings.
11. Search and quick action/filter controls.
12. Swimlane/group selector, active removable filter chips.
13. Optional structured-filter builder, advanced WSAPI query and field selection.
14. Optional board widgets/exit agreements/rules, then loaded-result totals.

Every row can be hidden with a minus control. Hidden rows appear as compact
restore buttons, including Restore all. Visibility persists across restart and
windows without changing the hidden controls' values. Controls wrap on narrow
windows. Keyboard search reveals the hidden search row. Source: legacy
`internal/ui/rally_rows.go`, `rally.go`, `rally_controls.go`.

## Page catalog (actual endpoints and default presentation)

| Section | Page ID / caption | WSAPI kind | Default mode |
|---|---|---|---|
| Home | myrally / My Rally | HierarchicalRequirement | dashboard |
| Home | projects / Teams | Project | list |
| Home | users / Users | User | list |
| Home | mywork / My Work Items | HierarchicalRequirement | list |
| Plan | backlog / Backlog | HierarchicalRequirement | list |
| Plan | userstories / User Stories | HierarchicalRequirement | list |
| Plan | timeboxes / Timeboxes | Iteration | list |
| Plan | teamplan / Team Planning | HierarchicalRequirement | planning |
| Plan | workviews / Work Views | HierarchicalRequirement | list |
| Track | iterationstatus / Iteration Status | HierarchicalRequirement | board |
| Track | teamboard / Team Board | mixed Artifact | board |
| Track | teamstatus / Team Status | HierarchicalRequirement | dashboard |
| Track | tasks / Tasks | Task | list |
| Quality | defects / Defects | Defect | list |
| Quality | defectsuites / Defect Suites | DefectSuite | list |
| Quality | testcases / Test Cases | TestCase | list |
| Quality | testfolders / Test Plan | TestFolder | list |
| Quality | qualitymanagement / Quality Management | TestCase | dashboard |
| Portfolio | portfolioitemstreegrid / Portfolio Items | PortfolioItem/Feature | list |
| Portfolio | capacityplanning / Capacity Planning | HierarchicalRequirement | planning |
| Portfolio | timeline / Timeline | PortfolioItem/Feature | timeline |
| Portfolio | releasetracking / FY Quarter Tracking | Release | list |
| Portfolio | portfoliokanban / Portfolio Kanban | PortfolioItem/Feature | board |
| Reports | reports / Reports | HierarchicalRequirement | dashboard |
| Reports | customreports / Custom Reports | HierarchicalRequirement | dashboard |
| Reports | insights / Insights | HierarchicalRequirement | dashboard |
| Reports | customviews / Custom Views | local preferences | list |

Team Board uses one server-paged `artifact` collection with concrete `types`:
HierarchicalRequirement, Defect, TestSet, DefectSuite. US/D/TS/DS cards retain their
concrete types for schemas, details, writes, ranking and exports. Team Board lanes
use ScheduleState even for defects; a defect's own State remains a separate field.
Portfolio lanes use State objects and their references. Task lanes use State.
Test-case state presentation uses LastVerdict. Workflow values come from workspace
schema/State catalogs, not a fabricated fixed list.

Backlog adds `(Iteration = null)`. My Work waits for the authenticated user's
unscoped `GET user` identity and then filters Owner by its verified reference.
Iteration Status enables Current iteration and widgets. Current iteration chooses
an active [StartDate, EndDate) interval, prefers the selected team, then latest
start and deterministic reference ordering; failures do not broaden the query.

Dashboard/report aliases share loaded-window charts. Planning aliases share
iteration/state count and estimate summaries. Timeline is a list of planned start
and end dates, not a Gantt control. Enterprise administration, Lookback history,
capacity data, app-catalog plugins and server-specific custom pages were absent.

## Connection, scope and query behavior

Settings → Rally contains endpoint, masked API-token input, workspace and project
selectors, parent/child project scope and connection feedback. Endpoint defaults
to https://rally1.rallydev.com. Token is in Windows Credential Manager/macOS
Keychain under service `fastrock`, username = endpoint; settings.json contains no
token. FASTROCK_RALLY_TOKEN overrides reads for local fixtures. FASTROCK_HOME
isolates application settings/session files. New settings writes are atomic.

Direct WSAPI v2 client adds `/slm/webservice/v2.0/`, `ZSESSIONID`, Accept JSON and
Fastrock integration headers. Production endpoints require HTTPS; HTTP is allowed
only for loopback hosts. References must stay on the configured origin and inside
its WSAPI subtree, without credentials, query, fragments, traversal or backslashes.
Redirects are disabled, including same-origin redirects. Concrete references are
validated by canonical type plus positive numeric ObjectID.

At most four concurrent requests; 30-second timeout; 32 MiB response cap. GET
429/502/503/504 retries at most three times, with cancellable exponential delay or
Retry-After capped at 30 seconds. Mutations never auto-retry. HTTP/WSAPI envelope
errors are surfaced and token material is redacted. 401/403 offers credential
Settings. Typed numeric/string/reference conversions preserve readable values.

Query parameters carry workspace/project/projectScopeUp/projectScopeDown,
fetch, one-based start, pagesize, order, query and Artifact types. Pages are
server-paged, checked for repeated/unexpected/missing records. General all-results
reads stop at 100,000 items. Board working sets cap at 2,048 projected cards;
page cache is bounded to 24 MiB and invalidated on mutation. Schema caches are
workspace/type scoped, expire after ten minutes, and cap at 32 entries.

Search matches ID, title, owner and readable descriptions with escaped server
predicates and local searchable projections. User search uses DisplayName/UserName;
project/timebox/folder search uses Name. Search is debounced around 305 ms. Owner,
state, Blocked and Ready quick filters combine with advanced query, iteration,
release, page preset and structured filters. Unassigned owner means Owner=null.
Reference timeboxes preserve both ID and name for cross-team equivalent names.

Structured filters support Iteration, FY Quarter (Release), Team (Project), Tags
and Type; operators are is/is not/contains. Exact choices use validated refs;
Tags use contains/!contains; substrings use nested Name. Type narrows the mixed
Artifact types rather than querying `_type`. Up to 64 clauses. Invalid restored
clauses fail closed. Chips remove individual clauses; Clear all also disables
Current iteration. Unapplied advanced/builder drafts survive restore separately.

Sort Rank maps to opaque DragAndDropRank internally; other schema fields sort
ascending/descending. Grouping prefixes order and supports None, Owner, Iteration,
Release, ScheduleState, Feature, Project and saved custom fields. Queries for
Workspace/User/Project remove project scope where inappropriate.

## Tables and inline editing

Columns vary by type: story ID/name/ScheduleState/PlanEstimate/Owner/Iteration;
defect State/Severity/Priority; task State/Estimate/ToDo/Actuals/Owner; test case
LastVerdict/Method/Owner; timeboxes Name/dates/Project; portfolio State/Owner/planned
dates; users DisplayName/UserName. Work artifacts include Rank and Blocked where
supported. Fields/columns can be selected from schema, including custom fields.

Rows wrap titles and grow to prevent clipping; Comfortable/Compact changes row
padding. Rank displays numeric position in the current result and continues across
pages. Formatted IDs open native details. Group headers and totals explicitly
cover loaded matching records/current page. Numeric totals are schema-aware.
Selection persists by ref, including retained selected objects across paging.

Single-click writable cells open native text/numeric inputs, boolean toggles,
allowed-value dropdowns or paged searchable reference choices. Identity/revision/
opaque rank, collections, readonly and rich TEXT fields are not inline editable.
Enter or Save applies field-only edits; Cancel discards; Full editor promotes the
same draft. Connection/scope snapshots and revision stamps guard saves. Conflicts
retain draft and offer reload/comparison. Row actions include native details,
browser artifact link and target-specific operations. Menus remain onscreen.

## Boards, drag and ranking

Board uses workflow columns, optional group swimlanes and horizontally scrollable
lanes. Unified cards show type badge, ID, wrapped title, Owner, Iteration, task
rollup, estimate, Ready/Blocked, color and age. Card-field choices persist. Groups
and lanes collapse; keyboard focus survives refresh/removal and reveals cards.
Tab/Shift+Tab navigates cards; Enter opens; `/` focuses search; Alt+B opens Team
Board; Escape closes details subject to dirty/saving guards.

Drag changes state/swimlane and relative position with immediate pending placement.
One mutation combines fields plus `rankAbove`/`rankBelow` referencing another item.
DragAndDropRank is never synthesized or directly written. Preflight rereads
LastUpdateDate/VersionId; missing stamps prevent movement. Failure rolls back.
Revision-checked Undo restores prior fields/relative placement. Pending writes
defer refresh. Same card cannot be ranked against itself.

Board Page settings: density, Work Item/Owner/Priority color, WIP limit (0 unlimited),
age threshold (0 disabled). Work Item honors DisplayColor; other modes use stable
identity colors. WIP warnings count loaded matching cards and are advisory. Age
uses whole days since LastUpdateDate. New tabs inherit applied defaults; existing
and saved views retain snapshots. Unapplied settings drafts survive restore.
DPI/font/density changes preserve lane scroll anchors. All calculations are
presentation only; they do not enforce Rally workflow rules.

## Detail editor, relations and rich fields

Details replace page content below navigation. Wide layout puts Name and rich
content left, schema properties right; narrow layout stacks them. Toolbar offers
back, Save/Cancel/reload/delete and edit status. New artifacts inherit current user
(default only while untouched), project, workspace, timebox and originating lane.
Required fields, numeric/boolean/date types, choices and readonly metadata are
validated. Identity fields cannot be edited. All c_* schema fields remain visible.

Description, Notes, AcceptanceCriteria, custom HTML TEXT and discussions support
bold/italic/underline/strike, headings, lists, links, preview, HTML source and
undo/redo. Untouched original HTML is restored byte-for-byte, including after undo.
Supported-format edits normalize HTML; source mode preserves complex tables/media.
Editor history is bounded across windows; unopened rich fields allocate lazily.

Reference pickers support Owner/Iteration/Release/Project/Feature/Parent and other
schema references, paged search with clear/current-value handling. Portfolio State
uses refs. Tags and Milestones are writable collections serialized as arrays of
`{_ref}`. Complete selections must load before saving (10,000-item guard), with
retryable errors for changed totals, duplicates, missing/invalid references.
DisplayColor uses documented Rally palette or server-advertised values; Default
clears it. Blocked reveals BlockedReason; Ready is independent.

Reload performs a three-way merge: unchanged local fields take remote values;
local-only edits remain; fields changed both locally/remotely require Keep my edit
or Use Rally value, with full comparison. Unresolved choices block Save. Typing
while reload is in flight and unsent comments remain. Original revision/connection
must still match when Save starts. WSAPI has no atomic compare-and-swap: another
edit can occur after the preflight read; the UI must not promise atomicity.

Save acknowledgements merge omitted fields with the submitted snapshot, retain
edits/comments typed while the request ran, and preserve known collection
membership when a returned summary has the same count. A different count requires
fresh review. Explicit server nulls take precedence. Reload retrieves complete
Tags/Milestones before the three-way comparison; validation errors keep an
in-flight mutation's guard active until its own completion callback.

Supported subtabs: Details, Tasks, Test Cases, Defects, Children (Feature→UserStories),
Discussions, Revision History, Attachments, More fields. Only schema-supported
relations appear; Task omits Tasks/Children. Counts show known totals or unknown.
Collections page at 128 records with range, total, Next/Previous and Retry.
Discussions query ConversationPost by Artifact. RevisionHistory resolves Revisions.

Tasks and children can be created with their parent reference. Related records
open details; associations can be added/removed where writable. Comments post
HTML through ConversationPost with current-user identity. Attachment upload creates
base64 AttachmentContent then Attachment (5 MiB cap), cleans orphan content on
second-step failure; download retrieves content, delete confirms and guards the
reviewed item. Artifact deletion requires confirmation and revision checking.

Bulk Edit selected intersects writable schema fields/choices across concrete
artifact kinds. State/owner/iteration updates preview exact old/new values per item.
Every item retains its type and reviewed revisions; first failure stops, reports
completed count and requires fresh review for remaining work. No multi-item atomic
transaction exists. Pending batches block tab transfer/close.

## Saved views, export, refresh and assistant

Private views live locally: name, page, mode, query, search, group, sort/direction,
columns/card fields, iteration/release refs+labels, Current iteration, quick filters,
structured filters and board display snapshot. Save validates names and confirms
overwrite. Custom Views works without connectivity and supports search, Add,
Edit/rename, Copy, Delete and Open. Copies own independent snapshots. Renaming or
deleting definitions does not replace live filters/drafts or delete Rally items.
Legacy JSON field names are recorded in `internal/settings/settings.go`.

CSV exports use the full applied server scope, not only displayed rows. Paging and
cancellation are explicit; formula-like spreadsheet cells are quoted to prevent
execution. Charts show loaded counts/estimates by workflow; they do not invent
historical velocity/burnup or imply complete server totals. Auto-refresh around
60 seconds pauses for dirty editors/pending mutations, uses backoff on failures,
retains useful stale results and exposes manual refresh/retry/cancel state.

Ask AI docks alongside the owning Rally document (below on narrow windows), keeps
history/draft/proposal when hidden, and uses the same external Codex provider/model
catalog. Tools: rally_query (≤200 rows in current scope), rally_get (scope checked),
rally_fields, rally_show_view (validated page/mode/group), rally_propose (1–50 typed
create/update/delete changes). Returned Rally text is untrusted data; tokens never
enter prompts. Writes cannot run through read/view tools or shell instructions.

Proposals contain real old/new values, per-change selection and full inspection.
Create fills the chosen workspace/project; identity fields and out-of-scope moves
are rejected. Explicit human Apply rechecks reviewed revisions, stops on first
failure and preserves outcomes/completed count. Failed/partial plans need a fresh
review. AI-applied views have a visible label. The assistant can describe delivery
health and change native filters/grouping; it must not invent records or successes.

## Port acceptance ledger

Each row requires Rust implementation plus fixture/Windows evidence. The full
layout/behavior contract above remains authoritative; a checked row must include
its detailed behaviors, not merely a same-named button.

| ID | Capability | Port state |
|---|---|---|
| R01 | catalog, horizontal documents, conversation-only sidebar | Rust catalog and native horizontal documents; Windows list/mixed-board/two-document checks |
| R02 | endpoint validation, transport security, errors, retries/cancellation | client.rs and transport.rs; reference/HTTPS/redaction/no-retry/cancellation/frame-bound tests |
| R03 | OS vault and legacy preference migration | store.rs OS-vault adapter and migration; legacy/null/snapshot/closed-tab tests |
| R04 | workspace/project/timebox/current-user catalogs and presets | controller catalogs + view presets; scoped Windows requests and scope/search/backlog tests |
| R05 | combined scoped query/search/quick/structured/type filters | view.rs composed queries + native builder; fail-closed filter tests and independent draft checks |
| R06 | list paging, columns, grouping, sort, numeric rank/totals | presentation.rs/list controls; 128-row paging, 300-card paging, grouping/rank/totals in Windows |
| R07 | native inline text/numeric/boolean/choice/reference edits | PropertyEditor + typed editor; JSON-shape validation and Windows field/reference edits |
| R08 | mixed boards, workflow lanes, swimlanes, collapse/card fields | native DragArea/DropArea, ListView lanes and vertical group rows; mixed-type/swimlane/card-field checks |
| R09 | drag, relative rank, revision guard, rollback and Undo | revision-guarded move/Undo; real Windows mouse drag and rankAbove/rankBelow request evidence |
| R10 | density/color/WIP/age and row visibility/restore | native page settings/row restore + persisted display snapshots; Windows hide/restore and layout screenshots |
| R11 | schema-driven details/create/save/delete and defaults | schema-driven controller/editor; Windows edit/save and create-in-Completed lane; required/typed tests |
| R12 | native rich editing/source/preview and exact HTML preservation | native TextInput/StyledText and source/preview; UTF-8/headings/lists/link/HTML-preservation tests and source save |
| R13 | references, Tags/Milestones, color and validation | native pickers/palette; real User-choice click, complete-selection paging and typed collection/ref validation |
| R14 | reload three-way conflict handling and dirty navigation | editor reload/acknowledgement merge; sparse/late-edit/collection-summary tests and Windows delayed-save, write-guard, Save/Discard/Cancel/Save-and-continue |
| R15 | tasks/children/test cases/defects/discussions/revisions | metadata-driven relations; supported-subtab test and Windows Tasks/Attachments/Revisions/Discussions reads/comment |
| R16 | attachment upload/download/delete and orphan cleanup | client attachment services + native file dialogs; download/conflicting-delete/orphan-cleanup/5-MiB fixture tests |
| R17 | bulk edit preview, scope/revision checks and partial outcomes | shared_fields and reviewed plan; Windows two-item bulk preview/apply plus partial-failure/no-retry test |
| R18 | personal views create/rename/copy/delete/open and restore | atomic private-view snapshots + native searchable manager; independent-copy test and Windows save/restore |
| R19 | safe full-query CSV export and loaded-only analytics/timeline | full applied-query paging + CSV neutralization tests; native loaded-only metrics/planning/timeline projections |
| R20 | freshness/auto-refresh/backoff/memory bounds | bounded cache/working sets/schema cache and cancellable workers; in-flight cache-invalidation test and board paging |
| R21 | assistant tools, native dock, proposals and human Apply | five external-Codex dynamic tools + native dock/review; Windows proposal/human Apply, typed/scope/partial tests |
| R22 | keyboard/DPI, independent tabs, draft/session restore and windows | native keyboard/focus/DPI/layout; persisted document UI/drafts and durable window handoff; restart/popout/transfer checks |

## Verification scope and native-control differences

The Rust port uses software-rendered Slint standard controls, layout-managed
lists and rich text. It contains no Rally canvas. Native scrollbars replace the
old painted virtualization; reference/action/dirty-navigation forms use native
buttons, selectors and editors. Group swimlanes stack vertically, workflow lanes
scroll horizontally, and the assistant docks below at narrow widths. Rich source
is the preservation path for HTML beyond the supported formatting subset.

The Windows suite drives a real mouse drag and native buttons, plus settled
callback-driven scenarios for the other workflows. Its loopback server and
external stdio Codex fixture record every request; no real Rally write is tested.
Attachment network, cleanup, bounds and conflict behavior use HTTP fixtures;
file-dialog invocation is implemented but file-dialog automation is not claimed.
OS-vault service/target compatibility is checked without replacing live tokens.
WSAPI revision preflight is not atomic compare-and-swap. Transfers require the
same scope; retired windows in other scopes retain their recoverable files.

Run `cargo test --locked --lib -- --skip window_runtime::tests` on the Mac, then
`dev/windows-rally-smoke.py` on Windows. The single skipped test requires a GUI;
no Mac GUI was launched. `dev/check-build-policy.py` and `cargo fmt --all --
--check` verify build settings and source formatting. Current acceptance evidence
is described in `docs/PORT-VERIFICATION.md` rather than inherited upstream logs.

## Source evidence

Paths below refer to the audited Go commit, not the new Rust working tree.

- `internal/rally/artifacts.go`: `ArtifactTypes`, `QueryKind`, `canonicalArtifactTypes`, `queryArtifacts`, `isArtifactQuery`.

- `internal/rally/cache.go`: `newPageCache`, `CachedQuery`, `PurgeCache`, `trimFetch`, `pageBytes`, `valueBytes`.

- `internal/rally/catalog.go`: `FindPage`, `StateField`, `States`.

- `internal/rally/client.go`: `lower`, `New`, `resolve`, `requestBytes`, `request`, `QueryValues`, `Query`, `Collection`, `All`, `Get`, `ReferenceKind`, `CurrentUser`, `getFields`, `Create`, `Update`, `mutate`, `mutateQuery`, `Delete`, `Fields`, `Upload`, `Eq`, `Quote`, `And`, `UpdateIfUnchanged`.

- `internal/rally/conflict.go`: `Error`, `SameReference`.

- `internal/rally/position.go`: `UpdatePositionIfUnchanged`.

- `internal/rally/response.go`: `decodePage`, `responseError`.

- `internal/rally/schema.go`: `cloneFields`, `Workflow`.

- `internal/rally/types.go`: `String`, `Ref`, `Number`, `Bool`, `ID`, `Kind`, `Clone`, `cloneValue`, `Count`, `Error`, `CanonicalKind`.

- `internal/assistant/tools.go`: `Specs`, `Execute`, `checkScope`, `sameRef`, `lastID`, `Apply`.

- `internal/ui/assistant.go`: `openAssistant`, `drawAssistant`, `sendAssistant`, `assistantEvent`, `runDynamicTool`.

- `internal/ui/assistant_history.go`: `transcriptText`, `drawAssistantHistory`, `proposalPreview`.

- `internal/ui/board.go`: `prepareCards`, `prepareBoardLayout`, `drawTeamBoard`, `drawBoardCard`.

- `internal/ui/board_display.go`: `snapshot`, `restore`, `matches`, `beginBoardSettings`, `setBoardDisplay`, `applyBoardSettings`, `drawBoardDisplayControls`, `boardMetrics`, `boardScrollAtStride`, `updateBoardStride`, `boardLaneLabel`, `stableBoardColor`, `workItemColor`, `displayColor`, `boardAge`, `ellipsizeBoardTitle`.

- `internal/ui/board_move.go`: `boardGroupIdentity`, `boardGroupChange`, `dropBoardCard`, `priorBoardPosition`, `replaceBoardObject`, `boardWriteBlocked`, `startBoardWrite`, `undoBoardWrite`, `applyBoardPlacements`, `boardWireFields`, `pendingRallyWrite`.

- `internal/ui/detail.go`: `makeDetail`, `isReferenceField`, `richDetailFields`, `leaveDetail`, `disposeDetail`, `openArtifact`, `openArtifactNow`, `newArtifact`, `newArtifactInState`, `startNewDetail`, `drawDetail`, `field`, `syncRich`, `dirty`, `changes`, `changesFor`, `values`, `savedSnapshot`, `acceptSave`, `saveDetail`, `addComment`, `loadCollection`, `loadCollectionPage`, `loadCollectionAt`, `detailCollection`, `selectionPlan`, `deleteArtifact`, `planResultVerb`, `applyPlan`, `mergeSchemaEditors`, `setStates`.

- `internal/ui/detail_collection_editor.go`: `editableCollection`, `collectionObjects`, `encodeCollectionRefs`, `decodeCollectionRefs`, `collectionRefsValue`, `rememberCollectionLabels`, `cancelCollectionEditors`, `collectionsLoading`, `readPropertyCollection`, `loadPropertyCollection`, `collectionObjectValues`, `detailReferenceCollection`.

- `internal/ui/detail_collection_text.go`: `collectionText`, `postInitials`, `postDate`, `prepareDetailCollection`, `collectionTextLayout`, `collectionSkip`, `drawCollectionText`, `drawCollectionRow`, `paintCollectionText`, `deleteDiscussion`.

- `internal/ui/detail_color.go`: `detailColor`.

- `internal/ui/detail_layout.go`: `detailCaption`, `schemaField`, `richFields`, `propertyFields`, `detailPaneWidths`, `detailFields`, `detailRichContent`, `detailMetadata`, `detailStatus`, `detailProperty`.

- `internal/ui/detail_reference.go`: `referenceField`, `originalValue`, `referenceKinds`, `close`, `detailReference`, `openReferencePicker`, `newReferencePicker`, `referenceQuery`, `searchReferences`, `selectReference`, `referenceLabel`, `drawReferencePicker`.

- `internal/ui/detail_reload.go`: `fieldValue`, `setFieldValue`, `mergeReload`, `resolveFieldConflict`, `reloadDetail`, `drawDetailRecovery`.

- `internal/ui/detail_selection.go`: `clone`, `openSelectionEditor`, `selectionStateField`, `rehydrateSelection`, `selectionScopeValid`, `selectionFields`, `selectionValues`, `selectionRows`, `prepareSelection`, `applySelection`, `finishSelection`, `applySelectionNext`, `reloadSelection`, `selectionValueLabel`, `drawSelectionEditor`, `selectionRowText`.

- `internal/ui/detail_tabs.go`: `detailCollectionField`, `detailTabs`, `tabLabel`, `drawDetailTabs`, `detailTabButton`.

- `internal/ui/detail_tasks.go`: `taskTableLayout`, `drawDetailTasks`, `newDetailTask`, `deleteDetailTask`.

- `internal/ui/rally.go`: `newRallyView`, `rallyQuery`, `refreshRally`, `refreshRallyItems`, `rallySignature`, `drawRally`, `drawRallyContent`, `rallyNav`, `searchable`, `filtered`, `table`, `board`, `metrics`, `charts`, `planning`, `timeline`, `index`, `contains`, `remove`, `fallback`, `stateColor`, `quickFilterSignature`, `rallyColumnOptions`, `rallyFieldLabel`.

- `internal/ui/rally_connection.go`: `rallyCredentialError`, `rallyErrorMessage`, `openRallySettings`, `enabledButton`.

- `internal/ui/rally_controls.go`: `drawRallyViewLabel`, `rallyModeChoices`, `drawRallyModes`, `rallyModeButton`, `rallyGroupChoices`, `drawRallyGrouping`, `rallyChipValue`, `rallyFilterLabel`, `activeRallyFilters`, `removeFilter`, `rallyFilterCaption`, `drawRallyFilterChips`, `rallyFilterChip`.

- `internal/ui/rally_filters.go`: `snapshot`, `matches`, `restore`, `newRallyFilterDraft`, `filter`, `closePicker`, `resetFilterDraft`, `rallyFilterField`, `rallyFilterClause`, `structuredFilterExpression`, `structuredFilterSignature`, `addRallyFilter`, `clearRallyFilters`, `chooseRallyFilter`, `drawRallyFilterBuilder`, `drawRallyAdvancedQuery`.

- `internal/ui/rally_inline.go`: `inlineScope`, `inlineField`, `startInline`, `inlineDirty`, `inlineChanges`, `finishInline`, `drawInlineStatus`, `inlineChoices`, `drawInlineCell`, `artifactWebURL`, `tableRowMenu`, `tableTotals`.

- `internal/ui/rally_keyboard.go`: `boardFocusEntries`, `currentBoardFocus`, `moveCardFocus`, `currentRally`, `rallyShortcutApplies`, `runRallyAction`, `closeRallyDetail`, `actionsOverlap`, `validateActionShortcut`, `revealBoardRange`.

- `internal/ui/rally_labels.go`: `rallyKindLabel`.

- `internal/ui/rally_metadata.go`: `loadViewMetadata`, `rehydrateDetail`, `rallyFetch`, `selectItem`, `stateNames`, `stateValue`.

- `internal/ui/rally_presets.go`: `rallyPresetScope`, `prepareRallyPreset`, `currentIteration`, `loadRallyUser`, `applyDefaultOwner`, `pendingDefaultOwner`.

- `internal/ui/rally_refresh.go`: `rallyRefreshState`, `refreshAge`, `drawRallyFreshness`.

- `internal/ui/rally_rows.go`: `rallyRows`, `rallyRowHidden`, `setRallyRowHidden`, `drawRallyRowRestore`, `rallyRow`, `rallyButtonWidth`, `compactRallyButtons`.

- `internal/ui/rally_sort.go`: `compareRally`.

- `internal/ui/rally_table.go`: `prepare`, `tableLink`.

- `internal/ui/rally_types.go`: `rallyTypeMatches`, `queryTypes`, `objectKind`, `metadataFor`, `objectStateValue`, `stateField`, `scheduleMetadata`, `commonRallyMetadata`, `loadRallyTypeMetadata`, `metadataList`, `artifactBadge`.

- `internal/ui/richtext.go`: `newRichEditor`, `ensureEditor`, `cache`, `synchronized`, `sync`, `changed`, `html`, `toggle`, `selectionStart`, `selectionEnd`, `undo`, `richField`, `validRichLink`, `editRichLink`, `paint`, `plainHTML`, `face`, `measure`.

- `internal/ui/saved_view_manager.go`: `savedViewPage`, `changeSavedView`, `reconcileSavedViewNames`, `openSavedView`, `deleteSavedViewDialog`, `savedViewDialog`, `savedViewMenu`, `drawSavedViewActions`, `projectSavedViews`, `prepareSavedViewManager`, `drawSavedViewManager`.

- `internal/ui/saved_views.go`: `savedViewEqual`, `normalizeSavedTimeboxes`, `findSavedView`, `storeSavedView`, `savedView`, `applySavedView`.
