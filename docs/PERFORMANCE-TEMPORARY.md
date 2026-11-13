# Temporary copy performance

Requested 2026-11-13 after `workspace_temporary` (opening a terminal or running a
tool on a virtual workspace) took eight to ten minutes on the work machine, whose
on-access scanner makes file operations slow. No work-machine trace is available.
The cause below was established from the code and reproduced on a Mac.

## What a temporary copy did

Per copy, in 0.5.1 and earlier:

1. Re-encode **all** permanent Git history of the repository through libgit2's
   pack builder: every object reconstructed and hash-verified from storage more
   than once, searched for deltas, compressed and indexed again.
2. Create one loose file per branch, tag and remote-tracking ref, each through a
   lock file and rename.
3. For every source file: check each ancestor directory, create its parent,
   attempt to read the destination, test for existence, then write.
4. Write a Git index with no file metadata, so the first `git status` in the copy,
   and vwork's own change report after every task, reread every file.

On release it read every tracked file to compare it with storage, then deleted
every file. All of this grows with the repository, not with what changed.

## What it does now

Fastrock 0.5.2 pins vwork `afec4b21646c0dd818d54909101fb211d90f5978` (0.4.2).

| Part | Work per copy |
|---|---|
| Git history | Two hard links per shared pack. History is transcoded once per repository into `git-cache/<repository>` beside the database, reusing stored deltas, and extended only when references change or a requested base commit is absent. |
| Refs | One `packed-refs` file. |
| Index | One file, written directly with each materialized file's size and timestamps. |
| Virtual edits | One private pack of blobs absent from history. |
| Source tree, first copy | One create per file, written by up to eight workers. |
| Source tree, later copies | The released tree is adopted from `pool/`. Each tracked directory is listed once; files whose entry or recorded metadata differ are rewritten; anything vwork did not write is renamed into `trash/`. |
| Release | Recovery opens only files whose metadata changed. The tree is renamed into the pool or trash; deletion happens in the background. |

Packs are immutable: Git and libgit2 replace them by rename and never rewrite one
in place, so sharing a file cannot carry one copy's activity into another. Source
files are deliberately **not** hard-linked: a tool that edits in place would
change every copy sharing that file.

## Measurements

Mac, internal SSD, no security software. Repository: 874 MB packed, 197,819
objects, 22,217 commits, 963 refs; source tree 3,156 files, 286 MB.
Reproduce with `cargo run --release --example temporary_bench` in vwork.

| | 0.5.1 engine | 0.5.2 engine |
|---|---|---|
| Temporary copy | 153.8 s, every time | 23.9 s once per repository; then 0.32 s (new tree) or 0.047 s (reused tree) |
| Written per copy | 1.31 GB in 4,236 files | about 1 MB; no source files when the tree is reused |
| Release | 902 ms | 45 ms |
| `git status` in the copy | 503 ms | 28 ms |

These numbers do not establish work-machine latency: a fast disk hides exactly
the per-file cost a scanner adds. Counts of files created and bytes written are
the transferable part. `workspace_temporary` returns `timings_ms`
(`shared_history`, `files`, `git_view`, `snapshot`, `repository_wait`, `total`)
and `work` (files written, reused, verified and removed; packs linked or copied;
history objects exported), so a slow host can be diagnosed from numbers alone.

## Limits

- The first copy of a repository pays the one-time history export, and the first
  source tree still costs one file creation per file. NTFS without administrator
  rights has no constant-time copy of a whole tree.
- Reuse trusts recorded size and timestamps, as Git's index does. An in-place edit
  that preserves both is not detected on Windows. `verify_materialized_content =
  true` in the store's `config.toml` reads every file back instead.
- Disk: one history cache per repository, about the size of its packed history,
  plus pooled trees. Pooled trees count toward `max_temporary_copies` (four) and
  expire after `pooled_tree_ttl_seconds` (four hours; zero disables pooling).
  `git-cache/`, `pool/` and `trash/` are derived and may be deleted while Fastrock
  is closed.
- If a tool still has files open in a released copy, Windows refuses the rename;
  the copy is then removed in place as before, and retried if that fails.

## Verification

Upstream: full test suite on macOS, strict lint for macOS and the Windows target,
and hosted Windows/macOS CI at the pinned revision. Thirteen new tests cover the
pack writer against Git's own verifier, shared and incremental history, rebuild of
a truncated cache, metadata-based reuse and recovery, strict mode, and pooled
trees (pristine adoption, bounds, expiry, fallback). Fastrock: Mac compile and the
signed-release Windows test job. The games VMs were stopped on 2026-11-13, so no
guarded Windows benchmark or native acceptance run exists for this change.
