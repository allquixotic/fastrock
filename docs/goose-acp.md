# Embedded Goose in Fastrock 0.4.0

Goose 1.57.0 is pinned at `30bba95105e59bc3e7df624cf6b4d433690cfe12`.
Its source is unchanged. Fastrock calls the public `GooseAcpAgent` and `serve`
APIs on its own Tokio runtime, connected to the UI adapter by a bounded memory
stream. There is no Goose process or HTTP/stdio hop to an external Goose service.
The existing adapter preserves native transcript, permission and queue behavior.

Goose's built-in extensions use its public built-in registry. The Fastrock tools
and vwork MCP adapters use session-scoped loopback routes inside the same process;
the routes call embedded Rust handlers. They never connect to a vwork daemon.
Configured third-party extensions, hooks, AWS credential helpers and user/LLM
tools retain their authorized ability to launch programs.

The provider factory permits `aws_bedrock`, `openai` and `anthropic`. Existing
Goose configuration and credential entries remain authoritative. Codex and remote
agent transports are removed; legacy Codex files are not migrated or deleted.

## Verification and remaining acceptance

The standalone `embedded_goose` test isolates environment/configuration before
starting threads and disables access to the user's Goose keyring. It sends requests
to a local Bedrock Runtime fixture with model IDs `openai.gpt-6.1-sol`,
`openai.gpt-6-luna`, and `us.anthropic.claude-opus-5-5`. These strings test route
preservation; they do not establish AWS model availability or account access.
Working profiles and final model acceptance are validated by the user.

Unchanged Goose currently has these integration limits:

- Bedrock construction reads process-global Goose/AWS configuration. Independent
  concurrent AWS credential/region profiles cannot be implemented by changing
  environment variables around requests; doing that would race. A supported
  per-provider construction API is needed for that degree of profile isolation.
- ACP reasoning configuration accepts Goose's supported effort values. Arbitrary
  custom reasoning strings are rejected by upstream and must not be advertised.
- Goose hooks are command actions; its agent hook manager is private. Fastrock's
  `--vwork-hook` helper evaluates the original vwork policy without opening a UI
  or database. Setup installs a Goose plugin and migrates recognized vwork hook
  commands, preserving custom actions and a first-migration backup.

Search uses immediate local substring matching, then bounded parallel fuzzy
matching. One background worker calls the pinned public `goose-context-management`
compaction API with no tools. It reuses the previous summary for later turns,
chunks large histories, cancels obsolete work and atomically replaces a single
compressed current record in the embedded vwork database. Startup backfills
missing/stale summaries; unchanged summaries do not cause another model call.
Hidden temporary sessions are excluded, and source conversations are never
rewritten by search. Settings → Goose selects the cheap provider/model using
Goose's existing credentials. Missing configuration or a provider failure preserves
local search and never triggers an expensive fallback.

The transcript adapter stores older items in lossless LZ4 pages and retains at
most 16 decoded pages within an 8 MiB estimated allocation budget. Oversized items
are restored without admission to that cache. Metadata and item ordinals support
direct page lookup; scrolling no longer clones the whole conversation for each
request. Older-page cursors retain their position when streamed items arrive.
The Slint transcript retains its existing bounded render window and asynchronous
history loader. This adapts Ghostty's cold-page idea to portable GUI data rather
than relying on terminal page mappings. Original Goose sessions remain intact.

The native sidebar/search/footer and active-title header follow T3 Code 0.1.5.
Rally has a permanent sidebar entry and a separate document strip; returning to
Rally selects the last document. Open tabs still exposes child conversations and
files. This is the navigation foundation, not complete T3 feature parity.

Graphical provider/credential setup, repository/workspace binding, full T3 orchestration/UI,
large-history UI/performance acceptance and tray lifecycle
remain tracked in `SPEC.md`. This development foundation is not a completed 0.4.0
release. Historical 0.3.x transport details follow for old releases only.

---

# Goose ACP in Fastrock

Fastrock 0.3 uses an installed `goose acp` process by default. One child serves all
conversations in that Fastrock window. Requests have independent JSON-RPC IDs;
streamed updates, permissions, cancellation, and pending tool calls are routed by
ACP session ID. Different windows own different children. Closing Fastrock closes
its child, while Goose retains ordinary conversations in its own session store.

## Why stdio ACP

The architecture was checked against Goose **v1.56.0** (commit
`9949cf926cd42f1d128b9991ed23a496ecace896`, released 2026-10-23), not inferred
from the protocol name. Goose stores multiple active sessions and starts prompt
handlers independently. Its cancel handler identifies the session. A long prompt
therefore does not serialize other conversations in the same process.

Sources: [Goose session and run management](https://github.com/aaif-goose/goose/blob/v1.56.0/crates/goose/src/acp/server.rs),
[concurrent ACP dispatch](https://github.com/aaif-goose/goose/blob/v1.56.0/crates/goose/src/acp/server/dispatch.rs),
and [Goose protocol extension schema](https://github.com/aaif-goose/goose/blob/v1.56.0/crates/goose/acp-schema.json).

`goose serve` exposes the same ACP handler over HTTP/WebSocket. It does not provide
a separate multi-conversation API that stdio lacks. Stdio gives Fastrock a child
lifecycle and needs no daemon authentication or listening agent port. A future
remote-Goose connection can use the HTTP transport without embedding Goose.
The current WebSocket/daemon connection choices are **Codex** connections.

No Goose crate or executable is bundled. Existing Codex protocol/config utility
crates remain as the native UI's presentation types and for optional Codex support;
they do not run the agent. The adapter translates ACP into these existing types.
Axum, already in the dependency graph, is used for the small Fastrock tool bridge.
Updating Goose does not require rebuilding Fastrock for new models, option values,
provider fixes or agent-side tools. New client UI capabilities or breaking changes
to Goose's unstable extensions can still require a Fastrock update.

## Behavior

- Create, list, search, reopen and stream conversations; replay persisted messages,
  thoughts, plans and tool results into the native transcript. ACP load replays the
  whole history before Fastrock presents pages to the existing UI.
- Concurrent prompts use one process. Prompt RPCs have no short completion timeout.
  Other requests have a bounded timeout. Stop and permissions apply to one session.
- Provider, model, mode and thinking choices come from `configOptions`, including
  grouped choices and values unknown when Fastrock was built. Changes apply to the
  selected idle session. Global provider credentials/extensions belong to Goose.
- ACP permission options appear in the native approval UI. Only an exact offered
  option is returned; dismissing or supplying an unrecognized answer cancels it.
  Goose owns tool execution. ACP permission mode is not an operating-system sandbox.
- Queued messages can be edited or deleted before dispatch. Draft queues are saved
  atomically under Fastrock's `goose-queues` directory (inside `FASTROCK_HOME` when
  set). Restored queues are visible when their session is reopened; they do not
  automatically resume after a restart. A `*.last-dispatch.json` recovery record
  retains the latest attempted submission; check the transcript before resending it.
- Rename, archive, unarchive, compact and in-flight steering use Goose extensions.
  Unsupported methods return a visible error. Session IDs are prefixed `goose:`
  inside Fastrock so a Codex conversation cannot be sent to Goose accidentally.

Rally's assistant uses a temporary hidden Goose session with only the Fastrock
MCP tools enabled. Fastrock verifies that restricted extension set and appends the
existing Rally instructions before prompting. It deletes the hidden session when
closed. The MCP listener binds loopback, checks Host/Origin and uses an unguessable
session-specific route. It never holds Rally credentials. Tool calls pass through
the existing native handlers: proposed changes still require the user's Apply.
Namespaced cross-tab tools are translated to MCP names and routed back to their
original namespace. They remain governed by the existing per-tab/user controls.

Fastrock 0.3.3 can replace eligible vwork stdio extensions with a shared localhost
MCP route, eliminating per-conversation vwork client processes. vwork 0.3.3 remains
the sole database owner. See [shared transport and database compaction](VIRTUAL_WORKSPACES.md).

## Current boundaries

Goose owns authentication and global configuration; Codex account, provider and
sandbox settings are hidden on the Goose connection. Configure Goose before
starting Fastrock. Inherited environment configuration follows the installed CLI.
No automatic CLI installation, update, or credential migration occurs.

Codex-specific features such as constrained JSON recap/purpose inference, review
RPCs, turn-boundary forks/side chats, rollback, Codex projects/sections, realtime
voice and server terminal APIs are not emulated. Background purpose inference is
disabled for Goose. Invoking an unsupported action returns an error; choose the
Codex connection for those workflows. Existing Codex data is not converted into
Goose history. Unknown ACP notifications are tolerated, while new client-side
methods return method-not-found rather than pretending to execute them.

## Verification

`src/goose/tests.rs` exercises a separate Python ACP process: simultaneous sessions,
isolated cancellation and permission outcomes, history replay, configuration values,
queue edits/persistence, namespaced MCP routing and disconnect handling. The full
Rust suite runs on Windows. `dev/windows-goose-smoke.py` exercises the native Rally
workflow through Goose ACP plus local HTTP/MCP fixtures. It performs no inference
or production Rally writes. Signed CI packages run both Goose and Codex Rally
fixtures on the exact Windows release executable. macOS only compiles and signs.
These fixtures do not validate a live provider account or make claims about its
availability, speed or model output quality.
