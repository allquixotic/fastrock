module github.com/allquixotic/fastrock

go 1.27.2

require (
	github.com/aarzilli/nucular v0.0.0-20260922071552-58b808aa5772
	github.com/ebitengine/purego v0.11.0
	github.com/pelletier/go-toml/v2 v2.3.1
	github.com/yuin/goldmark v1.8.2
	github.com/zalando/go-keyring v0.2.8
	golang.org/x/image v0.45.0
	golang.org/x/mobile v0.0.0-20231127183840-76ac6878050a
	golang.org/x/net v0.55.0
	golang.org/x/sync v0.22.0
	golang.org/x/sys v0.47.0
)

require (
	github.com/BurntSushi/xgb v0.0.0-20160522181843-27f122750802 // indirect
	github.com/danieljoos/wincred v1.2.3 // indirect
	github.com/ebitengine/gomobile v0.0.0-20260820040257-d11f821a26a6 // indirect
	github.com/ebitengine/hideconsole v1.0.0 // indirect
	github.com/godbus/dbus/v5 v5.2.2 // indirect
	github.com/golang/freetype v0.0.0-20161208064710-d9be45aaf745 // indirect
	github.com/hajimehoshi/ebiten/v2 v2.10.0 // indirect
	golang.org/x/exp/shiny v0.0.0-20250408133849-7e4ce0ab07d0 // indirect
	golang.org/x/text v0.41.0 // indirect
)

replace github.com/aarzilli/nucular => ./third_party/nucular
