# Third party notices

Fastrock is licensed under Apache-2.0. The UI shell is a native Go adaptation
of the layout and interactions in allquixotic/codex's `codex-rs/gui` (Apache-2.0).
The Rally navigation and workflows follow the locally supplied Rally Anywhere
HTML reference. Fastrock is an independent application, without Rally branding.

`third_party/nucular` is vendored from aarzilli/nucular commit
`58b808aa577248d4d3d0cd7af89ea4a3d0dc5d43` (MIT). Its original LICENSE is retained.
See `third_party/README.md` for the local changes. The Go font is BSD licensed;
its copyright and license accompany the module in `golang.org/x/image/font/gofont`.
Go module dependencies retain their respective licenses. `go list -m all` lists
exact versions, also pinned by go.mod and go.sum.

Ebitengine 2.10 and purego 0.11 provide the native desktop adapter and are
Apache-2.0 licensed. Their pinned modules include their copyright/license notices.

Codex is a separate user-installed application. It is not bundled with Fastrock.
No Python or Rust code is compiled into Fastrock.
