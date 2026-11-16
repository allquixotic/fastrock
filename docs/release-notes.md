# Fastrock 0.5.3

Updates embedded, unmodified Goose from 1.58.0 to
[1.59.2](https://github.com/aaif-goose/goose/releases/tag/v1.59.2).
All Goose crates are pinned to upstream release commit
`9da692fde8ac69ec385b5e7d68b92cc7101a509a`.

Fastrock otherwise retains 0.5.2's behavior and embedded vwork 0.4.2, including
shared temporary-copy history, pooled source trees and release by rename.
Provider configuration, credentials, conversations, workspace history,
checkpoints and composer drafts are preserved.

Windows x64 ZIP only, signed, timestamped and verified through GitHub Actions;
macOS receives a compatibility compile. The games VM remains stopped, so this
release uses the same CI verification scope as 0.5.2. No games benchmark or
games native acceptance is claimed.

Use **Quit** in the previous version's tray menu before launching 0.5.3.
