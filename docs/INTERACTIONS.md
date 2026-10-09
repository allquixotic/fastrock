# Codex GUI interaction audit

Reference: the Slint/Rust `codex-rs/gui/ui` and `codex-rs/gui/src` tree in the
adjacent Codex checkout, reviewed 2026-10-09. The inventory includes the public
callbacks in app, composer, sidebar, newtab, approvals, transcript, files_state,
info, settings_state, settings_keyboard, settings_sandbox, bedrock and xtab.
Repeated callbacks in shared widget components map to their owning action.

“Implemented” means an action reaches native state, the installed Codex protocol,
or the relevant OS operation. It does not mean every source widget has identical
geometry or that every server-dependent action was exercised against real accounts.

| Reference surface | Fastrock implementation | Coverage / remaining differences |
| --- | --- | --- |
| App menus and shortcuts | `menu.go`, `interactions.go`, `actions.go` | File/View/Thread/Help; tab navigation, new/close/open, sidebar/info, settings, theme, configurable bindings, conflict/reset/unbind. |
| Document strip | `app.go`, `theme.go`, `popout.go` | Integrated close target, middle close, drag reorder, horizontal overflow, context close/others/right, native pop-out and move to existing window. |
| New tab and folders | `chat.go`, `session.go` | Native folder/file chooser, folderless chat, recent folders, forget/copy/open, resume and account sign-in. Native OS chooser replaces custom Slint picker geometry. |
| Sidebar | `chat.go`, `interactions.go` | Project grouping/collapse, search, archived toggle, refresh/pagination, thread/folder menus, rename/fork/side/worktree/recap/compact/review/init/export/copy/archive/unarchive/delete. |
| Composer | `chat.go`, `suggestions.go`, `clipboard.go`, `slash.go` | Per-chat drafts, model/effort/speed, plan, send/steer/interrupt, queue edit/reorder/send/delete, attachments and image paste, slash/file suggestions. Caret-local slash/file/skill completion, cancellation and explicit skill protocol input. Suggestion presentation differs from Slint. |
| Transcript | `transcript.go`, `chat.go` | Streaming, bounded history/layout, load older, expand/collapse, top/latest, message/selection copy/quote/forward, text view, external/file links. CommonMark/GFM styles and tables, contextual links, direct formatted-text drag selection, Ctrl/Cmd+A/C and Escape. Plain-text selection mode is also available. |
| File and diff documents | `files.go`, `diff.go` | Read-only paged file, find next/previous, go to line, wrap, path/reveal/external/reload/copy/save; per-file diff collapse/open/text view. Added/removed/hunk colors, project-relative file opening and line/column links. Full intraline diff emphasis remains open. |
| Information panel | `info.go`, `xtab.go` | Usage/status, goals and budgets, pause/resume/clear, terminals, agent hierarchy/opening, peers and mailbox. Terminal cards show commands/folders, poll while active, page in the background, expose copy/details/folder context actions, stop/stopping and confirmed stop-all. |
| Cross-tab messaging | `xtab.go` | Native delivery approval, scoped disable, mailbox, size/rate/wait limits, queue delivery, reply correlation. Open-chat discovery spans native windows; mailbox follows moved tabs, and reply capture observes the matching turn without cloning another window’s transcript. |
| Approval cards | `approvals.go` | Active-thread routing, server decision lists and policy amendments, turn/session/strict permission grants, input forms, URL elicitation, copy/open, explicit focus/arming for choice hotkeys. Primitive schema defaults, bounds, date/URI/email validation, labeled single/multiselect, optional omission and choice-plus-note encoding. Detailed source-card layout differs. |
| Account and provider settings | `settings_extra.go`, `providers.go` | ChatGPT/device/API-key login/logout/cancel, usage, Bedrock environment/profile/API token, local model discovery/pull/cancel/use, shared restart. Mantle/Runtime, access keys, profile discovery and GovCloud checks are available. Region choices and credential validation feedback have a simpler layout. |
| Model configuration | `settings.go`, `configuration.go`, `config_schema.go` | Catalog and independent speed gating, typed configuration edits, reset/search, raw TOML validation/conflict detection/atomic save. Context/layer selection, source files, origins and read-only overrides; field help, defaults and enum choices from a credited reference-schema snapshot. Runtime values and unknown keys come from the installed CLI; schema updates require refreshing the snapshot. |
| MCP, skills, hooks and plugins | `settings_extra.go` | MCP add/remove/enable/OAuth/reload; skills enable/source; hook enable/trust/source; plugin install/uninstall. Marketplace browsing/install/read/toggle/uninstall and separate MCP transport/environment/header forms. Marketplace presentation differs. |
| Other settings | `settings_extra.go`, `configuration.go` | Queue/Steer default, Ctrl/Cmd+Enter, future-thread messaging toggle, feature enablement, memory reset, import detection/selection, feedback, sandbox setup, diagnostics, keyboard customization. Some screens expose structured server data where the reference has dedicated cards. |
| Server lifecycle | `broker.go`, `client.go`, `session.go` | Installed CLI verification, one shared app-server, restart/reconnect, sequencing/ownership, errors. Embedded Rust server and remote daemon connection controls are intentionally excluded by the installed-local-CLI requirement. |
| Theme and geometry | `theme.go`, `fonts.go` | Slint color tokens, dark/light, installed UI fonts with bundled fallback, thin borders and compact radii. Full pixel parity at all sizes/DPI remains open. |

## Validation boundary

Windows fixture tests cover chat/Rally streaming, filters, rich edits, tab lifecycle,
keyboard and native dialog flows; the large-board test also covers restart and
multi-window draft preservation. Unit tests cover core state/protocol/cache rules.
macOS is cross-built and tested headlessly only. Account/provider writes requiring
real credentials, production Rally writes and paid inference are not test fixtures.
The remaining differences above are tracked work, not completed parity claims.
Desktop notification delivery, automatic system-theme following and the reference
renderer-choice UI are also not ported; Fastrock uses its single pure-Go
rendering backend. Dark/light overrides are available.
