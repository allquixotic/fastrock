# Dependency refresh for 0.5.0

The registry audit and exact Git revisions are recorded in
[the dependency receipt](benchmarks/050-dependencies-2026-11-09.json).
`Cargo.lock` records every resolved package and source checksum.

Goose 1.58.0 captures hooks in `StateMachineServices`. Fastrock creates public
services and an ACP connection for each activated conversation, loading that
repository's current hooks while sharing the public session store and permission
manager. Bounded
in-memory routing preserves reply order and separates permission-request IDs;
connections are released after Goose completes its close operation. No separate
scheduler starts. Context reports use the public
`_goose/unstable/context/report` API. Goose source is neither vendored nor patched.
Goose 1.58 routes bare closed-weight GPT IDs to Mantle; `us.` inference profiles
use Bedrock Runtime. Fastrock's default cheap model is `us.openai.gpt-6-luna`.
Explicitly saved model choices and credentials remain intact.

Codex support crates move from 0.162.0 to stable 0.167.0. Its timestamped
`ThreadItem` now wraps `ThreadItemPayload`; Fastrock preserves that wrapper at
notification, history and export boundaries. This does not enable a Codex backend.

Keyring 4 uses its native OS stores. Explicit Windows target modifiers retain
`fastrock:<endpoint>` and `fastrock:<kind>/<UUID>` identities from older versions;
macOS keeps the same service/user pair. No secret migration or plaintext fallback
is introduced. SHA-256 identities keep their original lowercase hexadecimal
encoding after the SHA-2/HMAC API upgrade.

Slint 1.18.1 and Fjall 3.1.13 were already the latest stable registry versions.
Existing Slint rendering/selection/accessibility patches and engine LZ4 storage
remain. vwork moves to its latest source revision, retaining the performance
fixes shipped with Fastrock 0.4.2. Legacy redb/zstd readers remain migration-only.
