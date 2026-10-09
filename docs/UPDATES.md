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

Downloads use HTTPS and SHA-256 to detect transport corruption. Archives are
limited to 192 MiB compressed, 512 MiB expanded and 4,096 entries. Absolute/traversal
paths, symlinks, duplicate files and unexpected top-level contents are rejected.
Before an update becomes ready, and again immediately before replacement, the
native code signature authenticates the publisher:

- Windows verifies Authenticode trust, revocation and the complete publisher name
  against the running signed Fastrock executable. A renewed certificate for the
  same publisher is accepted; an unsigned or differently signed update is rejected.
- macOS requires Developer ID team `B6XDYNLMPU`, bundle ID
  `com.allquixotic.fastrock`, a valid sealed bundle, a stapled notarization ticket,
  and a successful Gatekeeper assessment.

Signed Go build metadata must also identify Fastrock and match the selected
release version, preventing an old signed binary from being relabeled as newer.
A changed checksum in a compromised release feed cannot bypass these checks.
The existing installation remains intact on a failed check. Development builds
cannot serve as an unsigned bridge into managed updates.

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
Windows amd64 and macOS arm64/amd64 with Go 1.27.2 and `CGO_ENABLED=1`, statically linking FLTK, runs headless
tests, and publishes:

- `fastrock-windows-amd64.zip`
- `fastrock-darwin-arm64.zip` and `.dmg`
- `fastrock-darwin-amd64.zip` and `.dmg`
- `SHA256SUMS`

On a provisioned Mac:

```sh
go run dev/package.go -os darwin -arch arm64 -version vX.Y.Z -dmg=true
```

The packager signs the executable and enclosing app with hardened runtime and
secure timestamps, notarizes and staples the app, then builds its ZIP and DMG.
It signs/notarizes/staples the DMG separately. The pinned application identity is
`9A3CFFC04D3472208A62C48E707EA6D4261998A1`; the Keychain notarization profile is
`AC_NOTARY`. No private key is stored in this repository or exported by the scripts.
Reports, submission IDs and Apple logs remain under `build/release/signing-<arch>`.
If notarization times out, rerun `dev/sign_macos.py` for the same artifact and
report directory to resume its recorded submission; do not rebuild it first.

The macOS CI job requires a trusted self-hosted runner labeled `macOS` and
`fastrock-signing`, with this identity/profile provisioned. The Windows job uses
Azure Artifact Signing with OIDC, matching the Codex repository's signing action.
Its protected `azure-artifact-signing` environment needs these secrets:

- `AZURE_ARTIFACT_SIGNING_CLIENT_ID`, `AZURE_ARTIFACT_SIGNING_TENANT_ID`,
  `AZURE_ARTIFACT_SIGNING_SUBSCRIPTION_ID`
- `AZURE_ARTIFACT_SIGNING_ENDPOINT`, `AZURE_ARTIFACT_SIGNING_ACCOUNT_NAME`,
  `AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME`

Optionally set `FASTROCK_WINDOWS_PUBLISHER` to the certificate's exact subject to
check the expected identity during packaging. Configure the Azure federated
credential for this repository's protected environment before releasing. No
credentials, runner registration, or Azure permissions are provisioned by a
normal build. CI refuses to publish if signing or verification fails. All action
versions are pinned. Build provenance attestations are required for every ZIP,
DMG, platform executable and the final checksum manifest. Before publishing,
CI verifies every release asset's attestation against this repository, release
workflow, tag and source commit. Attestation failure prevents publication.
Downloaded files can be checked with
`gh attestation verify FILE --repo allquixotic/fastrock --signer-workflow allquixotic/fastrock/.github/workflows/release.yml`.
The v0.1.1 release predates public-repository attestations.
Native signature verification remains mandatory. The final DMG is mounted read-only to verify the
contained app signature, notarization ticket, and Gatekeeper assessment.

For a Windows cross-build, use `-phase build`; sign the resulting executable on
Windows, then use `-phase package` there to verify and archive it. `-unsigned` is
available only as an explicit local fixture option; those artifacts are rejected
by the updater. Generated files remain under ignored `build/` and `dist/`.
A tag is not created by ordinary builds or commits. The previously generated
Ed25519 update key is not used by this native-signing pipeline.

References: [GitHub release assets](https://docs.github.com/en/rest/releases/releases#get-the-latest-release),
[Windows shell links](https://learn.microsoft.com/en-us/windows/win32/shell/links),
[process wait semantics](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject).
