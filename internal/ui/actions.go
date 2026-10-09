package ui

import (
	"context"
	"encoding/base64"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
	"unicode"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
)

func (a *App) inputDialog(title, value string, accept func(string)) {
	ed := textEditor(value, false)
	a.window.PopupOpen(title, desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(520, 165), false, func(w *desktop.Window) {
		w.Row(34).Dynamic(1)
		ed.Edit(w)
		w.Row(30).Dynamic(2)
		if w.ButtonText("Cancel") {
			w.Close()
		}
		apply := primary(w, dialogAction(title, "Save"), a.p)
		for event := range w.Input().Keyboard.Events() {
			if event.HandleKey(key.CodeReturnEnter, 0) {
				apply = true
			}
		}
		if apply {
			value := text(ed)
			w.Close()
			a.post(func() { accept(value) })
		}
	})
}
func (a *App) confirm(title, message string, accept func()) {
	a.window.PopupOpen(title, desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(520, 210), false, func(w *desktop.Window) {
		w.Row(max(55, w.LayoutAvailableHeight()-42)).Dynamic(1)
		w.LabelWrap(message)
		w.Row(30).Dynamic(2)
		if w.ButtonText("Cancel") {
			w.Close()
		}
		action := dialogAction(title, "Continue")
		apply := false
		if action == "Delete" || action == "Run" || action == "Stop" {
			apply = dangerButton(w, action, a.p)
		} else {
			apply = primary(w, action, a.p)
		}
		for event := range w.Input().Keyboard.Events() {
			if event.HandleKey(key.CodeReturnEnter, 0) {
				apply = true
			}
		}
		if apply {
			w.Close()
			a.post(accept)
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
	if e != nil || (u.Scheme != "https" && u.Scheme != "http" && !(u.Scheme == "mailto" && u.Opaque != "")) {
		a.toast = "Only HTTP, HTTPS and email links can be opened"
		return
	}
	a.work(func() {
		if e := platform.OpenURL(target); e != nil {
			a.post(func() { a.report(e) })
		}
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
func (a *App) shortcuts(w *desktop.Window) {
	primaryKey := platform.PrimaryModifier()
	if v := a.currentRally(); v != nil && w.Input().Mouse.Pressed(mouse.ButtonLeft) {
		v.cardFocusActive = false
	}
	var searchShortcut *rallyView
	for event := range w.Input().Keyboard.Events() {
		// Native input carries typing keys and their text separately. The slash
		// that focuses search must not also become part of the search query.
		if searchShortcut != nil && event.HandleText() {
			value := event.Text()
			if strings.HasPrefix(value, "/") {
				if rest := strings.TrimPrefix(value, "/"); rest != "" {
					searchShortcut.Search.Paste(rest)
				}
				searchShortcut = nil
				continue
			} else {
				event.Unhandle()
			}
			searchShortcut = nil
		}
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
			if problem := validateActionShortcut(a.recordShortcut, e.Code, e.Modifiers); problem != "" {
				a.toast = problem
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
				if s.ID != id && actionsOverlap(id, s.ID) && code == e.Code && mods == e.Modifiers {
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
						if s.ID != id && actionsOverlap(id, s.ID) && k == binding {
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
			if s.ID == "escape" {
				// Leave Escape available to the focused popup/editor unless
				// there is an application action that can actually handle it.
				t := a.state.Current()
				handled := a.paletteOpen
				if t != nil {
					if c := a.state.Chats[t.Target]; c != nil {
						handled = handled || c.Busy()
					}
					if v := a.chats[t.Target]; v != nil {
						if len(v.Suggest) > 0 && event.HandleKey(key.CodeEscape, 0) {
							v.Suggest = nil
							matched = true
							break
						}
						handled = handled || v.RichSelection.BlockID != ""
					}
				}
				if !handled {
					continue
				}
			}
			code, mods := s.Code, s.Mods
			if k, ok := a.prefs.Keymap[s.ID]; ok {
				if k.Code == 0 {
					continue
				}
				code = key.Code(k.Code)
				mods = key.Modifiers(k.Mods)
			}
			if event.HandleKey(code, mods) {
				if strings.HasPrefix(s.ID, "rally-") && !a.rallyShortcutApplies(s.ID, code, mods) {
					event.Unhandle()
					continue
				}
				if s.ID == "rally-search" && code == key.CodeSlash && mods == 0 {
					searchShortcut = a.currentRally()
				}
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
				a.fileSearchKey(v, event, primaryKey)
				if event.HandleKey(key.CodeG, primaryKey) {
					a.fileGoTo(v)
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
				if event.HandleKey(key.CodeTab, 0) || event.HandleKey(key.CodeReturnEnter, 0) {
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
	a.window.PopupOpen("Command palette", desktop.WindowTitle|desktop.WindowMovable|desktop.WindowClosable, rect.Rect{X: 330, Y: 130, W: 660, H: 520}, true, func(w *desktop.Window) {
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
	cwd := a.prefs.WorkingDirectory
	if tab := a.state.Current(); tab != nil {
		if c := a.state.Chats[tab.Target]; c != nil {
			cwd = c.Cwd
		}
	}
	a.openLinkAt(target, cwd)
}

func (a *App) openLinkAt(target, cwd string) {
	target = strings.TrimSpace(target)
	if externalTranscriptLink(target) {
		a.openURL(target)
	} else {
		path, line, column := fileLocation(target)
		if path == "" {
			a.toast = "Unsupported link"
			return
		}
		if strings.HasPrefix(path, "~/") || strings.HasPrefix(path, `~\`) {
			if home, err := os.UserHomeDir(); err == nil {
				path = filepath.Join(home, path[2:])
			}
		}
		if !filepath.IsAbs(path) {
			path = filepath.Join(fallback(cwd, a.prefs.WorkingDirectory), path)
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
	target = strings.TrimSpace(target)
	if target == "" || strings.ContainsFunc(target, unicode.IsControl) {
		return "", 0, 0
	}
	line, column := 0, 1
	fragment := ""
	if strings.HasPrefix(strings.ToLower(target), "file:") {
		u, err := url.Parse(target)
		if err != nil || u.Opaque != "" || u.Path == "" || u.User != nil {
			return "", 0, 0
		}
		target = u.Path
		if u.Host != "" && !strings.EqualFold(u.Host, "localhost") {
			target = "//" + u.Host + target
		}
		if len(target) > 3 && target[0] == '/' && target[2] == ':' {
			target = target[1:]
		}
		fragment = u.Fragment
	} else {
		if i := strings.LastIndexByte(target, '#'); i >= 0 {
			fragment, target = target[i+1:], target[:i]
		}
		var err error
		target, err = url.PathUnescape(target)
		if err != nil {
			return "", 0, 0
		}
	}
	if strings.HasPrefix(fragment, "L") {
		location := strings.Replace(fragment[1:], "-L", "-", 1)
		if j := strings.IndexByte(location, 'C'); j >= 0 {
			column, _ = strconv.Atoi(location[j+1:])
			location = location[:j]
		}
		line, _ = fileLineNumber(location)
	}
	// Parse suffixes even with a fragment; an explicit #L location wins.
	path, suffixLine, suffixColumn := splitFileLocation(target)
	if line == 0 {
		line, column = suffixLine, suffixColumn
	}
	if path == "" || strings.ContainsFunc(path, unicode.IsControl) || strings.Contains(path, "://") {
		return "", 0, 0
	}
	if i := strings.IndexByte(path, ':'); i >= 0 && !(i == 1 && len(path) > 2 && (path[2] == '/' || path[2] == '\\') && (path[0] >= 'a' && path[0] <= 'z' || path[0] >= 'A' && path[0] <= 'Z')) {
		return "", 0, 0
	}
	return path, max(0, line), max(1, column)
}

func splitFileLocation(target string) (string, int, int) {
	line, column := 0, 1
	if i := strings.LastIndexByte(target, ':'); i >= 0 {
		if n, ok := fileLineNumber(target[i+1:]); ok {
			line = n
			target = target[:i]
			if j := strings.LastIndexByte(target, ':'); j >= 0 {
				if n, ok := fileLineNumber(target[j+1:]); ok {
					column, line = line, n
					target = target[:j]
				}
			}
		}
	}
	return target, line, column
}

func fileLineNumber(value string) (int, bool) {
	first, last, ranged := strings.Cut(value, "-")
	n, err := strconv.Atoi(first)
	if err != nil || n < 1 {
		return 0, false
	}
	if ranged {
		end, err := strconv.Atoi(last)
		if err != nil || end < n {
			return 0, false
		}
	}
	return n, true
}
