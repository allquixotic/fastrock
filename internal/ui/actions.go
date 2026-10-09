package ui

import (
	"context"
	"encoding/base64"
	"encoding/csv"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
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
	blocks := append([]workspace.Block(nil), c.Blocks...)
	a.choosePath(true, false, func(path string) {
		a.work(func() {
			err := os.WriteFile(path, []byte(transcriptText(blocks)), 0600)
			a.post(func() {
				if err != nil {
					a.report(err)
				} else {
					a.toast = "Exported conversation"
				}
			})
		})
	})
}
func (a *App) exportRally(v *rallyView) {
	items := append([]rally.Object(nil), v.filtered()...)
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
		if a.approvalKey(event) {
			continue
		}
		if a.recordShortcut != "" && event.HandleKeyAny() {
			e := event.Key()
			if e.Code == key.CodeEscape {
				a.recordShortcut = ""
				continue
			}
			if e.Code >= key.CodeLeftControl && e.Code <= key.CodeRightGUI {
				continue
			}
			id := a.recordShortcut
			a.recordShortcut = ""
			binding := settings.KeyBinding{Code: int(e.Code), Mods: uint32(e.Modifiers)}
			apply := func() {
				if a.prefs.Keymap == nil {
					a.prefs.Keymap = map[string]settings.KeyBinding{}
				}
				a.prefs.Keymap[id] = binding
				a.savePrefs()
			}
			conflict := ""
			for _, s := range shellActions() {
				code, mods := s.Code, s.Mods
				if k, ok := a.prefs.Keymap[s.ID]; ok {
					code = key.Code(k.Code)
					mods = key.Modifiers(k.Mods)
				}
				if s.ID != id && code == e.Code && mods == e.Modifiers {
					conflict = s.Title
					break
				}
			}
			if conflict != "" {
				a.confirm("Shortcut conflict", "This also runs "+conflict+". Replace that shortcut?", func() {
					if a.prefs.Keymap == nil {
						a.prefs.Keymap = map[string]settings.KeyBinding{}
					}
					for _, s := range shellActions() {
						k, ok := a.prefs.Keymap[s.ID]
						if !ok {
							k = settings.KeyBinding{Code: int(s.Code), Mods: uint32(s.Mods)}
						}
						if s.ID != id && k == binding {
							a.prefs.Keymap[s.ID] = settings.KeyBinding{}
						}
					}
					apply()
				})
			} else {
				apply()
			}
			continue
		}
		matched := false
		for _, s := range shellActions() {
			code, mods := s.Code, s.Mods
			if k, ok := a.prefs.Keymap[s.ID]; ok {
				if k.Code == 0 {
					continue
				}
				code = key.Code(k.Code)
				mods = key.Modifiers(k.Mods)
			}
			if event.HandleKey(code, mods) {
				a.runAction(s.ID)
				matched = true
				break
			}
		}
		if matched {
			continue
		}
		tab := a.state.Current()
		if tab == nil {
			continue
		}
		if tab.Kind == workspace.Rally {
			v := a.rallyViews[tab.ID]
			if v == nil {
				continue
			}
			if event.HandleKey(key.CodeR, primaryKey) {
				a.refreshRally(v)
			}
			if event.HandleKey(key.CodeS, primaryKey) && v.Detail != nil {
				a.saveDetail(v)
			}
		}
		if tab.Kind == workspace.File {
			v := a.files[tab.ID]
			if v != nil {
				if event.HandleKey(key.CodeF, primaryKey) {
					v.FindOpen = true
				}
				if event.HandleKey(key.CodeG, primaryKey) {
					a.fileFind(v, false)
				}
			}
		}
		if tab.Kind == workspace.Chat {
			c, v := a.state.Chats[tab.Target], a.chats[tab.Target]
			if c != nil && v != nil && v.Editor.Active && len(v.Suggest) > 0 {
				if event.HandleKey(key.CodeUpArrow, 0) {
					v.SuggestIndex = max(0, v.SuggestIndex-1)
					continue
				}
				if event.HandleKey(key.CodeDownArrow, 0) {
					v.SuggestIndex = min(len(v.Suggest)-1, v.SuggestIndex+1)
					continue
				}
				if event.HandleKey(key.CodeTab, 0) {
					acceptSuggestion(v)
					continue
				}
			}
			if c != nil && v != nil && v.Editor.Active && event.HandleKey(key.CodeV, primaryKey) {
				a.pasteComposer(v)
				continue
			}
			if c != nil && v != nil && v.Editor.Active && a.prefs.EnterSends && event.HandleKey(key.CodeReturnEnter, 0) {
				a.send(c, "send")
			}
			if c != nil && v != nil && v.Editor.Active && event.HandleKey(key.CodeReturnEnter, primaryKey) {
				a.send(c, "send")
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

func (a *App) openLink(target string) {
	if strings.HasPrefix(target, "http://") || strings.HasPrefix(target, "https://") {
		a.openURL(target)
	} else {
		path, line, column := fileLocation(target)
		if path == "" {
			a.toast = "Unsupported link"
			return
		}
		if !filepath.IsAbs(path) {
			cwd := a.prefs.WorkingDirectory
			if tab := a.state.Current(); tab != nil {
				if c := a.state.Chats[tab.Target]; c != nil {
					cwd = c.Cwd
				}
			}
			path = filepath.Join(cwd, path)
		}
		a.openFile(path)
		if tab := a.state.Current(); tab != nil && line > 0 {
			if v := a.files[tab.ID]; v != nil {
				v.PendingLine, v.PendingColumn = line, column
				a.goToFileLine(v, line, column)
			}
		}
	}
}

func fileLocation(target string) (string, int, int) {
	line, column := 0, 1
	if strings.HasPrefix(target, "file://") {
		u, err := url.Parse(target)
		if err != nil {
			return "", 0, 0
		}
		target = u.Path
		if u.Host != "" {
			target = "//" + u.Host + target
		}
		if len(target) > 3 && target[0] == '/' && target[2] == ':' {
			target = target[1:]
		}
		if u.Fragment != "" {
			target += "#" + u.Fragment
		}
	} else if strings.Contains(target, "://") {
		return "", 0, 0
	}
	if i := strings.LastIndex(target, "#L"); i >= 0 {
		location := target[i+2:]
		target = target[:i]
		if j := strings.IndexByte(location, 'C'); j >= 0 {
			column, _ = strconv.Atoi(location[j+1:])
			location = location[:j]
		}
		if j := strings.IndexByte(location, '-'); j >= 0 {
			location = location[:j]
		}
		line, _ = strconv.Atoi(location)
	} else if i := strings.LastIndexByte(target, ':'); i >= 0 {
		if n, err := strconv.Atoi(target[i+1:]); err == nil && n > 0 {
			line = n
			target = target[:i]
			if j := strings.LastIndexByte(target, ':'); j >= 0 {
				if n, err := strconv.Atoi(target[j+1:]); err == nil && n > 0 {
					column, line = line, n
					target = target[:j]
				}
			}
		}
	}
	return target, max(0, line), max(1, column)
}
