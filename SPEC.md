# Fastrock Specification

Status: initial product and engineering spec
Date: 2026-06-15

## 1. Goal

Fastrock is a native, high-performance Rust GUI coding-agent app for macOS 26,
Windows 11 25H2, and Ubuntu 26.04 LTS. It clones the practical agentic coding
workflow of Cmon Roo Code while making AWS Bedrock, especially Bedrock Mantle,
the first-class default provider path.

Fastrock must feel closer to a fast local tool than a SaaS product. It must not
show signup funnels, telemetry prompts, or routine permission confirmations.
The default posture is autonomous local execution through `rtk`, with user
control exposed as settings, not as blocking modal prompts.

## 2. Product Principles

1. Responsiveness is the top product requirement. UI input, scrolling, typing,
   conversation switching, file viewing, and cancellation must stay responsive
   while models stream, commands run, remote files sync, or indexes update.
2. AWS Bedrock support is not a plugin-class provider. Bedrock Mantle and
   Bedrock Runtime are core product surfaces with dedicated settings, diagnostics,
   test fixtures, and stream parsers.
3. No cloud dependency owned by Fastrock. Fastrock may call user-configured
   provider APIs, AWS, SSH hosts, Session Manager, MCP servers, and local tools,
   but it must not require a Fastrock account or third-party Fastrock backend.
4. External `rtk` is required for all shell command execution. Fastrock must not
   vendor, reimplement, or natively link RTK behavior.
5. Parallel conversations are normal. Every conversation owns its model profile,
   project folder, remote target, command queue, memory/cache context, goal
   state, and transcript stream.
6. First implementation scope is full platform. Do not stage a toy MVP that
   omits remote projects, MCP, skills, editor tabs, goal loop, or Bedrock
   Mantle. Individual tasks can land incrementally, but the initial architecture
   must include the full product.

## 3. Non-Goals

- No hosted Fastrock service, billing plane, login, telemetry, crash upload, or
  cloud sync.
- No VS Code extension dependency.
- No Electron, webview app shell, or browser-based GUI runtime.
- No local model runtime in v1. Local models may be added later through a
  provider abstraction, but v1 optimizes for AWS Bedrock.
- No full IDE. The editor is a fast file viewer/editor for agent work, not a
  replacement for JetBrains, VS Code, Zed, or Vim.
- No mandatory safety confirmation layer. Dangerous defaults can be changed in
  settings, but baseline execution is non-blocking.

## 4. Target Platforms and Toolchain

Fastrock targets latest stable Rust and latest stable OS releases:

- macOS 26 on Apple Silicon first; Intel support only if Slint and Rust stable
  keep it practical.
- Windows 11 25H2.
- Ubuntu 26.04 LTS with Wayland primary and X11 fallback where Slint supports it.

Build requirements:

- Rust latest stable.
- Cargo workspace.
- Slint latest stable Rust bindings for GUI.
- Tokio multi-thread runtime for background async work.
- SQLite through a Rust library that supports WAL and async-friendly use, such
  as `sqlx` with blocking work isolated, or `rusqlite` behind a dedicated DB
  actor.
- OS keyring crate for secrets.
- Tree-sitter for syntax highlighting.
- Rope-backed text buffer, preferably `ropey` unless benchmarks prove another
  rope better.
- AWS SigV4 implementation from maintained Rust AWS crates where feasible.
- AWS shared config/credential resolution compatible with AWS CLI named
  profiles, including `~/.aws/config`, `~/.aws/credentials`, `AWS_PROFILE`,
  SSO, assume-role, `credential_process`, environment credentials, and the
  default provider chain.

## 5. External References

These references are requirements inputs, not code dependencies:

- Bedrock Mantle Responses API:
  https://docs.aws.amazon.com/bedrock/latest/userguide/bedrock-mantle.html
- Bedrock endpoints:
  https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints.html
- Bedrock Mantle quotas:
  https://docs.aws.amazon.com/bedrock/latest/userguide/quotas-mantle.html
- Bedrock Mantle CloudTrail:
  https://docs.aws.amazon.com/bedrock/latest/userguide/logging-cloudtrail-mantle.html
- Bedrock Projects:
  https://docs.aws.amazon.com/bedrock/latest/userguide/projects.html
- Slint Rust docs:
  https://docs.slint.dev/latest/docs/rust/slint/
- RTK README:
  https://github.com/rtk-ai/rtk/blob/develop/README.md

Local inspiration repositories under `inspiration/` are design references only.
Fastrock must not depend on them at build or runtime.

## 6. High-Level Architecture

Fastrock is a Cargo workspace with clear crate boundaries:

- `fastrock-app`: binary, process startup, OS integration, Slint window setup,
  Tokio runtime startup, logging, panic handling, and global shutdown.
- `fastrock-ui`: Slint `.slint` files, generated Rust bridge, UI state adapters,
  and pure presentation models.
- `fastrock-core`: conversation state machine, agent loop, mode handling,
  tool routing, context assembly, transcript events, cancellation, and goal loop.
- `fastrock-bedrock`: Bedrock Mantle and Bedrock Runtime clients, auth, request
  conversion, stream parsing, model discovery, quota diagnostics, and tests.
- `fastrock-rtk`: external `rtk` command runner, PTY/non-PTY process handling,
  output streaming, output truncation, exit status, and cancellation.
- `fastrock-projects`: project folders, local filesystem abstraction, SSH remote
  transport, AWS Session Manager transport, remote command execution, remote file
  sync, and project metadata.
- `fastrock-persistence`: SQLite schema, migrations, event store, settings
  files, OS keyring secret references, import/export, and backup/repair.
- `fastrock-mcp`: MCP config, stdio and streamable HTTP clients, OAuth support,
  server lifecycle, tool/resource discovery, and tool invocation.
- `fastrock-skills`: skill discovery, frontmatter validation, global/project
  scope, mode restrictions, file watching, and prompt injection.
- `fastrock-cache`: prompt cache policy, model-aware cache point planning,
  token/cache accounting, local artifact cache, and deduped tool-output cache.
- `fastrock-editor`: rope buffer, syntax highlighting, viewport model, file
  load/save, diff, find, tabs, and editor commands.
- `fastrock-remote`: shared lower-level remote process/file primitives if they
  grow beyond `fastrock-projects`.
- `fastrock-test-support`: fake providers, fake streams, temp workspaces, mocked
  remotes, and UI harness helpers.

Crates must communicate with typed commands and events. UI code must not hold
provider clients, subprocess handles, database connections, or remote sockets.

## 7. Runtime and Threading Model

Slint owns the main thread. The main thread may update Slint models and handle
input only. It must never block on network, disk, subprocess, SSH, SSM, SQLite,
indexing, parsing, or model streaming.

Process startup:

1. `main()` initializes logging and crash-safe panic handling.
2. `main()` creates a Tokio multi-thread runtime on background worker threads.
3. `main()` creates the Slint main window and lightweight UI state.
4. UI events enqueue typed `AppCommand` values into bounded async channels.
5. Background actors handle commands and send `AppEvent` values back.
6. UI updates enter Slint through `slint::invoke_from_event_loop` or equivalent
   Slint-safe bridge. The callback must only touch UI state and return quickly.

Actor boundaries:

- One app supervisor actor.
- One persistence actor.
- One settings actor or settings service behind persistence.
- One conversation actor per loaded conversation.
- One command runner actor per active project target.
- One MCP supervisor plus one actor per connected MCP server.
- One remote target actor per SSH/SSM target.
- One editor file actor per open large file or per tab group, depending on
  profiling.

Channels:

- Use bounded `tokio::sync::mpsc` for high-volume streams.
- Use `watch` or `broadcast` only for low-volume state snapshots.
- Every event that can flood UI, such as token deltas or command output, must be
  coalesced before reaching Slint.
- Cancellation uses `CancellationToken` passed through each long-running task.

Blocking work:

- SQLite writes run through a DB actor.
- Filesystem traversal, tree-sitter parsing, git operations, keyring calls, and
  any sync library APIs run in dedicated blocking tasks or dedicated actors.
- Large file reads stream chunks and build the rope incrementally.

Shutdown:

- Stop accepting new commands.
- Cancel active model streams and subprocesses.
- Flush transcript/event store.
- Close MCP and remote sessions.
- Persist UI/session metadata.
- Drop Slint window last.

## 8. UI Model

The primary UI is a three-region app:

- Left sidebar: conversation list grouped/filterable by project folder and
  remote target. Each row shows title, folder, model profile, status, active
  goal state, and last update.
- Center panel: active conversation transcript with streaming model text,
  reasoning/tool sections, command output, file changes, checkpoints, and
  compact status.
- Right/editor area: tab-driven file viewer/editor, diff views, settings panes,
  MCP/skills panels, and project/remote diagnostics. The user can hide this
  area for chat-focused work.

Expected first-tier workflows:

- Create conversation in a local folder.
- Create conversation in SSH folder.
- Create conversation in AWS Session Manager folder.
- Switch between active conversations without stopping them.
- Edit model profiles while preserving unsaved settings until Save.
- Manage Bedrock profiles with test connection and model discovery.
- Manage MCP servers and skills from GUI.
- Open and edit files from agent output in tabs.
- Set, pause, resume, complete, or clear a conversation goal.
- Toggle plan mode per conversation.
- View cache, token, command, and quota diagnostics.

UI must avoid modal interruptions for normal agent operation. Error surfaces are
inline banners, status chips, logs, and retry controls.

## 9. Core Data Model

All persisted IDs are stable UUIDv7 or ULID strings. Timestamps are UTC
milliseconds since Unix epoch unless a provider wire format requires otherwise.

### 9.1 LlmProfile

```rust
struct LlmProfile {
    id: ProfileId,
    name: String,
    provider: LlmProvider,
    model_id: String,
    enabled: bool,
    default_for_new_conversations: bool,
    auth_ref: SecretRef,
    request: RequestTuning,
    bedrock: Option<BedrockProfile>,
    cache: CacheProfile,
    tool_policy: ToolPolicy,
    created_at_ms: i64,
    updated_at_ms: i64,
}

enum LlmProvider {
    BedrockMantle,
    BedrockRuntime,
}

struct RequestTuning {
    max_output_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    reasoning_effort: Option<ReasoningEffort>,
    timeout_ms: Option<u64>,
    retry: RetryPolicy,
}
```

### 9.2 BedrockProfile

```rust
struct BedrockProfile {
    region: String,
    endpoint_override: Option<String>,
    credential_source: AwsCredentialSource,
    mantle: Option<MantleSettings>,
    runtime: Option<RuntimeSettings>,
}

enum AwsCredentialSource {
    DefaultChain,
    CliProfile { profile_name: String },
    Environment,
    ExplicitStaticSecret(SecretRef),
}

struct MantleSettings {
    project_id: Option<String>,
    store_default: bool,
    api_shape: MantleApiShape,
    auth_mode: MantleAuthMode,
    use_background: bool,
}

enum MantleApiShape {
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

enum MantleAuthMode {
    BearerApiKey,
    AwsSigV4,
}

struct RuntimeSettings {
    invoke_target: BedrockInvokeTarget,
    use_converse_stream: bool,
    service_tier: Option<String>,
    prompt_cache_ttl: Option<CachePointTtl>,
}

enum BedrockInvokeTarget {
    FoundationModel(String),
    InferenceProfileArn(String),
    ApplicationInferenceProfileArn(String),
    PromptRouterArn(String),
    CustomArn(String),
}
```

Mantle `store_default` is retained in the data model for wire compatibility but
is ignored at runtime: Fastrock always sends `store=false` so AWS never retains
response state (see §V5). There is no store toggle in the GUI and no
per-conversation override; conversation state is local-only.

`credential_source` is shared by all AWS-signed Bedrock calls made by the
profile. `DefaultChain` follows the AWS SDK default chain. `CliProfile` loads a
named AWS CLI profile exactly as the CLI would when `AWS_PROFILE=<name>` is set,
including profile region fallback, SSO, assume-role, and `credential_process`
where the AWS SDK supports them. `Environment` uses only process environment
credentials and region settings. `ExplicitStaticSecret` is for stored access key
material referenced through OS keyring and should be discouraged except for
exploration. Mantle bearer API keys are not an `AwsCredentialSource`; they are
selected by `MantleAuthMode::BearerApiKey` and stored through `auth_ref`.

### 9.3 Conversation

```rust
struct Conversation {
    id: ConversationId,
    title: Option<String>,
    project_folder_id: ProjectFolderId,
    profile_id: ProfileId,
    mode: AgentMode,
    status: ConversationStatus,
    approval_policy: ApprovalPolicy,
    transcript_head: EventId,
    active_goal_id: Option<GoalId>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

enum ConversationStatus {
    Idle,
    Running,
    WaitingForModel,
    WaitingForTool,
    Cancelling,
    Paused,
    Failed,
}

enum AgentMode {
    Code,
    Architect,
    Ask,
    Debug,
    Plan,
    Custom(String),
}

enum ApprovalPolicy {
    Never,
    OnFailure,
    OnRequest,
    Granular(GranularApprovalPolicy),
}
```

Default `approval_policy` is `Never`.

### 9.4 ThreadGoal

```rust
struct ThreadGoal {
    id: GoalId,
    conversation_id: ConversationId,
    objective: String,
    status: ThreadGoalStatus,
    token_budget: Option<i64>,
    tokens_used: i64,
    time_used_seconds: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

enum ThreadGoalStatus {
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}
```

Goal loop behavior:

- Active goal automatically continues when conversation becomes idle.
- Plan mode turns do not count against goal token budget unless explicitly run
  as execution.
- Usage-limit provider errors move active goal to `UsageLimited`.
- Tool/model errors that block progress move active goal to `Blocked`.
- Budget crossing moves active goal to `BudgetLimited`.
- User can resume `Paused`, `Blocked`, `UsageLimited`, or `BudgetLimited` goals.
- Goal progress accounting charges token deltas and wall-clock deltas exactly
  once per turn.

### 9.5 ProjectFolder and RemoteTarget

```rust
struct ProjectFolder {
    id: ProjectFolderId,
    label: String,
    path: String,
    target: ProjectTarget,
    default_profile_id: Option<ProfileId>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

enum ProjectTarget {
    Local,
    Remote(RemoteTargetId),
}

struct RemoteTarget {
    id: RemoteTargetId,
    name: String,
    kind: RemoteKind,
    host_label: String,
    auth_ref: Option<SecretRef>,
    default_shell: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

enum RemoteKind {
    Ssh(SshTarget),
    AwsSessionManager(SsmTarget),
}
```

Remote folders are tagged with their remote target. A remote project path must
never be treated as a local path.

### 9.6 RtkCommandRequest

```rust
struct RtkCommandRequest {
    id: CommandId,
    conversation_id: ConversationId,
    project_folder_id: ProjectFolderId,
    cwd: String,
    command: Vec<String>,
    shell: Option<String>,
    pty: bool,
    timeout_ms: Option<u64>,
    env: BTreeMap<String, String>,
    stdin: Option<Vec<u8>>,
}
```

Fastrock must execute shell commands by invoking external `rtk`:

- Local non-shell command: `rtk <program> <args...>`.
- Local shell command: `rtk <shell> -lc <command>`.
- Remote command: run remote shell such that remote side invokes `rtk` if
  available. If remote `rtk` is missing, command must fail with install guidance
  unless profile explicitly allows uncompressed fallback.
- Command output streams through Fastrock with IDs, timestamps, stdout/stderr
  separation, byte counts, token-savings metadata when `rtk` reports it, and
  truncation markers.

### 9.7 MCP

```rust
struct McpServerConfig {
    id: McpServerId,
    name: String,
    scope: ConfigScope,
    enabled: bool,
    transport: McpTransportConfig,
    timeout_ms: u64,
    always_allow_tools: BTreeSet<String>,
    disabled_tools: BTreeSet<String>,
}

enum McpTransportConfig {
    Stdio { command: String, args: Vec<String>, cwd: Option<String>, env: BTreeMap<String, String> },
    StreamableHttp { url: String, headers: BTreeMap<String, String> },
}
```

MCP must support stdio and streamable HTTP in v1. OAuth 2.1 with PKCE is required
for HTTP servers that ask for it. Tokens are stored in OS keyring.

### 9.8 Skills

```rust
struct SkillManifest {
    name: String,
    description: String,
    path: String,
    scope: ConfigScope,
    mode_slugs: Vec<String>,
}

enum ConfigScope {
    Global,
    Project(ProjectFolderId),
}
```

Skill discovery:

- Global: `~/.fastrock/skills/<name>/SKILL.md`.
- Project: `<project>/.fastrock/skills/<name>/SKILL.md`.
- Compatibility import paths may read `.agents/skills` and `.roo/skills`, but
  Fastrock writes only `.fastrock/skills`.
- `name` must match parent directory.
- `description` must be non-empty and at most 1024 chars.
- Project skill overrides global skill of same name.
- Mode-specific skill overrides generic skill of same name at same scope.

### 9.9 EditorBuffer

```rust
struct EditorBuffer {
    id: BufferId,
    project_folder_id: ProjectFolderId,
    path: String,
    language_id: Option<String>,
    dirty: bool,
    read_only: bool,
    version: u64,
    encoding: TextEncoding,
    line_count: usize,
}
```

Large files are allowed. Rendering must be virtualized by visible line range.

### 9.10 Commands and Events

Core command enum:

```rust
enum ConversationCommand {
    UserMessage { text: String, attachments: Vec<AttachmentRef> },
    StartPlan { prompt: String },
    ExecutePlan { plan_id: PlanId },
    CancelTurn,
    RunTool { call: ToolCall },
    SetMode { mode: AgentMode },
    SetGoal { objective: String, token_budget: Option<i64> },
    UpdateGoalStatus { status: ThreadGoalStatus },
    ClearGoal,
}
```

Core event enum:

```rust
enum ConversationEvent {
    UserMessageAppended,
    ModelStreamStarted,
    ModelTextDelta { item_id: ItemId, delta: String },
    ReasoningDelta { item_id: ItemId, delta: String },
    ToolCallStarted { call: ToolCall },
    ToolCallDelta { call_id: ToolCallId, delta: ToolDelta },
    ToolCallFinished { call_id: ToolCallId, result: ToolResult },
    CommandOutputDelta { command_id: CommandId, stream: OutputStream, bytes: Vec<u8> },
    FileChanged { path: String, diff_summary: DiffSummary },
    GoalUpdated { goal: ThreadGoal },
    TurnFinished { usage: TokenUsage },
    Error { error: FastrockError },
}
```

Events are append-only in persistence. UI state is derived from events plus
small materialized indexes.

## 10. Bedrock Mantle Support

Mantle endpoint format:

```text
https://bedrock-mantle.{region}.api.aws/v1
```

Required Mantle features:

- `GET /v1/models` discovery.
- `POST /v1/responses` streaming and non-streaming.
- `GET /v1/responses/{id}` where supported by stored responses.
- `store=false` on every request; AWS retains no response state.
- No `previous_response_id` conversation chaining (chaining requires server-side
  stored state, which Fastrock never enables).
- `OpenAI-Project` header or documented project selection mechanism when a
  project is configured.
- Bearer API key auth for OpenAI-compatible SDK behavior.
- Direct HTTPS with AWS SigV4 auth for users who do not want Bedrock API keys.
- AWS CLI named profile support for SigV4 Mantle requests. A Mantle profile
  using `AwsSigV4` must be able to select `CliProfile { profile_name }`, resolve
  that profile through shared AWS config files, honor profile region fallback
  unless the Fastrock profile region overrides it, and produce signed HTTP
  requests to `bedrock-mantle.{region}.api.aws`.
- `AWS_PROFILE` parity for diagnostics: selecting `CliProfile { profile_name }`
  in the GUI must behave like launching a CLI process with
  `AWS_PROFILE=profile_name` for supported AWS SDK credential mechanisms.
- Request/response logging redaction.
- CloudTrail diagnostics showing Mantle calls use `bedrock-mantle.amazonaws.com`.

Mantle API shape policy:

- Responses API is preferred for models that support it.
- Chat Completions API is fallback for Mantle models that do not support
  Responses API.
- Anthropic Messages API is optional but should be implemented because Mantle
  exposes it and Anthropic-compatible workflows are common.
- Model discovery must record API compatibility per model, not assume all models
  support all Mantle APIs.

Mantle quotas:

- Track separate input TPM and output TPM where exposed.
- No RPM quota assumption for Mantle. Throttling strategy keys off token quotas
  and 429 responses.
- Cached input tokens read through prompt caching do not count against Mantle
  input TPM.
- Runtime and Mantle quotas are independent. Diagnostics must show endpoint
  distinction.

Mantle error handling:

- 401/403: auth/project/IAM guidance.
- 404 model: refresh models and show region/profile mismatch.
- 409/422 incompatible request: show API shape/model compatibility issue.
- 429: token quota/backoff guidance with endpoint/model/region.
- Stream disconnect: retry only if request is idempotent or provider supports
  safe resume; otherwise preserve partial transcript and ask agent to continue.

## 11. Bedrock Runtime Support

Runtime endpoint format:

```text
https://bedrock-runtime.{region}.amazonaws.com
```

Required Runtime features:

- Converse and ConverseStream primary.
- InvokeModel and InvokeModelWithResponseStream fallback for models or features
  unavailable through Converse.
- Foundation model IDs.
- System inference profiles.
- Application inference profiles.
- Custom ARNs.
- Prompt routers where available.
- Service tiers where model supports them.
- Bedrock prompt cache points and TTL.
- Reasoning/thinking budget mapping for models that support it.
- Model capability discovery and manual override.
- AWS CLI named profile support, environment credentials, SSO, assume-role,
  `credential_process`, and default credential chain support through the shared
  `AwsCredentialSource` model. Runtime SDK clients must use the same profile
  selection and region fallback behavior as Mantle SigV4 clients.

Runtime request conversion:

- Internal conversation format converts to Bedrock messages, system blocks,
  tools, tool results, image blocks, and cache points.
- Tool schemas are normalized for Bedrock compatibility.
- Strict JSON-schema features unsupported by Bedrock are stripped or rejected
  with precise diagnostics.
- Usage parser must preserve input tokens, output tokens, cache read/write
  tokens, latency, stop reason, invoked model ID, and trace metadata.

## 12. LLM Settings GUI

Settings must be robust enough to replace hand-edited config for most users.

Profile list:

- Add, duplicate, remove, rename.
- Enable/disable.
- Mark default.
- Search/filter by provider/model/region.
- Show validation status.

Bedrock Mantle settings:

- Region.
- Endpoint override.
- Auth mode: API key or AWS SigV4.
- API key secret picker/create/update/delete.
- AWS credential source selector for SigV4: default chain, named AWS CLI
  profile, environment-only, or explicit static secret.
- AWS CLI profile picker for SigV4 that discovers local shared config profile
  names without reading secret values into UI state.
- AWS credential source preview for SigV4 showing resolved source type, profile
  name, region source, account/identity when safely probeable, and expiration
  when available.
- Project ID/name.
- API shape: Responses, Chat Completions, Anthropic Messages, Auto.
- Background/async inference toggle where supported.
- Model discovery button.
- Test connection button.
- Quota diagnostics.
- CloudTrail note and event source.

Bedrock Runtime settings:

- Region.
- Endpoint override.
- AWS credential source selector: default chain, named AWS CLI profile,
  environment-only, or explicit static secret.
- AWS CLI profile picker that uses the same shared config profile discovery as
  Mantle SigV4.
- AWS credential source preview showing resolved source type, profile name,
  region source, account/identity when safely probeable, and expiration when
  available.
- Target type selector.
- Model/profile/ARN input.
- Discovery button.
- Service tier selector.
- Prompt cache TTL.
- Max output token probe where practical.
- Reasoning budget/effort.
- Test connection button.

All settings forms must maintain local unsaved state until Save. Navigation away
from settings must not silently discard edits.

## 13. Agent Modes

Built-in modes:

- Code: implement code, run tests, edit files, use tools freely.
- Architect: design, inspect, propose plans/specs, avoid mutations unless user
  requests execution.
- Ask: answer questions, inspect files, no mutations by default.
- Debug: reproduce, inspect logs, add targeted instrumentation if needed.
- Plan: read-only planning, no repo-tracked edits, no side-effectful execution.

Custom modes:

- User-defined slug, display name, role definition, instructions, tool groups,
  file restrictions, and default profile.
- Stored globally or per project.
- GUI supports add/edit/delete/import/export.

Mode selection affects:

- System prompt sections.
- Skill availability.
- Tool list.
- Default approval policy, though global default remains `Never`.
- File restrictions.
- Goal loop eligibility.

## 14. Autonomous Goal Loop

The goal loop is native in `fastrock-core`.

Goal loop requirements:

- Single active goal per conversation.
- Goal can be created, updated, paused, resumed, completed, blocked, budget
  limited, usage limited, or cleared.
- Goal state persists in SQLite and survives app restart.
- Active goal resumes only when user requests resume or when settings allow
  automatic continuation after app restart.
- Each loop iteration is a normal conversation turn with a synthetic internal
  goal-continuation context item.
- Goal objective changes during an active turn inject a steering item.
- Token budget and elapsed time are displayed in UI.
- Budget-limited status stops automatic continuation.
- User interrupt stops current turn and leaves goal active or paused based on UI
  action.

Termination hierarchy:

1. User marks complete.
2. Agent marks complete through goal status update.
3. Token budget exceeded.
4. Provider usage limit.
5. Repeated blocking error.
6. User pause/cancel.
7. App shutdown.

## 15. Plan Mode

Plan mode is a strict runtime mode, not a prompt suggestion.

Plan mode rules:

- No file writes.
- No `rtk` commands whose purpose is mutation.
- No formatters, codegen, migrations, package installs, or commits.
- Read/search/static analysis allowed.
- Tests/builds allowed only when used to validate feasibility and when they do
  not edit repo-tracked files.
- Output is a plan artifact attached to the conversation.
- User can execute a plan, which switches to Code mode or a selected execution
  mode.

Fastrock must enforce this at tool routing layer even if model asks for a write.

## 16. Command Execution Through RTK

Every shell command produced by the agent runs through `rtk`.

Local command execution:

- Detect `rtk` on PATH at startup and per project.
- Show missing `rtk` diagnostic with install link/instructions.
- Wrap commands without changing user-visible semantics.
- Preserve cwd, env, stdin, PTY mode, timeout, and cancellation.
- Store raw and RTK-filtered output where available, with redaction.
- Stream output to UI with backpressure.

Remote command execution:

- SSH target: execute command in remote shell with `rtk` prefix.
- SSM target: execute via AWS Session Manager channel or document mechanism with
  remote `rtk` prefix.
- If remote `rtk` missing, fail unless profile allows explicit fallback.
- Remote command output must include remote target ID and remote cwd.

Command policy:

- Default does not ask permission.
- Denylist can block exact commands or prefixes.
- Allowlist can restrict auto-run in locked-down profiles.
- Max runtime, max output bytes, and max concurrent commands are configurable.
- Commands modifying files outside current project are allowed by default but
  visibly labeled.

## 17. Project Folders and Remotes

Project folder kinds:

- Local path.
- SSH path.
- AWS Session Manager path.

Local:

- Native filesystem APIs.
- File watcher for project metadata, skills, MCP, and open buffers.
- Git metadata detection.

SSH:

- OpenSSH-compatible config support.
- Keychain/agent auth support.
- Remote shell detection.
- Remote path normalization.
- Remote `rtk` detection.
- Remote file read/write through SFTP or shell-safe transfer.
- Remote process execution with cancellation.

AWS Session Manager:

- AWS profile/region/target instance settings.
- Session Manager connectivity check.
- Remote shell startup.
- Remote `rtk` detection.
- Remote file operations through SSM-compatible channels or controlled shell
  transfer.
- Clear diagnostics for IAM, SSM agent, region, and target state failures.

All project folders:

- Associate with default model profile.
- Track recent conversations.
- Track project-local settings path.
- Load project instructions from `AGENTS.md` and compatible instruction files.
- Load project skills and MCP config.

## 18. MCP Support

MCP GUI:

- List global and project servers.
- Add/edit/delete server.
- Enable/disable server.
- Show connection state.
- Show discovered tools, resources, and resource templates.
- Toggle tools enabled/disabled.
- Mark tools always allowed.
- Show OAuth auth state.
- Restart server.
- View last error/log excerpt.

MCP runtime:

- Validate config before start.
- Debounce config changes.
- Start stdio servers with explicit cwd/env.
- Connect streamable HTTP servers.
- Support OAuth 2.1 + PKCE.
- Store tokens in keyring.
- Restart on watched path changes.
- Apply per-server timeout.
- Surface tool progress and errors into conversation transcript.

## 19. Skills Support

Skills GUI:

- Global/project tabs.
- List discovered skills with source and mode restrictions.
- Create skill.
- Edit `SKILL.md` in Fastrock editor.
- Delete skill.
- Move between global/project scope.
- Set mode restrictions.
- Show validation errors.

Skill runtime:

- Parse YAML frontmatter.
- Validate name/description.
- Resolve project over global.
- Resolve mode-specific over generic.
- Inject skill metadata into prompt only when available for current mode.
- Load full skill body only when selected or invoked.
- Watch skill dirs and update list without restart.

## 20. Caching and Memory

Fastrock caching has four layers:

1. Provider prompt caching.
2. Local context fragment cache.
3. Tool-output dedup cache.
4. Optional semantic memory index.

Provider prompt caching:

- Enabled by default where model/API supports it.
- Bedrock Runtime cache points inserted based on token threshold and model
  capability.
- Bedrock Mantle cache support tracked per model/API when documented or observed.
- Cache read/write token usage parsed and shown.
- Disabling supported prompt caching shows cost/performance warning.

Local context cache:

- Hash file fragments, instruction fragments, tool schemas, MCP manifests, and
  skill metadata.
- Avoid re-tokenizing unchanged fragments when assembling context.
- Invalidate on file watcher events or hash mismatch.

Tool-output dedup:

- Hash command output and file reads per conversation.
- Repeated large output becomes a compact reference.
- Raw bytes stay available in local event store if retention setting allows.

Memory:

- Local-first semantic memory, no hosted dependency.
- Store observations, project facts, user preferences, and prior fixes.
- Search uses lexical index first, vector index optional.
- GUI supports inspect/delete/disable.
- Memory injection is bounded and cited by source.

## 21. Lightweight Editor

Purpose: fast inspection and small-to-medium edits during agent sessions.

Required features:

- Tabs.
- Open file from transcript/tool output/project tree.
- Rope buffer.
- Async chunked load for large files.
- Virtualized line rendering.
- Tree-sitter syntax highlighting.
- Find in file.
- Go to line.
- Save.
- Revert.
- Dirty indicator.
- Read-only mode for remote or generated views.
- Diff view for agent changes.
- Basic undo/redo.
- Keyboard shortcuts configurable.

Explicitly excluded in v1:

- LSP.
- IntelliSense/autocomplete.
- Refactor engine.
- Debugger.
- Extensions.
- Multi-cursor beyond basic selection unless cheap.
- Rich notebook rendering.

Performance requirements:

- Opening a 10 MB text file must not freeze UI.
- Scrolling a large file renders only visible lines plus small overscan.
- Highlighting runs incrementally and can lag behind scrolling.
- Saving writes atomically and updates dirty state only after success.

## 22. Persistence

Default storage:

- macOS: `~/Library/Application Support/Fastrock`.
- Windows: `%APPDATA%\\Fastrock`.
- Linux: `$XDG_DATA_HOME/fastrock` or `~/.local/share/fastrock`.

SQLite:

- WAL mode.
- One primary database: `fastrock.db`.
- Append-only event table for conversations.
- Materialized tables for conversation list, profiles, folders, goals, editor
  tabs, MCP status snapshots, and cache metadata.
- Migrations are versioned, idempotent, and tested.
- DB actor serializes writes.

Files:

- User-editable settings: `settings.toml`.
- Project-local settings: `.fastrock/settings.toml`.
- MCP config: compatible `.mcp.json` read; Fastrock writes
  `.fastrock/mcp.json` unless user imports compatibility file.
- Skills under `.fastrock/skills`.
- Logs under `logs/` with rotation.

Secrets:

- OS keyring only.
- Database stores `SecretRef`, never secret bytes.
- Export omits secrets by default.

## 23. Security and Privacy

Default autonomy does not mean hidden behavior.

Required safeguards:

- All model requests visible in transcript metadata.
- All command executions visible with cwd, target, command, status, and output.
- All remote targets labeled in UI.
- All file writes visible in transcript and diff.
- No telemetry.
- No crash uploads.
- No hidden network calls except configured providers/AWS/MCP/remotes.
- Secrets redacted in logs, transcripts, and command output where detectable.
- User can disable command execution per profile.
- User can set locked-down approval profile, but default remains `Never`.

AWS privacy:

- Mantle requests always send `store=false`, so AWS does not retain Fastrock
  response state. There is no option to enable `store=true`.
- No `previous_response_id` chaining is used; conversation state is local-only.
- CloudTrail behavior must be documented in diagnostics.

## 24. Performance Budgets

Startup:

- Cold app window visible within 700 ms on target developer machines after OS
  process launch, excluding first-run font/cache warmup.
- Conversation list usable before full model/MCP/remote discovery completes.

UI:

- Main-thread tasks target under 4 ms, hard warning over 16 ms.
- Typing latency under 16 ms p95 in composer and editor for normal files.
- Conversation switch under 100 ms p95 for cached transcripts.
- Streaming token updates coalesced to at most 30 UI updates/sec per active
  conversation.

Concurrency:

- At least 8 active conversations can stream or run tools without UI stall.
- Backpressure must drop/coalesce display deltas before unbounded memory growth.
- Command output memory bounded by retention settings.

Editor:

- 10 MB text file opens progressively.
- 100k-line file scroll remains responsive.
- Syntax highlighting may degrade gracefully before blocking UI.

Persistence:

- App must tolerate crash mid-write without corrupting database.
- Event append and materialized index update must be atomic per transaction.

## 25. Error Handling

Errors are typed:

- Provider auth.
- Provider quota/throttle.
- Provider request validation.
- Provider stream disconnect.
- RTK missing.
- Command failed.
- Command timed out.
- Remote unavailable.
- Remote auth.
- MCP config invalid.
- MCP server failed.
- Skill invalid.
- File conflict.
- Database migration.
- Keyring unavailable.

Each error carries:

- Stable code.
- Human summary.
- Technical detail.
- Retryability.
- Suggested action.
- Source subsystem.
- Conversation/project/profile IDs where applicable.

No panic should reach UI for recoverable operational failures.

## 26. Testing Strategy

Unit tests:

- Mantle request conversion.
- Mantle SSE/stream parser.
- Mantle model discovery parser.
- Mantle SigV4 credential resolution from fake AWS shared config files,
  including named CLI profile, profile region fallback, and explicit Fastrock
  region override.
- Mantle `store` and `previous_response_id` policy.
- Runtime Converse conversion.
- Runtime stream parser.
- Runtime SDK credential resolution from fake AWS shared config files using the
  same `AwsCredentialSource` cases as Mantle SigV4.
- Cache point planner.
- Usage/cache token accounting.
- SQLite migrations.
- Settings validation.
- Keyring SecretRef behavior with fake keyring.
- `rtk` command wrapper.
- Plan mode tool gate.
- Goal accounting.
- Skills frontmatter validation.
- MCP config validation.
- Editor rope operations.

Integration tests:

- Mock Mantle `/v1/models`.
- Mock Mantle streaming `/v1/responses`.
- Mock Mantle SigV4 request signed with fake AWS CLI named profile credentials.
- Mock Runtime ConverseStream event sequence.
- Mock Runtime client constructed from fake AWS CLI named profile credentials.
- Local project command through fake `rtk`.
- Remote SSH command through test server/fake transport.
- SSM command through fake AWS transport.
- MCP stdio server lifecycle.
- MCP streamable HTTP lifecycle.
- OAuth fake callback flow.
- Multiple concurrent conversations.
- Goal loop resume/stop transitions.
- Editor open/save/diff flow.

UI tests:

- Conversation list renders multiple active conversations.
- Settings add/edit/remove profile.
- Bedrock diagnostics states.
- MCP and skills management flows.
- Plan mode prevents execution controls.
- Goal status controls.
- Large transcript virtualization.

Performance tests:

- Startup fixture with many conversations.
- High-rate token stream.
- Large command output.
- Large file open/scroll.
- Concurrent remote and provider streams.
- Cache hit/miss accounting.

All provider tests use mocks or recorded sanitized fixtures. No paid provider
calls in CI.

## 27. Acceptance Invariants

V1. Slint event loop thread never performs blocking I/O.

V2. All shell command execution invokes external `rtk` unless an explicit
test-only fake runner is injected.

V3. Default approval policy for new profiles and conversations is `Never`.

V4. Bedrock Mantle is available before any non-Bedrock provider work.

V5. Bedrock Mantle requests always send `store=false`, so AWS never retains
Fastrock response state, and no `previous_response_id` chaining is used. There is
no store toggle or per-conversation override. Mantle profiles still expose
project selection.

V6. Runtime and Mantle quota/usage accounting are kept separate.

V7. Conversations can run concurrently and switching visible conversation does
not cancel hidden active conversations.

V8. Project folder path is always interpreted relative to its target: local,
SSH, or SSM.

V9. Secrets never persist in SQLite, transcript files, logs, or exported config.

V10. Plan mode blocks mutations at tool-routing layer.

V11. Goal token/time accounting charges each turn delta at most once.

V12. Large command output and model streams are bounded by backpressure.

V13. Editor renders visible ranges only for large files.

V14. MCP and skills config changes hot-reload without app restart.

V15. No telemetry, crash upload, or Fastrock-owned cloud endpoint is called.

V16. Mantle SigV4 and Bedrock Runtime use one shared AWS credential/profile
resolver. Selecting an AWS CLI named profile in Fastrock must work for both
`bedrock-mantle.{region}.api.aws` SigV4 HTTP requests and
`bedrock-runtime.{region}.amazonaws.com` SDK/API requests.

V17. The GUI shell is dark-mode-only, resizable, and data-truthful. Startup must
render only persisted user data or an empty state; it must not seed prototype
conversations or project folders. Initial and minimum window sizes must keep
settings, RTK status, plan toggle, editor tabs, and conversation controls inside
the visible window bounds. RTK availability is detected from `PATH` and shown as
read-only status; it is not a user-disable button.

V18. The GUI shell exposes settings and conversation hierarchy without clipping
or ambiguity. Settings and other long right-pane content must scroll within the
window. The left sidebar is a master/detail tree: project folders are
collapsible top-level nodes, conversations appear under their project folder, and
unbound conversations appear in an unbound group whose execution cwd defaults to
the user's home directory. Interactable controls have hover/pressed states and
plain-language tooltips. Sending defaults to `Ctrl+Enter` and is configurable in
settings. RTK and prompt token compression are default-on settings that the user
can disable explicitly; disabling RTK is the explicit fallback override allowed
by V2, and disabling prompt compression affects only model-facing prompt context,
not persisted raw transcripts.

V19. GUI overlay and right-pane layout are z-order and geometry safe. Tooltips
must render in a popup/overlay layer above normal content and must not be clipped
or hidden under sibling panes, scroll views, or cards. Inactive right-pane panels
must allocate zero layout height; the active Settings panel must occupy the
usable right drawer area with its own internal scroll region instead of appearing
as a small clipped panel after hidden content.

V20. GUI controls are self-describing and state-truthful. Every interactive
control reads as its control type: momentary actions render as buttons, boolean
settings render as labeled on/off switches, and mutually-exclusive choices render
as segmented controls whose currently-selected option is visibly highlighted.
Controls that are not interactive in the current state are visually distinct from
active controls and do not respond to hover or click. This covers two cases:
read-only status (RTK detection, mode, the configuration summary) and actions
gated on an unmet prerequisite (for example, profile edit/tuning/probe actions
and message send while no model profile is configured). Scrollable regions show a
scrollbar thumb whenever their content overflows. When a prerequisite blocks an
action, the UI states the prerequisite in plain language and offers a path to
satisfy it.

V21. GUI hover tooltips are non-disruptive. A tooltip appears only after the
pointer rests on a control for a short delay, never on raw pointer movement, so
dragging the pointer across controls shows no tooltip. The active tooltip renders
in a single window-level overlay above all panes (so it is never clipped, per
V19) and must not grab keyboard focus or capture the pointer from the control
beneath it or from the message composer. At most one tooltip is visible at a
time, and it disappears as soon as the pointer leaves the control.

## 27.1 Backprop Bug Ledger

| id | status | bug | invariant | fix task |
|----|--------|-----|-----------|----------|
| B1 | x | Settings pane clipped below the window, sidebar split conversations from project folders, key buttons lacked tooltips, Send used the wrong placement/shortcut, and RTK/prompt-compression defaults were not user-toggleable. | V18 | T30 |
| B2 | x | Tooltip rectangles rendered inside clipped child controls and hidden right-pane panels still consumed layout height, burying tooltips and shrinking Settings to a clipped strip. | V19 | T31 |
| B3 | x | Settings options were cryptic look-alike buttons: the user could not tell which were toggles, which were mutually-exclusive choices, or which option was active; scroll regions had no scrollbar; and profile-dependent actions and message send gave no signal that a model profile must be configured first. | V20 | T32 |
| B4 | x | Hover tooltips used a per-control PopupWindow shown on every pointer-move event; they flickered constantly while the pointer dragged across controls and grabbed keyboard/pointer focus (e.g. from the composer), feeling hackish. | V21 | T33 |

## 28. Implementation Task Graph

Status values: `.` pending, `>` in progress, `x` done.

| id | status | task | depends |
|----|--------|------|---------|
| T1 | x | Create Cargo workspace and crate skeleton | V1 |
| T2 | x | Add Slint app shell with sidebar, conversation pane, tab pane, settings pane | T1,V1 |
| T3 | x | Add Tokio runtime, app supervisor, typed command/event channels | T1,V1,V12 |
| T4 | x | Add SQLite persistence actor, schema, migrations, WAL | T1,V9 |
| T5 | x | Add settings model, OS keyring SecretRef storage, and shared AWS credential/profile config | T4,V9,V16 |
| T6 | x | Add LlmProfile GUI with add/edit/remove/duplicate/save validation | T2,T5 |
| T7 | x | Implement Bedrock Mantle client: API-key auth, AWS CLI profile SigV4 auth, models, responses, streaming | T3,T5,V4,V5,V16 |
| T8 | x | Implement Bedrock Mantle quota, project, CloudTrail diagnostics | T7,V5,V6 |
| T9 | x | Implement Bedrock Runtime client: AWS CLI profile auth, Converse/ConverseStream/Invoke fallback | T3,T5,V4,V6,V16 |
| T10 | x | Implement provider-neutral stream/event normalization | T7,T9 |
| T11 | x | Implement conversation actor and transcript event store | T3,T4,V7 |
| T12 | x | Implement prompt/context assembly and built-in modes | T11 |
| T13 | x | Implement plan mode enforcement | T12,V10 |
| T14 | x | Implement external `rtk` runner for local commands | T3,V2,V12 |
| T15 | x | Implement local project folders and file watcher | T4,T14,V8 |
| T16 | x | Implement SSH remote target and remote `rtk` execution | T14,T15,V2,V8 |
| T17 | x | Implement AWS Session Manager remote target and remote `rtk` execution | T14,T15,V2,V8 |
| T18 | x | Implement MCP config, stdio, streamable HTTP, OAuth, GUI | T3,T5,V14 |
| T19 | x | Implement skills discovery, validation, GUI, prompt injection | T5,T12,V14 |
| T20 | x | Implement cache planner and usage/cache accounting | T7,T9,T12,V6 |
| T21 | x | Implement goal loop, goal UI, persistence, accounting | T11,T12,V11 |
| T22 | x | Implement editor rope buffer, tabs, syntax highlighting, save/diff/find | T2,T15,V13 |
| T23 | x | Add command/file/model transcript renderers and coalescing | T2,T10,T14,V12 |
| T24 | x | Add multi-conversation scheduler and hidden conversation progress UI | T11,V7 |
| T25 | x | Add full test-support crate with fake Bedrock, fake RTK, fake remotes, fake MCP | T1 |
| T26 | x | Add unit/integration/UI/performance test suites from section 26 | T25 |
| T27 | x | Add packaging for macOS, Windows, Ubuntu | T1,T2 |
| T28 | x | Add developer docs and contribution workflow | T1 |
| T29 | x | Replace prototype light shell with Codex-like dark responsive shell and truthful startup state | T2,T11,T14,V1,V2,V7,V17 |
| T30 | x | Implement scroll-safe settings, project conversation tree, tooltips, send shortcut preferences, RTK disable, and prompt compression preference | T2,T11,T12,T14,T29,V2,V7,V12,V18 |
| T31 | x | Fix tooltip z-order and full-height Settings pane geometry | T2,T30,V19 |
| T32 | x | Rehabilitate GUI clarity with a typed control kit (action buttons, on/off switches, segmented controls, read-only fields), scrollbar affordance, disabled/prerequisite states, and Codex-like layout | T2,T30,T31,V18,V19,V20 |
| T33 | x | Replace per-control popup tooltips with a single delayed window-level overlay (global Tip + HoverTip + absolute-position) that does not steal focus or flicker | T31,T32,V19,V21 |
| T34 | x | Remove the Mantle store option and force store=false on every Mantle request (no AWS retention, no previous_response_id chaining) | T7,T8,V5 |

## 29. Release Criteria for v1

v1 is releasable when:

- User can create/edit/delete Bedrock Mantle and Runtime profiles.
- User can run at least two concurrent conversations in different folders.
- User can run local commands through `rtk`.
- User can run remote commands through SSH and SSM when target has `rtk`.
- User can use plan mode and execute a plan.
- User can create/resume/complete a goal.
- User can configure MCP and skills in GUI.
- User can open/edit/save files in tabs.
- UI remains responsive under provider streaming and command output load.
- No tests call real paid provider APIs.
- No telemetry or Fastrock cloud endpoint exists.
