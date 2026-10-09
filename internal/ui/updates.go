package ui

import (
	"encoding/json"
	"fmt"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
)

func (a *App) showUpdates() {
	a.rpc("fastrock/update", map[string]bool{"check": true}, func(raw json.RawMessage) { json.Unmarshal(raw, &a.updateStatus) })
	a.window.PopupOpen("Fastrock updates", desktop.WindowTitle|desktop.WindowMovable|desktop.WindowClosable, rect.Rect{X: 280, Y: 190, W: 510, H: 230}, true, func(w *desktop.Window) {
		w.Row(80).Dynamic(1)
		w.LabelWrap(a.updateStatus.Message)
		if a.updateStatus.Total > 0 && a.updateStatus.State == "downloading" {
			w.Row(25).Dynamic(1)
			w.Label(fmt.Sprintf("%.1f / %.1f MiB", float64(a.updateStatus.Downloaded)/(1<<20), float64(a.updateStatus.Total)/(1<<20)), "LC")
		}
		w.Row(46).Dynamic(1)
		w.LabelWrap("Updates install after all Fastrock windows close, then reopen the app. Your settings and saved drafts stay in place.")
		w.Row(30).Dynamic(2)
		if w.ButtonText("Close") {
			w.Close()
		}
		if a.updateStatus.State == "error" && w.ButtonText("Retry") {
			a.rpc("fastrock/update", map[string]bool{"check": true}, func(raw json.RawMessage) { json.Unmarshal(raw, &a.updateStatus) })
		}
	})
}
