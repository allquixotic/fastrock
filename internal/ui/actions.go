package ui

import (
	"context"
	"encoding/base64"
	"encoding/csv"
	"fmt"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

func labelText(s string) label.Label { return label.T(s) }
func (a *App) inputDialog(title, value string, accept func(string)) {
	ed := textEditor(value, false)
	a.window.PopupOpen(title, nucular.WindowTitle|nucular.WindowMovable, rect.Rect{X: 360, Y: 240, W: 490, H: 150}, true, func(w *nucular.Window) {
		w.Row(34).Dynamic(1)
		ed.Edit(w)
		w.Row(30).Dynamic(2)
		if primary(w, "Save", a.p) {
			accept(text(ed))
			w.Close()
		}
		if w.ButtonText("Cancel") {
			w.Close()
		}
	})
}
func (a *App) confirm(title, message string, accept func()) {
	a.window.PopupOpen(title, nucular.WindowTitle|nucular.WindowMovable, rect.Rect{X: 360, Y: 240, W: 490, H: 165}, true, func(w *nucular.Window) {
		w.Row(65).Dynamic(1)
		w.LabelWrap(message)
		w.Row(30).Dynamic(2)
		if primary(w, "Confirm", a.p) {
			accept()
			w.Close()
		}
		if w.ButtonText("Cancel") {
			w.Close()
		}
	})
}
func (a *App) choosePath(save, dir bool, accept func(string)) {
	a.work(func() {
		path, e := platform.ChoosePath(save, dir)
		a.post(func() {
			if e != nil {
				a.report(e)
			} else if path != "" {
				accept(path)
			}
		})
	})
}
func (a *App) attachDialog(v *chatView) {
	a.choosePath(false, false, func(path string) { v.Attachments = append(v.Attachments, path) })
}
func (a *App) openURL(target string) {
	u, e := url.Parse(target)
	if e != nil || (u.Scheme != "https" && u.Scheme != "http") {
		a.toast = "Only HTTP and HTTPS links may open in the browser"
		return
	}
	a.work(func() {
		if e := platform.OpenURL(target); e != nil {
			a.post(func() { a.report(e) })
		}
	})
}
func (a *App) exportChat(c *workspace.Conversation) {
	var b strings.Builder
	for _, block := range c.Blocks {
		fmt.Fprintf(&b, "## %s\n\n%s\n\n", block.Role, block.Text)
	}
	data := []byte(b.String())
	a.choosePath(true, false, func(path string) {
		a.work(func() {
			e := os.WriteFile(path, data, 0600)
			a.post(func() {
				if e != nil {
					a.report(e)
				} else {
					a.toast = "Exported conversation"
				}
			})
		})
	})
}
func (a *App) exportRally(v *rallyView) {
	items := v.filtered()
	columns := append([]string{}, v.Columns...)
	a.choosePath(true, false, func(path string) {
		a.work(func() {
			f, e := os.Create(path)
			if e == nil {
				writer := csv.NewWriter(f)
				e = writer.Write(columns)
				for _, o := range items {
					record := []string{}
					for _, k := range columns {
						cell := o.String(k)
						if len(cell) > 0 && strings.ContainsAny(cell[:1], "=+-@\t\r") {
							cell = "'" + cell
						}
						record = append(record, cell)
					}
					if e == nil {
						e = writer.Write(record)
					}
				}
				writer.Flush()
				if e == nil {
					e = writer.Error()
				}
				ce := f.Close()
				if e == nil {
					e = ce
				}
			}
			a.post(func() {
				if e != nil {
					a.report(e)
				} else {
					a.toast = "Exported Rally work items"
				}
			})
		})
	})
}
func (a *App) downloadAttachment(o rally.Object) {
	c := a.rallyClient
	a.choosePath(true, false, func(path string) {
		a.work(func() {
			ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
			defer cancel()
			content, e := c.Get(ctx, o.Ref("Content"))
			if e == nil {
				var data []byte
				data, e = base64.StdEncoding.DecodeString(content.String("Content"))
				if e == nil {
					e = os.WriteFile(path, data, 0600)
				}
			}
			a.post(func() { a.report(e) })
		})
	})
}
func (a *App) shortcuts(w *nucular.Window) {
	primaryKey := platform.PrimaryModifier()
	for event := range w.Input().Keyboard.Events() {
		if event.HandleKey(key.CodeT, primaryKey|key.ModShift) {
			if a.prefs.Theme == "dark" {
				a.prefs.Theme = "light"
			} else {
				a.prefs.Theme = "dark"
			}
			a.theme()
			continue
		}
		if event.HandleKey(key.CodeP, primaryKey|key.ModShift) {
			a.paletteOpen = !a.paletteOpen
			continue
		}
		if event.HandleKey(key.CodeT, primaryKey) {
			a.state.Open(workspace.New, "New tab", "", "")
			continue
		}
		if event.HandleKey(key.CodeW, primaryKey) {
			a.state.Close(a.state.Active)
			continue
		}
		if event.HandleKey(key.CodeComma, primaryKey) {
			a.openSettings()
			continue
		}
		if event.HandleKey(key.CodeEscape, 0) {
			a.paletteOpen = false
			continue
		}
		tab := a.state.Current()
		if tab == nil {
			continue
		}
		if event.HandleKey(key.CodeTab, key.ModControl) {
			a.nextTab(1)
			continue
		}
		if event.HandleKey(key.CodeTab, key.ModControl|key.ModShift) {
			a.nextTab(-1)
			continue
		}
		if tab.Kind == workspace.Rally {
			v := a.rallyViews[tab.ID]
			if event.HandleKey(key.CodeR, primaryKey) {
				a.refreshRally(v)
			}
			if event.HandleKey(key.CodeS, primaryKey) && v.Detail != nil {
				a.saveDetail(v)
			}
		}
		if tab.Kind == workspace.Chat {
			c := a.state.Chats[tab.Target]
			v := a.chats[tab.Target]
			if c != nil && v != nil && v.Editor.Active && a.prefs.EnterSends {
				if event.HandleKey(key.CodeReturnEnter, 0) {
					a.send(c, "send")
				}
			}
		}
	}
}
func (a *App) nextTab(direction int) {
	for i, t := range a.state.Tabs {
		if t.ID == a.state.Active {
			a.state.Active = a.state.Tabs[(i+direction+len(a.state.Tabs))%len(a.state.Tabs)].ID
			return
		}
	}
}
func (a *App) drawPalette() {
	a.paletteOpen = false
	a.window.PopupOpen("Command palette", nucular.WindowTitle|nucular.WindowMovable|nucular.WindowClosable, rect.Rect{X: 330, Y: 130, W: 660, H: 520}, true, func(w *nucular.Window) {
		for event := range w.Input().Keyboard.Events() {
			if event.HandleKey(key.CodeEscape, 0) {
				w.Close()
				return
			}
		}
		w.Row(32).Dynamic(1)
		a.paletteSearch.Edit(w)
		needle := strings.ToLower(text(a.paletteSearch))
		for _, p := range rally.Pages {
			if strings.Contains(strings.ToLower(p.Title), needle) {
				w.Row(28).Dynamic(1)
				if w.ButtonText("Rally: " + p.Title) {
					a.openRally(p.ID)
					w.Close()
				}
			}
		}
		for _, action := range []string{"New conversation", "Settings", "Choose project folder", "Open file"} {
			if strings.Contains(strings.ToLower(action), needle) {
				w.Row(28).Dynamic(1)
				if w.ButtonText(action) {
					switch action {
					case "New conversation":
						a.newThread(a.prefs.WorkingDirectory)
					case "Settings":
						a.openSettings()
					case "Choose project folder":
						a.choosePath(false, true, func(path string) { a.prefs.WorkingDirectory = path; setText(a.newFolder, path); a.savePrefs() })
					case "Open file":
						a.choosePath(false, false, a.openFile)
					}
					w.Close()
				}
			}
		}
	})
}
