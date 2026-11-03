# Fastrock 0.4.0-rc.10

- Fastrock conservatively learns durable terminology, preferences and project conventions from explicit user statements after completed turns. It does not learn after individual tool calls or mine old conversations. Learning uses the configured cheap model, runs while idle, and is cancelled when foreground work starts.
- A native in-process recall tool reads only the RAM cache and returns at most 2 KiB of relevant facts per user turn. Facts stay within their project unless explicitly shared. Goose remains unchanged; recall is model-initiated and uses an ordinary tool step.
- Memories are compressed in vwork's existing redb, with deduplication, expiry, capacity limits and protection against stale learning results. Settings → Memories lets you inspect, edit, pin, forget or clear facts, and independently disable learning or recall.
- Existing provider controls, Bedrock routes, speed settings, Rally boards, scheduling, workspaces and conversation features are retained. No separate backend installation is required.

See [memory behavior and limits](https://github.com/allquixotic/fastrock/blob/v0.4.0-rc.10/docs/MEMORIES.md). Forgetting a memory does not erase prior conversation/tool results. Memories maintained by previously enabled external extensions remain separate.

Signed Windows x64 ZIP; macOS compatibility compile only. This is a release candidate; final live AWS profile acceptance remains with the user.
