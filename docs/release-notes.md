Fastrock 0.3.1 adds vwork database maintenance to the native virtual-workspace UI.

Open **File → Virtual workspaces…**, select a repository/workspace, then choose
**Storage → Compact selected vwork store** (or the Compact store button).
Fastrock reports reclaimed space, a threshold skip, or an active-operation deferral.
The request runs in the background and preserves unsaved editor text.

Requires vwork 0.3.1 for online compaction. vwork also checks automatically each
hour after more than five minutes of user inactivity, and only compacts when more
than 1 GB is estimated reclaimable. Its new repo_fetch tool refreshes stored Git
refs without changing existing workspace snapshots. Upgrade vwork independently;
Fastrock continues to use installed Goose ACP and vwork out of process.

Windows x64 ZIP, using the existing fast build profile. Authenticode signing,
RFC 3161 timestamp, native acceptance checks and GitHub provenance are required.
