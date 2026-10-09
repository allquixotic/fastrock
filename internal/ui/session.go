package ui

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

// Only local presentation state and unsent drafts live here. Codex owns history.
type session struct {
	Mailbox   map[string][]mailMessage
	Documents []tabTransfer
	Tabs      []workspace.Tab
	Active    string
	Counter   int
	Chats     map[string]*workspace.Conversation
}

func (a *App) loadSession() {
	b, err := os.ReadFile(filepath.Join(a.store.Dir, "session.json"))
	if err != nil {
		return
	}
	var s session
	if json.Unmarshal(b, &s) != nil {
		return
	}
	a.state.Tabs, a.state.Active, a.state.Counter = s.Tabs, s.Active, s.Counter
	a.mailbox = s.Mailbox
	for id, c := range s.Chats {
		if c == nil || id != c.ID {
			continue
		}
		c.Status, c.TurnID = "idle", ""
		a.state.Chats[id] = c
		a.rememberDraft(id)
	}
	for _, doc := range s.Documents {
		a.installTransfer(doc)
	}
	a.state.Active = s.Active
	// Pop-outs from the preceding run rejoin the primary workspace on restart.
	paths, _ := filepath.Glob(filepath.Join(a.store.Dir, "session-popout-*.json"))
	for _, path := range paths {
		if data, err := os.ReadFile(path); err == nil {
			var extra session
			if json.Unmarshal(data, &extra) == nil {
				a.importedSessions = append(a.importedSessions, path)
				for id, c := range extra.Chats {
					if c != nil {
						a.state.Chats[id] = c
						a.rememberDraft(id)
					}
				}
				for _, t := range extra.Tabs {
					found := false
					for _, prior := range a.state.Tabs {
						if prior.ID == t.ID {
							found = true
							break
						}
					}
					if !found {
						a.state.Tabs = append(a.state.Tabs, t)
					}
				}
				for _, doc := range extra.Documents {
					a.installTransfer(doc)
				}
			}
		}
	}
	a.state.Active = s.Active
}
func (a *App) sessionSnapshot() session {
	s := session{Mailbox: map[string][]mailMessage{}, Tabs: append([]workspace.Tab(nil), a.state.Tabs...), Active: a.state.Active, Counter: a.state.Counter, Chats: map[string]*workspace.Conversation{}}
	for id, messages := range a.mailbox {
		s.Mailbox[id] = append([]mailMessage(nil), messages...)
	}
	candidates := make(map[string]bool, len(a.draftChats)+len(a.chats)+len(a.state.Tabs))
	for id := range a.draftChats {
		candidates[id] = true
	}
	for id := range a.chats {
		candidates[id] = true
	}
	for _, t := range a.state.Tabs {
		if t.Kind == workspace.Chat {
			candidates[t.Target] = true
		}
	}
	for id := range candidates {
		c := a.state.Chats[id]
		if c == nil || c.Ephemeral {
			continue
		}
		local := *c
		local.Queue = cloneQueue(c.Queue)
		local.DraftAttachments = append([]string(nil), c.DraftAttachments...)
		local.Blocks, local.TurnID, local.Status = nil, "", "idle"
		if v := a.chats[id]; v != nil {
			local.Draft = text(v.Editor)
			local.DraftAttachments = append([]string(nil), v.Attachments...)
		}
		s.Chats[id] = &local
	}
	s.Tabs = nil
	for _, t := range a.state.Tabs {
		if c := a.state.Chats[t.Target]; c != nil && c.Ephemeral {
			continue
		}
		s.Tabs = append(s.Tabs, t)
		if t.Kind == workspace.Rally || t.Kind == workspace.File {
			doc := a.tabSnapshot(t)
			doc.Chat = nil
			if doc.File != nil && !doc.File.Virtual {
				doc.File.Text = ""
				doc.File.TextLoaded = false
			}
			s.Documents = append(s.Documents, doc)
		}
	}
	return s
}
func (a *App) saveSession() {
	if a.store.WriteJSON(a.sessionFile(), a.sessionSnapshot()) == nil {
		a.removeImportedSessions()
	}
}
func (a *App) checkpoint() {
	if time.Since(a.lastCheckpoint) < time.Second {
		return
	}
	a.lastCheckpoint = time.Now()
	a.publishThreads()
	snapshot := a.sessionSnapshot()
	select {
	case a.sessions <- snapshot:
	default:
		select {
		case <-a.sessions:
		default:
		}
		select {
		case a.sessions <- snapshot:
		default:
		}
	}

}
func (a *App) publishThreads() {
	rows := []codex.OpenThread{}
	for _, tab := range a.state.Tabs {
		if tab.Kind == workspace.Chat {
			if c := a.state.Chats[tab.Target]; c != nil {
				rows = append(rows, codex.OpenThread{ID: c.ID, Title: c.Title, Cwd: c.Cwd, Status: c.Status, AcceptsMessages: !c.NoMessages && !c.EphemeralLost})
			}
		}
	}
	if slices.Equal(rows, a.openThreads) || a.client == nil {
		return
	}
	a.openThreads = rows
	client := a.client
	a.work(func() { _ = client.Notify("fastrock/publishThreads", map[string]any{"data": rows}) })
}
func (a *App) restoreDocuments() {
	active := a.state.Active
	tabs := append([]workspace.Tab(nil), a.state.Tabs...)
	for _, t := range tabs {
		switch t.Kind {
		case workspace.Rally:
			a.openRally(t.Page)
		case workspace.File:
			if a.files[t.ID] == nil {
				a.openFile(t.Target)
			}
		case workspace.Settings:
			a.openSettings()
		case workspace.Chat:
			if t.ID == active {
				a.resumeThread(t.Target)
			}
		}
	}
	a.state.Active = active
}
func (a *App) reconnect() {
	if a.connecting {
		return
	}
	a.connecting = true
	a.status = "Connecting Codex…"
	cwd := a.prefs.WorkingDirectory
	a.work(func() {
		c, err := codex.Dial(a.ctx, a.connection.Address, a.connection.Token)
		a.post(func() {
			a.connecting = false
			if err != nil {
				a.report(err)
				a.status = "Codex disconnected"
				return
			}
			a.client = c
			a.openThreads = nil
			a.publishThreads()
			a.status = c.Version + " · Connected"
			for id, v := range a.chats {
				if chat := a.state.Chats[id]; chat != nil {
					chat.Draft = text(v.Editor)
					chat.DraftAttachments = v.Attachments
				}
				delete(a.chats, id)
			}
			a.restoreDocuments()
			go a.consume(c)
			a.work(func() { a.loadCatalog(c, cwd); a.loadThreads(c, false) })
		})
	})
}

func (a *App) sessionFile() string {
	if a.sessionName == "" {
		return "session.json"
	}
	return a.sessionName
}

func cloneKeys(in map[string]settings.KeyBinding) map[string]settings.KeyBinding {
	if in == nil {
		return nil
	}
	out := make(map[string]settings.KeyBinding, len(in))
	for k, v := range in {
		out[k] = v
	}
	return out
}

func cloneQueue(in []workspace.Draft) []workspace.Draft {
	out := append([]workspace.Draft(nil), in...)
	for i := range out {
		out[i].Attachments = append([]string(nil), in[i].Attachments...)
	}
	return out
}
func (a *App) removeImportedSessions() {
	for _, path := range a.importedSessions {
		_ = os.Remove(path)
	}
	a.importedSessions = nil
}

func (a *App) rememberFolder(folder string) {
	if folder == "" {
		return
	}
	next := []string{folder}
	for _, s := range a.prefs.RecentFolders {
		if s != folder && len(next) < 12 {
			next = append(next, s)
		}
	}
	a.prefs.RecentFolders = next
}
func (a *App) forgetFolder(folder string) {
	out := make([]string, 0, len(a.prefs.RecentFolders))
	for _, s := range a.prefs.RecentFolders {
		if s != folder {
			out = append(out, s)
		}
	}
	a.prefs.RecentFolders = out
	a.savePrefs()
}

func (a *App) rememberDraft(id string) {
	c := a.state.Chats[id]
	if c == nil || (c.Draft == "" && len(c.DraftAttachments) == 0 && len(c.Queue) == 0) {
		delete(a.draftChats, id)
		return
	}
	if a.draftChats == nil {
		a.draftChats = make(map[string]bool)
	}
	a.draftChats[id] = true
}
