# Release updates and installation

Release builds (a stable `X.Y.Z` stamped into `internal/buildinfo.Version`) check
`https://api.github.com/repos/allquixotic/fastrock/releases/latest` on startup.
**Help → Check for updates** and **Settings → About → Check for updates** show the
same shared updater. Only stable, newer releases for the current OS/architecture
are selected. Local `dev` builds stay portable and do not install/update themselves.

One updater belongs to the shared app-server broker, so popped-out windows do not
download duplicates. Network requests, hashing, extraction and staging run in the
background. The original installation continues running until all its windows
close. A copy of the executable then waits for the old process, copies the staged
payload beside the installation, preserves a rollback copy, replaces the app and
restarts it. A failed replacement restores the old installation. An unlaunchable
new executable is rolled back. Failed automatic installation is recorded under
`<Fastrock settings directory>/updates/last-update.json` and reported next launch.
The updater never stops Codex conversations just to install an update.

Downloads use HTTPS and GitHub's asset SHA-256 digest (or the release's
`SHA256SUMS` for older assets). Archives are limited to 192 MiB compressed,
512 MiB expanded and 4,096 entries. Absolute/traversal paths, symlinks, duplicate
files and unexpected top-level contents are rejected. macOS additionally verifies
the app bundle's code signature before replacing it. Checksums authenticate the
transported asset through GitHub; they are not a separately signed update feed.
The existing binary is preserved on download, checksum and extraction failures.

Windows release launches establish `%LOCALAPPDATA%\Programs\Fastrock\fastrock.exe`
and a **per-user** Start menu shortcut using native `IShellLinkW`/`IPersistFile`.
An existing managed copy is launched instead of overwriting a running executable;
that copy uses the normal updater. No MSI, service, scheduled task, UAC request,
C compiler or administrator installation is used. Settings and credentials stay
in their existing per-user locations. A failed initial install/shortcut operation
is reported; the downloaded binary remains usable.

macOS releases contain `Fastrock.app` in a DMG with an Applications link. Drag the
app into `/Applications`, or `~/Applications` when the system folder is not
writable. Updates need write permission to that app's parent directory and never
request elevation. Quit all Fastrock windows to apply a ready update.

## Publishing

Tagging a tested commit `vX.Y.Z` triggers `.github/workflows/release.yml`. It builds
Windows amd64 and macOS arm64/amd64 with Go 1.27.2 and `CGO_ENABLED=0`, runs headless
tests, and publishes:

- `fastrock-windows-amd64.zip`
- `fastrock-darwin-arm64.zip` and `.dmg`
- `fastrock-darwin-amd64.zip` and `.dmg`
- `SHA256SUMS`

Locally: `go run dev/package.go -os darwin -arch arm64 -version vX.Y.Z -dmg=true`.
For Windows omit `-dmg`; packaging works from macOS using a Go cross-build.
Generated files are ignored under `build/` and `dist/`.

The current workflow ad-hoc signs macOS bundles. Public distribution without a
Gatekeeper warning additionally needs a Developer ID certificate and notarization;
these credentials are not installed or inferred by this workflow. The packager
accepts `FASTROCK_SIGN_IDENTITY` on a Mac where the signing identity is provisioned.
A tag is not created automatically by ordinary builds or commits.

References: [GitHub release assets](https://docs.github.com/en/rest/releases/releases#get-the-latest-release),
[Windows shell links](https://learn.microsoft.com/en-us/windows/win32/shell/links),
[process wait semantics](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject).
