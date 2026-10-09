# Third party notices

Fastrock is licensed under Apache-2.0. The UI shell is a native Go adaptation
of the layout and interactions in allquixotic/codex's `codex-rs/gui` (Apache-2.0).
The Rally navigation and workflows follow the locally supplied Rally Anywhere
HTML reference. Fastrock is an independent application, without Rally branding.

The custom layout and canvas code in `internal/desktop` is derived from aarzilli/nucular commit
`58b808aa577248d4d3d0cd7af89ea4a3d0dc5d43` (MIT). Its original LICENSE is retained.
See `third_party/README.md` for the local changes. The Go font is BSD licensed;
its copyright and license accompany the module in `golang.org/x/image/font/gofont`.
Go module dependencies retain their respective licenses. `go list -m all` lists
exact versions, also pinned by go.mod and go.sum.

`github.com/pwiecz/go-fltk` commit `3e944122e7b1f66db5e3a968e679285930c7e10e`
provides the Go bindings (MIT) and pinned FLTK 1.4 static libraries. FLTK is
licensed under LGPL-2.0 with exceptions permitting static linking. The binding's
LICENSE and include/COPYING.fltk retain the full notices in the module distribution.
The executable uses these static archives, not a separately distributed FLTK DLL.
Purego 0.11 remains in use for existing OS integrations (Apache-2.0); its module
retains its license. Ebitengine and the Shiny window backend have been removed.

Codex is a separate user-installed application. It is not bundled with Fastrock.
No Python or Rust code is compiled into Fastrock.

`internal/ui/config_schema.json` is an unmodified Apache-2.0 snapshot from
`allquixotic/codex`, commit `9c5657db96a6afa5f2a1f3c18885ac9b66983193`,
`codex-rs/core/config.schema.json` (OpenAI and contributors). It supplies field
descriptions and defaults without requiring a Rust build. The installed CLI
remains authoritative for runtime values and validation; newer keys are editable
even when absent from this snapshot. The Apache license is retained in LICENSE.

The TOML configuration editor uses `github.com/pelletier/go-toml/v2` (MIT).
Its module includes the license and copyright notice; no C code is required.

Native transcript Markdown uses `github.com/yuin/goldmark` 1.8.2 (MIT,
Copyright 2019 Yusuke Inuzuka). The pinned module contains its complete license.
It parses text locally; it does not execute HTML or fetch remote content.
