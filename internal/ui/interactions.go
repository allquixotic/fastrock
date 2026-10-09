package ui

import (
	"encoding/json"
	"image"
	"path/filepath"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

type actionSpec struct {
	ID, Title string
	Code      key.Code
	Mods      key.Modifiers
}

func shellActions() []actionSpec {
	m := platform.PrimaryModifier()
	return []actionSpec{
		{"new-tab", "New tab", key.CodeT, m}, {"close-tab", "Close tab", key.CodeW, m}, {"next-tab", "Next tab", key.CodeTab, key.ModControl}, {"prev-tab", "Previous tab", key.CodeTab, key.ModControl | key.ModShift},
		{"settings", "Settings", key.CodeComma, m}, {"toggle-sidebar", "Toggle sidebar", key.CodeB, m}, {"toggle-info", "Toggle information", key.CodeI, m | key.ModShift}, {"open-file", "Open file", key.CodeO, m},
		{"rally-search", "Rally: Focus search", key.CodeSlash, 0}, {"rally-team-board", "Rally: Open Team Board", key.CodeB, key.ModAlt},
		{"rally-close-detail", "Rally: Close work item", key.CodeEscape, 0}, {"rally-open-card", "Rally: Open focused work item", key.CodeReturnEnter, 0},
		{"rally-next-card", "Rally: Focus next work item", key.CodeTab, 0}, {"rally-prev-card", "Rally: Focus previous work item", key.CodeTab, key.ModShift},
		{"escape", "Interrupt / dismiss", key.CodeEscape, 0}, {"palette", "Command palette", key.CodeP, m | key.ModShift}, {"theme", "Toggle dark / light", key.CodeT, m | key.ModShift},
		{"tab-1", "Select tab 1", key.Code1, m}, {"tab-2", "Select tab 2", key.Code2, m}, {"tab-3", "Select tab 3", key.Code3, m}, {"tab-4", "Select tab 4", key.Code4, m}, {"tab-5", "Select tab 5", key.Code5, m}, {"tab-6", "Select tab 6", key.Code6, m}, {"tab-7", "Select tab 7", key.Code7, m}, {"tab-8", "Select tab 8", key.Code8, m}, {"tab-9", "Select last tab", key.Code9, m},
	}
}
func (a *App) runAction(id string) {
	if a.runRallyAction(id) {
		return
	}
	switch id {
	case "new-tab":
		a.state.OpenNew()
	case "close-tab":
		a.closeTab(a.state.Active)
	case "next-tab":
		a.nextTab(1)
	case "prev-tab":
		a.nextTab(-1)
	case "settings":
		a.openSettings()
	case "open-file":
		a.choosePath(false, false, a.openFile)
	case "toggle-sidebar":
		a.prefs.Sidebar = !a.prefs.Sidebar
		a.savePrefs()
	case "toggle-info":
		a.prefs.Info = !a.prefs.Info
		a.savePrefs()
	case "palette":
		a.paletteOpen = true
	case "theme":
		if a.prefs.Theme == "dark" {
			a.prefs.Theme = "light"
		} else {
			a.prefs.Theme = "dark"
		}
		a.theme()
	case "escape":
		a.paletteOpen = false
		if t := a.state.Current(); t != nil {
			if v := a.chats[t.Target]; v != nil && v.RichSelection.BlockID != "" {
				v.RichSelection = transcriptSelection{}
				return
			}
			if c := a.state.Chats[t.Target]; c != nil && c.Busy() {
				a.rpc("turn/interrupt", map[string]any{"threadId": c.ID, "turnId": c.TurnID}, nil)
			}
		}
	default:
		if strings.HasPrefix(id, "tab-") {
			i := int(id[len(id)-1] - '1')
			if i == 8 {
				i = len(a.state.Tabs) - 1
			}
			if i >= 0 && i < len(a.state.Tabs) {
				a.state.Active = a.state.Tabs[i].ID
			}
		}
	}
}

var chatActions = []struct{ ID, Title string }{
	{"rename", "Rename…"}, {"fork", "Fork conversation"}, {"side", "Side chat…"}, {"worktree", "Continue in worktree…"}, {"recap", "Recap conversation"}, {"compact", "Compact context"}, {"review", "Review changes…"}, {"init", "Create AGENTS.md"}, {"export", "Export Markdown…"}, {"view-text", "View as text"}, {"copy-id", "Copy thread ID"}, {"archive", "Archive"},
}

func (a *App) chatTabMenu(w *desktop.Window, t workspace.Tab) {
	for _, item := range chatActions {
		if w.MenuItem(label.T(item.Title)) {
			a.chatAction(t.Target, item.ID)
		}
	}
}
func (a *App) copyText(value string) { a.clipboard = value; a.toast = "Copied" }
func (a *App) chatAction(id, action string) {
	c := a.state.Chats[id]
	if c == nil {
		return
	}
	if c.Ephemeral && (action == "rename" || action == "fork" || action == "side" || action == "archive" || action == "worktree") {
		a.toast = "Ephemeral side conversations cannot be renamed, forked, archived, or attached to worktrees"
		return
	}
	switch action {
	case "side":
		a.startSide(c)
	case "rename":
		a.inputDialog("Rename conversation", c.Title, func(name string) {
			if strings.TrimSpace(name) == "" {
				return
			}
			a.rpc("thread/name/set", map[string]any{"threadId": id, "name": name}, func(_ json.RawMessage) {
				a.invalidateSidebar(c.ID)
				c.Title = name
				for i := range a.state.Tabs {
					if a.state.Tabs[i].Target == id {
						a.state.Tabs[i].Title = name
					}
				}
			})
		})
	case "fork":
		a.rpc("thread/fork", map[string]any{"threadId": id}, func(raw json.RawMessage) {
			r := codex.Decode(raw)
			t, _ := r["thread"].(map[string]any)
			next := str(t, "id")
			if next == "" {
				return
			}
			title := threadTitle(t)
			if action == "side" {
				title = "Side chat · " + c.Title
			}
			a.state.Chats[next] = &workspace.Conversation{ID: next, Title: title, Cwd: c.Cwd, Updated: time.Now().Unix()}
			a.resumeThread(next)
			if action == "side" {
				a.inputDialog("Side question", "", func(value string) {
					if cv := a.chats[next]; cv != nil {
						setText(cv.Editor, value)
						a.send(a.state.Chats[next], "send")
					}
				})
			}
		})
	case "compact":
		a.rpc("thread/compact/start", map[string]any{"threadId": id}, nil)
	case "recap":
		a.startRecap(c)
	case "init":
		a.startTurn(c, "Create or update AGENTS.md with concise, repository-specific instructions for coding agents. Inspect the repository and document its actual build, testing, architecture and conventions.", nil, "send")
	case "review":
		a.reviewDialog(id)
	case "export":
		a.exportChat(c)
	case "view-text":
		a.viewChatText(c)
	case "copy-id":
		a.copyText(id)
	case "archive", "unarchive":
		method := "thread/" + action
		a.rpc(method, map[string]any{"threadId": id}, func(_ json.RawMessage) {
			a.invalidateSidebar(c.ID)
			c.Archived = action == "archive"
			for _, t := range append([]workspace.Tab(nil), a.state.Tabs...) {
				if t.Target == id {
					a.closeTab(t.ID)
				}
			}
		})
	case "delete":
		a.confirm("Delete conversation?", "Permanently delete this conversation and its saved history?", func() {
			a.rpc("thread/delete", map[string]any{"threadId": id}, func(_ json.RawMessage) {
				for _, t := range append([]workspace.Tab(nil), a.state.Tabs...) {
					if t.Target == id {
						a.closeTab(t.ID)
					}
				}
				delete(a.state.Chats, id)
				a.invalidateSidebar(id)
			})
		})
	case "worktree":
		a.choosePath(false, true, func(path string) { a.continueWorktree(c, path) })
	}
}
func (a *App) openText(title, content string) {
	id := a.state.Open(workspace.File, title, "text:"+title, "")
	v := &fileView{Path: title, Find: textEditor("", false), Wrap: true, Virtual: true, Loading: true}
	a.files[id] = v
	a.work(func() {
		e := textEditor(cut(content, maxFileBytes), true)
		e.Flags |= desktop.EditReadOnly
		a.post(func() {
			if a.files[id] == v {
				v.Editor = e
				v.Loading = false
			}
		})
	})
}
func (a *App) sidebarContext(w *desktop.Window, c *workspace.Conversation) {
	if menu := w.ContextualOpen(0, image.Pt(230, 220), w.LastWidgetBounds, nil); menu != nil {
		menu.Row(28).Dynamic(1)
		if menu.MenuItem(label.T("Open")) {
			a.resumeThread(c.ID)
		}
		if menu.MenuItem(label.T("New chat in this folder")) {
			a.newThread(c.Cwd)
		}
		menu.Row(7).Dynamic(1)
		menu.Spacing(1)
		menu.Row(28).Dynamic(1)
		if menu.MenuItem(label.T("Rename…")) {
			a.chatAction(c.ID, "rename")
		}
		action, caption := "archive", "Archive"
		if c.Archived {
			action, caption = "unarchive", "Unarchive"
		}
		if menu.MenuItem(label.T(caption)) {
			a.chatAction(c.ID, action)
		}
		if menu.MenuItem(label.T("Delete permanently…")) {
			a.chatAction(c.ID, "delete")
		}
		menu.Row(7).Dynamic(1)
		menu.Spacing(1)
		menu.Row(28).Dynamic(1)
		if menu.MenuItem(label.T("Copy thread ID")) {
			a.copyText(c.ID)
		}
	}
}
func (a *App) folderContext(w *desktop.Window, path string) {
	if menu := w.ContextualOpen(0, image.Pt(220, 120), w.LastWidgetBounds, nil); menu != nil {
		if menu.MenuItem(label.T("New conversation")) {
			a.newThread(path)
		}
		if menu.MenuItem(label.T("Open folder")) {
			a.openPath(path, false)
		}
		if menu.MenuItem(label.T("Copy path")) {
			a.copyText(filepath.Clean(path))
		}
	}
}
