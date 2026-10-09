package ui

import (
	"image/color"
	"runtime"

	"github.com/allquixotic/fastrock/internal/desktop"
)

type settingsGroup struct {
	Title string
	Pages []string
}

func settingsNavigation(goos string) []settingsGroup {
	app := []string{"Appearance", "Keyboard", "Rally"}
	if goos == "windows" {
		app = append(app, "Sandbox")
	}
	app = append(app, "Diagnostics")
	return []settingsGroup{
		{"Configuration", []string{"Common", "Codex configuration", "Raw configuration", "Models", "Import"}},
		{"Sign-in", []string{"Account", "AWS Bedrock", "Local providers"}},
		{"Extensions", []string{"MCP servers", "Skills", "Plugins", "Hooks", "Features", "Memories"}},
		{"App", app},
		{"Help", []string{"Feedback", "About"}},
	}
}

func (a *App) drawSettingsNavigation(w *desktop.Window, s *settingsView) {
	for _, group := range settingsNavigation(runtime.GOOS) {
		muted(w, group.Title, a.p)
		for _, page := range group.Pages {
			w.Row(30).Dynamic(1)
			if flatRow(w, page, "", s.Page == page, color.RGBA{}, a.p) {
				a.settingsPage(page)
			}
		}
	}
}
