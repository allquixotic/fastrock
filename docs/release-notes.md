# Fastrock 0.5.2

Opening a terminal or running a tool on a virtual workspace no longer copies the
repository's whole Git history. Each temporary copy used to re-encode all
history, create one file per branch and tag, and look up every source file about
a dozen times; closing it read and then deleted every file. That work grew with
the size of the repository rather than with what had changed.

Embedded vwork 0.4.2 now:

- exports a repository's history once into shared files that every temporary
  copy refers to by hard link, and extends them only after a fetch or commit;
- writes branches and tags as a single file, and a Git index that already knows
  each file's size and time, so Git status checks do not reread sources;
- keeps a closed copy's source tree for the next copy of the same repository,
  which then rewrites only files that differ and moves anything left behind by
  tools out of the way;
- closes a copy by renaming it aside and deleting in the background.

The first temporary copy of each repository after upgrading performs the one-time
history export; later copies do not. The first copy also still creates every
source file once. Measured on a Mac without security software, using an
874 MB, 197,819-object repository with 963 refs and a 286 MB source tree: a copy
took 153.8 seconds and wrote 1.31 GB before; now 23.9 seconds once, then 0.32
seconds for a new tree and 0.05 seconds for a reused one. This does not establish
work-machine latency under on-access scanning. The `workspace_temporary` tool
result now includes `timings_ms` and a `work` receipt for that purpose. See
[temporary copy performance](https://github.com/allquixotic/fastrock/blob/main/docs/PERFORMANCE-TEMPORARY.md).

Reused source trees count toward the existing limit of four temporary copies and
are removed four hours after release. Shared history is stored once per
repository beside the workspace database and can be deleted while Fastrock is
closed. Unchanged files are recognized by recorded size and timestamps, as Git
does; set `verify_materialized_content = true` in the workspace store's
`config.toml` to read every file back instead.

Goose 1.58.0 remains unmodified. Provider configuration, credentials,
conversations, workspace history and checkpoints are preserved. Windows x64 ZIP
only; macOS receives a compatibility compile. The Windows test VMs were
unavailable for this release: no games benchmark or native acceptance run is
claimed. Windows evidence is the embedded engine's own Windows CI and the
signed-release Windows test job.

Use **Quit** in the previous version's tray menu before launching 0.5.2.
