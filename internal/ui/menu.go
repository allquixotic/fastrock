package ui

import (
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"path/filepath"
)

var applicationMenus = []struct {
	Title string
	Items []struct{ ID, Title string }
}{
	{"File", []struct{ ID, Title string }{{"new-tab", "New tab"}, {"open-file", "Open file…"}, {"close-tab", "Close tab"}, {"settings", "Settings…"}, {"quit", "Quit"}}},
	{"View", []struct{ ID, Title string }{{"toggle-sidebar", "Toggle sidebar"}, {"toggle-info", "Toggle information"}, {"toggle-status", "Show/hide status bar"}, {"next-tab", "Next tab"}, {"prev-tab", "Previous tab"}}},
	{"Thread", []struct{ ID, Title string }{{"escape", "Interrupt"}, {"fork", "Fork"}, {"compact", "Compact context"}, {"review", "Review changes…"}, {"export", "Export Markdown…"}}},
	{"Help", []struct{ ID, Title string }{{"about", "About Fastrock"}, {"updates", "Check for updates…"}, {"keyboard", "Keyboard shortcuts"}, {"feedback", "Send feedback…"}, {"open-logs", "Open log folder"}, {"docs", "Codex documentation"}}},
}

func (a *App) drawMenu(w *desktop.Window) {
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
	case "updates":
		a.showUpdates()
	case "toggle-status":
		a.prefs.StatusBar = !a.prefs.StatusBar
		a.savePrefs()
	case "quit":
		a.requestQuit()
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
