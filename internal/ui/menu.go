package ui

import (
	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/label"
	"path/filepath"
)

var applicationMenus = []struct {
	Title string
	Items []struct{ ID, Title string }
}{
	{"File", []struct{ ID, Title string }{{"new-tab", "New tab"}, {"open-file", "Open file…"}, {"close-tab", "Close tab"}, {"settings", "Settings…"}, {"quit", "Quit"}}},
	{"View", []struct{ ID, Title string }{{"toggle-sidebar", "Toggle sidebar"}, {"toggle-info", "Toggle information"}, {"next-tab", "Next tab"}, {"prev-tab", "Previous tab"}}},
	{"Thread", []struct{ ID, Title string }{{"escape", "Interrupt"}, {"fork", "Fork"}, {"compact", "Compact context"}, {"review", "Review changes…"}, {"export", "Export Markdown…"}}},
	{"Help", []struct{ ID, Title string }{{"about", "About Fastrock"}, {"keyboard", "Keyboard shortcuts"}, {"feedback", "Send feedback…"}, {"open-logs", "Open log folder"}, {"docs", "Codex documentation"}}},
}

func (a *App) drawMenu(w *nucular.Window) {
	w.MenubarBegin()
	w.Row(23).Static(42, 44, 58, 44)
	for _, group := range applicationMenus {
		if menu := w.Menu(label.T(group.Title), 220, nil); menu != nil {
			menu.Row(27).Dynamic(1)
			for _, item := range group.Items {
				if menu.MenuItem(label.T(item.Title)) {
					a.menuAction(item.ID)
				}
			}
		}
	}
	w.MenubarEnd()
}
func (a *App) menuAction(id string) {
	switch id {
	case "quit":
		a.window.Close()
	case "keyboard":
		a.settingsPage("Keyboard")
	case "feedback":
		a.settingsPage("Feedback")
	case "about":
		a.settingsPage("About")
	case "open-logs":
		a.openPath(filepath.Join(codexHome(), "log"), false)
	case "docs":
		a.openURL("https://developers.openai.com/codex")
	case "fork", "compact", "review", "export":
		if t := a.state.Current(); t != nil {
			a.chatAction(t.Target, id)
		}
	default:
		a.runAction(id)
	}
}
