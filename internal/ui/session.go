package ui

import (
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

// Only local presentation state and unsent drafts live here. Codex owns history.
type session struct {
	Tabs    []workspace.Tab
	Active  string
	Counter int
	Chats   map[string]*workspace.Conversation
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
	for id, c := range s.Chats {
		if c == nil || id != c.ID {
			continue
		}
		c.Status, c.TurnID = "idle", ""
		a.state.Chats[id] = c
	}
	// Reopen external resources after the connection is ready.
}
func (a *App) sessionBytes() []byte {
	s := session{Tabs: a.state.Tabs, Active: a.state.Active, Counter: a.state.Counter, Chats: map[string]*workspace.Conversation{}}
	for id, c := range a.state.Chats {
		local := *c
		local.Blocks, local.TurnID, local.Status = nil, "", "idle"
		if v := a.chats[id]; v != nil {
			local.Draft = text(v.Editor)
			local.DraftAttachments = v.Attachments
		}
		s.Chats[id] = &local
	}
	b, _ := json.Marshal(s)
	return b
}
func (a *App) saveSession() {
	b := a.sessionBytes()
	_ = a.store.WriteJSON("session.json", json.RawMessage(b))
}
func (a *App) checkpoint() {
	if time.Since(a.lastCheckpoint) < time.Second {
		return
	}
	a.lastCheckpoint = time.Now()
	b := a.sessionBytes()
	if string(b) == a.lastSession {
		return
	}
	a.lastSession = string(b)
	a.writes <- func() { _ = a.store.WriteJSON("session.json", json.RawMessage(b)) }
}
func (a *App) restoreDocuments() {
	active := a.state.Active
	tabs := append([]workspace.Tab(nil), a.state.Tabs...)
	for _, t := range tabs {
		switch t.Kind {
		case workspace.Rally:
			a.openRally(t.Page)
		case workspace.File:
			a.openFile(t.Target)
		case workspace.Settings:
			a.openSettings()
		case workspace.Chat:
			a.resumeThread(t.Target)
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
		c, err := codex.Start(a.ctx)
		a.post(func() {
			a.connecting = false
			if err != nil {
				a.report(err)
				a.status = "Codex disconnected"
				return
			}
			a.client = c
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
