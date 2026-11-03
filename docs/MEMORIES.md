# Automatic memories

Fastrock learns small, durable facts from future completed user turns. It uses the
cheap profile selected in Providers. It does not mine existing histories or learn
from tool output, assistant statements, delegated work, temporary chats, slash
commands, failed/cancelled turns, or turns changed through steering. Inputs over
8 KiB or containing detected secret-like material are skipped. A model selects
explicit terminology, preferences, corrections and project conventions; each
candidate must include an exact quotation from the user's input. This is a
conservative filter, not a guarantee of perfect semantic judgment or secret detection.

Settings → Memories independently controls learning and recall. Inspect, edit,
pin, forget, or clear entries there. Editing protects a fact from automatic
replacement. “Use across all projects” explicitly promotes a fact to global
recall; automatic learning always stays within its repository. Pinned facts stay
until forgotten. Other facts expire 180 days after their last update. The collection
holds at most 512 facts overall and 96 per repository; oldest unpinned facts make
room. There are at most five candidate facts per completed turn.

The dedicated memory state is zstd-compressed in the same redb owned by embedded
vwork. It does not enumerate source blobs or worktrees. Stable repository identity
shares memories among isolated vwork workspaces. Ordinary directory chats use a
normalized directory identity. Search summaries remain separate and unchanged.

Foreground sends never await memory storage, extraction, maintenance, or a cache
lock. Cold/contended caches skip recall. Goose's supported builtin extension API
provides `fastrockmemories__recall` over in-process memory streams. Its instructions
tell the model to recall prior terminology/preferences when relevant, without
calling it for self-contained tasks. The tool reads only RAM and returns at most
2,048 UTF-8 bytes of fact context per user turn, including framing. Parallel or
repeated calls cannot multiply that budget. Goose supplies the session identity;
the model cannot select another project. Memory text is quoted reference data
subordinate to current instructions. Matching is local, with no embedding service.

Recall is model-initiated, so using it takes an ordinary tool step and its result
appears in the conversation like other tool results. Forgetting a fact does not
erase those historical results. New ordinary chats include the tool; existing
coding chats acquire it on reopen. Explicit restricted/empty extension sets remain
restricted. The native memory controls govern its content independently of tools.
Automatic transient system-prompt injection is **not supported by the embedded
Goose ACP API**: its public provider factory handles inventory, while a private
manager creates actual conversational providers. Fastrock does not patch Goose,
rewrite every prompt/history, or pretend that factory decorates model calls.

Learning has a 45-second request timeout, shares a single background provider slot
with search summaries, and is cancelled when any foreground conversation starts.
It debounces idle work, bounds its queue at 64 turns, and retries unavailable
providers at most once a minute. Optional learning may be skipped under overload.
An idle worker warms the small cache once; it does not repeatedly scan the database.
Background writes use redb's non-fsync durability; a crash can lose the newest
learned facts. User edits/forget/settings use durable transactions. An already
committing OS write cannot be forcibly cancelled; brief shared-store contention
remains possible. Send has no dependency on the memory worker, and queued writes
recheck cancellation before committing.

Revision checks reject stale results. Forgetting stores hashed key/text tombstones
and invalidates queued/in-flight learning. Clear also invalidates active learning;
old conversations are not reprocessed. A semantically equivalent fact expressed
with a different key and different wording can still be learned from a future
explicit user statement. Disable learning when no future retention is wanted.
Deleting a conversation does not automatically delete already learned facts;
Memories provides their separate retention controls.

This layer does not enable or modify Goose's separate Memory extension. If that
extension was enabled previously, its file-based memories remain independent and
are not covered by Fastrock's context budget or forget controls.

Verified on Windows with 834 Rust tests, embedded provider/store fixtures, and
native learning/recall/settings/restart/cancellation tests; macOS compile only.
The [verification receipt](benchmarks/memory-games-2026-11-03.json) records the
tested executable hashes and related regression checks. No live provider
credentials or paid model calls were used.
