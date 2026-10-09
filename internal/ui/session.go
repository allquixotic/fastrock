package ui

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"slices"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

// Only local presentation state and unsent drafts live here. Codex owns history.
type session struct {
	SchemaVersion int `json:"schemaVersion,omitempty"`
	Mailbox       map[string][]mailMessage
	Documents     []tabTransfer
	Tabs          []workspace.Tab
	Active        string
	Counter       int
	Chats         map[string]*workspace.Conversation
}

const sessionSchemaVersion = 3 // validated records and document presentation

type loadedSessions struct {
	primary *session
	extras  []session
	paths   []string
	blocked bool
	problem string
}

func (a *App) applySessions(loaded loadedSessions) {
	a.sessionLoading = false
	a.sessionBlocked, a.persistenceError = loaded.blocked, loaded.problem
	if loaded.primary == nil {
		return
	}
	primary := loaded.primary
	a.state.Tabs, a.state.Active, a.state.Counter = primary.Tabs, primary.Active, primary.Counter
	a.mailbox = primary.Mailbox
	all := append([]session{*primary}, loaded.extras...)
	for i, s := range all {
		for id, c := range s.Chats {
			if c == nil || id != c.ID {
				continue
			}
			c.Status, c.TurnID = "idle", ""
			c.QueuePaused = true
			for j := range c.Outbox {
				c.Outbox[j].Status = "unconfirmed"
			}
			for j := range c.Queue {
				if c.Queue[j].Status == "pending" {
					c.Queue[j].Status = "unconfirmed"
				}
			}
			a.state.Chats[id] = c
			a.invalidateSidebar(id)
			a.rememberDraft(id)
		}
		if i > 0 {
			for _, t := range s.Tabs {
				found := false
				for _, prior := range a.state.Tabs {
					found = found || prior.ID == t.ID
				}
				if !found {
					a.state.Tabs = append(a.state.Tabs, t)
				}
			}
		}
		for _, doc := range s.Documents {
			a.installTransfer(doc)
		}
	}
	a.importedSessions = append(a.importedSessions, loaded.paths...)
	a.state.Active = primary.Active
}
func (a *App) loadSession() { a.applySessions(readSessions(a.store.Dir)) }
func (a *App) sessionSnapshot() session {
	a.checkpointEpoch++
	s := session{SchemaVersion: sessionSchemaVersion, Mailbox: map[string][]mailMessage{}, Active: a.state.Active, Counter: a.state.Counter, Chats: map[string]*workspace.Conversation{}}
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
		s.Chats[id] = a.checkpointConversation(c)
	}
	s.Tabs = nil
	for _, t := range a.state.Tabs {
		if c := a.state.Chats[t.Target]; c != nil && c.Ephemeral {
			continue
		}
		if t.Kind == workspace.File {
			if f := a.files[t.ID]; f != nil && f.Virtual {
				continue
			}
		}
		s.Tabs = append(s.Tabs, t)
		if t.Kind == workspace.Rally || t.Kind == workspace.File {
			s.Documents = append(s.Documents, a.checkpointDocument(t))
		}
	}
	for id := range a.chatCheckpoints {
		if s.Chats[id] == nil {
			delete(a.chatCheckpoints, id)
		}
	}
	for id := range a.documentCheckpoints {
		if a.documentCheckpoints[id].observed != a.checkpointEpoch {
			delete(a.documentCheckpoints, id)
		}
	}
	return s
}
func (a *App) saveSession() error {
	snapshot := a.sessionSnapshot()
	var err error
	if a.sessionBlocked {
		err = fmt.Errorf("original session is unreadable")
	} else {
		err = writeSessionFile(a.store, a.sessionFile(), snapshot)
	}
	if err == nil {
		a.removeImportedSessions()
		return nil
	}
	// A failed final flush must leave a durable, discoverable recovery copy.
	data, encodeErr := json.MarshalIndent(snapshot, "", "  ")
	if encodeErr == nil {
		if f, createErr := os.CreateTemp("", "fastrock-session-recovery-*.json"); createErr == nil {
			_ = f.Chmod(0600)
			_, writeErr := f.Write(data)
			syncErr := f.Sync()
			closeErr := f.Close()
			if writeErr == nil && syncErr == nil && closeErr == nil {
				return fmt.Errorf("could not save the session: %w. Drafts were preserved at %s", err, f.Name())
			}
			_ = os.Remove(f.Name())
		}
	}
	return fmt.Errorf("could not save the session or a recovery copy: %w", err)
}
func (a *App) checkpoint() {
	if a.sessionBlocked || a.sessionLoading || a.sessions == nil {
		return
	}
	if elapsed := time.Since(a.lastCheckpoint); elapsed < 100*time.Millisecond {
		if a.checkpointObserveTimer == nil {
			a.checkpointObserveTimer = time.AfterFunc(100*time.Millisecond-elapsed, func() {
				a.post(func() { a.checkpointObserveTimer = nil; a.lastCheckpoint = time.Time{} })
			})
		}
		return
	}
	a.lastCheckpoint = time.Now()
	a.publishThreads()
	snapshot := a.sessionSnapshot()
	if sameSessionCheckpoint(a.pendingCheckpoint, &snapshot) {
		return
	}
	a.pendingCheckpoint = &snapshot
	a.checkpointGeneration++
	generation := a.checkpointGeneration
	if a.checkpointTimer != nil {
		a.checkpointTimer.Stop()
	}
	if sameSessionCheckpoint(a.publishedCheckpoint, &snapshot) {
		return
	}
	a.checkpointTimer = time.AfterFunc(300*time.Millisecond, func() {
		a.post(func() {
			if generation == a.checkpointGeneration {
				a.publishCheckpoint()
			}
		})
	})
}

func (a *App) publishCheckpoint() {
	if a.pendingCheckpoint == nil || a.sessions == nil {
		return
	}
	snapshot := *a.pendingCheckpoint
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
			return
		}
	}
	a.publishedCheckpoint = a.pendingCheckpoint
}

func (a *App) publishThreads() {
	rows := []codex.OpenThread{}
	for _, tab := range a.state.Tabs {
		if tab.Kind == workspace.Chat {
			if c := a.state.Chats[tab.Target]; c != nil {
				rows = append(rows, codex.OpenThread{ID: c.ID, Title: c.Title, Cwd: c.Cwd, Status: c.Status, AcceptsMessages: !c.NoMessages && !c.EphemeralLost, Permissions: deliveryScope(c)})
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
	a.work(func() {
		c, err := codex.Dial(a.ctx, a.connection.Address, a.connection.Token)
		a.post(func() {
			a.connecting = false
			if err != nil {
				a.report(err)
				a.serverError = err.Error()
				a.status = "Codex disconnected"
				return
			}
			a.client = c
			a.serverError, a.startedProvider = "", ""
			a.serverGeneration++
			a.catalog.PolicyLoaded = false
			a.serverPaused, a.serverStarting = false, false
			a.resetInfoConnection()
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
			a.refreshCatalog()
			a.requestThreads(false, "")
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
		// Remove the backup first so a consumed window cannot reappear on
		// the next startup solely from its last-good snapshot.
		if err := os.Remove(path + ".bak"); err != nil && !os.IsNotExist(err) {
			continue
		}
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
	if c == nil || (c.Draft == "" && len(c.DraftAttachments) == 0 && len(c.Queue) == 0 && len(c.Outbox) == 0 && c.EditQueue == "") {
		delete(a.draftChats, id)
		return
	}
	if a.draftChats == nil {
		a.draftChats = make(map[string]bool)
	}
	a.draftChats[id] = true
}

func readSessionFile(path string) ([]byte, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	data, err := io.ReadAll(io.LimitReader(f, (32<<20)+1))
	if err == nil && len(data) > 32<<20 {
		return nil, fmt.Errorf("session exceeds 32 MiB limit")
	}
	return data, err
}
