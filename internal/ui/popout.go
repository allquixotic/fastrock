package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/settings"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type Connection struct {
	Client                 *codex.Client
	Address, Token, Ticket string
}
type editorPosition struct{ Cursor, Start, End int }

func position(e *nucular.TextEditor) editorPosition {
	if e == nil {
		return editorPosition{}
	}
	return editorPosition{e.Cursor, e.SelectStart, e.SelectEnd}
}
func (p editorPosition) apply(e *nucular.TextEditor) {
	if e != nil {
		e.Cursor = min(p.Cursor, len(e.Buffer))
		e.SelectStart = min(p.Start, len(e.Buffer))
		e.SelectEnd = min(p.End, len(e.Buffer))
	}
}

type fileTransfer struct {
	Diff                bool
	BasePath            string
	Text, Find          string
	Virtual, Wrap, More bool
	TextLoaded          bool
	Offset              int64
	Position            editorPosition
	ScrollX, ScrollY    int
}
type chatTransfer struct {
	Position           editorPosition
	Follow             bool
	Scroll, ShowBlocks int
}
type tabTransfer struct {
	Mailbox  []mailMessage
	File     *fileTransfer
	ChatView *chatTransfer
	Text     string
	Tab      workspace.Tab
	Chat     *workspace.Conversation
	Rally    *rallyTransfer
}
type rallyTransfer struct {
	Mode, Group, Timebox, Query, Search, Owner, State string
	OnlyBlocked, OnlyReady                            bool
	Columns                                           []string
	LaneScroll                                        map[string]int
	ListPage                                          int
	Detail                                            *detailTransfer
}
type detailTransfer struct {
	Rich               map[string]richtext.Saved
	CommentDocument    richtext.Saved
	Original           rally.Object
	Kind, Tab, Comment string
	New                bool
	Values             map[string]string
}

func (a *App) tabSnapshot(t workspace.Tab) tabTransfer {
	p := tabTransfer{Tab: t}
	if v := a.files[t.ID]; v != nil && v.Editor != nil {
		p.File = &fileTransfer{Diff: len(v.Diff) > 0, BasePath: v.BasePath, TextLoaded: true, Text: text(v.Editor), Find: text(v.Find), Virtual: v.Virtual, Wrap: v.Wrap, More: v.More, Offset: v.Offset, Position: position(v.Editor), ScrollX: v.Editor.Scrollbar.X, ScrollY: v.Editor.Scrollbar.Y}
	}
	if c := a.state.Chats[t.Target]; c != nil {
		copy := *c
		copy.Blocks = append([]workspace.Block(nil), c.Blocks...)
		copy.Queue = cloneQueue(c.Queue)
		copy.DraftAttachments = append([]string(nil), c.DraftAttachments...)
		if v := a.chats[c.ID]; v != nil {
			copy.Draft = text(v.Editor)
			p.ChatView = &chatTransfer{Position: position(v.Editor), Follow: v.Follow, Scroll: v.Scroll, ShowBlocks: v.ShowBlocks}
			copy.DraftAttachments = append([]string(nil), v.Attachments...)
		}
		p.Chat = &copy
		p.Mailbox = append([]mailMessage(nil), a.mailbox[c.ID]...)
	}
	if v := a.rallyViews[t.ID]; v != nil {
		r := &rallyTransfer{Mode: v.Mode, Group: v.Group, Timebox: v.Timebox, Query: text(v.Query), Search: text(v.Search), Owner: v.OwnerFilter, State: v.StateFilter, OnlyBlocked: v.OnlyBlocked, OnlyReady: v.OnlyReady, Columns: append([]string(nil), v.Columns...)}
		r.ListPage = v.Page
		r.LaneScroll = make(map[string]int, len(v.LaneScroll))
		for k, n := range v.LaneScroll {
			r.LaneScroll[k] = n
		}
		if d := v.Detail; d != nil {
			data := &detailTransfer{Original: d.Original.Clone(), Kind: d.Kind, Tab: d.Tab, New: d.New, Values: map[string]string{}, Rich: map[string]richtext.Saved{}}
			for k, e := range d.Editors {
				data.Values[k] = text(e)
			}
			for k, r := range d.Rich {
				r.sync()
				data.Rich[k] = r.doc.Save()
			}
			if d.CommentRich != nil {
				d.CommentRich.sync()
				data.CommentDocument = d.CommentRich.doc.Save()
			}
			r.Detail = data
		}
		p.Rally = r
	}
	return p
}
func (a *App) installTransfer(p tabTransfer) {
	t := p.Tab
	id := t.ID
	if a.transferPending && a.incomingTicket != "" {
		id = "window-" + a.incomingTicket
		a.incomingTab = id
	}
	if id == "" {
		id = a.state.Open(t.Kind, t.Title, t.Target, t.Page)
	} else {
		found := false
		for _, existing := range a.state.Tabs {
			if existing.ID == id {
				found = true
				break
			}
		}
		if !found {
			t.ID = id
			a.state.Tabs = append(a.state.Tabs, t)
		}
		a.state.Active = id
	}
	if c := p.Chat; c != nil {
		if a.mailbox == nil {
			a.mailbox = map[string][]mailMessage{}
		}
		a.mailbox[c.ID] = p.Mailbox
		delete(a.detached, c.ID)
		a.state.Chats[c.ID] = c
		v := newChatView()
		setText(v.Editor, c.Draft)
		v.Attachments = c.DraftAttachments
		if view := p.ChatView; view != nil {
			view.Position.apply(v.Editor)
			v.Follow = view.Follow
			v.Scroll = view.Scroll
			v.RestoreScroll = true
			v.ShowBlocks = max(200, view.ShowBlocks)
		}
		a.chats[c.ID] = v

	}
	if r := p.Rally; r != nil {
		if old := a.rallyViews[id]; old != nil {
			old.Closed = true
			if old.cancel != nil {
				old.cancel()
			}
		}
		v := newRallyView(rally.FindPage(t.Page))
		v.Mode, v.Group, v.Timebox = r.Mode, r.Group, r.Timebox
		setText(v.Query, r.Query)
		setText(v.Search, r.Search)
		v.OwnerFilter, v.StateFilter = r.Owner, r.State
		v.OnlyBlocked, v.OnlyReady = r.OnlyBlocked, r.OnlyReady
		v.Columns = r.Columns
		v.RestoreLaneScroll = r.LaneScroll
		v.Page = max(1, r.ListPage)
		if d := r.Detail; d != nil {
			v.Detail = makeDetail(d.Original, d.Kind, d.New)
			v.Detail.Tab = d.Tab
			for k, value := range d.Values {
				if e := v.Detail.Editors[k]; e != nil {
					setText(e, value)
				}
				if v.Detail.Rich[k] != nil {
					v.Detail.Rich[k] = newRichEditor(value)
				}
			}
			for k, saved := range d.Rich {
				r := newRichEditor("")
				r.doc = richtext.Restore(saved)
				setText(r.editor, string(r.doc.Text))
				r.cache()
				v.Detail.Rich[k] = r
			}
			r := newRichEditor("")
			r.doc = richtext.Restore(d.CommentDocument)
			setText(r.editor, string(r.doc.Text))
			r.cache()
			v.Detail.CommentRich = r
		}
		a.rallyViews[id] = v
		if a.rallyClient != nil {
			a.refreshRally(v)
		}
	}
	if t.Kind == workspace.File {
		if f := p.File; f != nil {
			e := textEditor(f.Text, true)
			e.Flags |= nucular.EditReadOnly
			f.Position.apply(e)
			e.Scrollbar.X = f.ScrollX
			e.Scrollbar.Y = f.ScrollY
			v := &fileView{Path: t.Target, Editor: e, Find: textEditor(f.Find, false), Virtual: f.Virtual, Wrap: f.Wrap, More: f.More, Offset: f.Offset}
			a.files[id] = v
			v.BasePath = f.BasePath
			if f.Diff {
				a.prepareDiff(v, f.Text)
			}
			if !f.Virtual && !f.TextLoaded {
				v.Editor = nil
				v.PendingPosition = &f.Position
				a.loadFilePage(v, false)
			}
		} else if p.Text != "" {
			a.openText(t.Title, p.Text)
		} else {
			a.openFile(t.Target)
		}
	}
	if t.Kind == workspace.Settings {
		a.openSettings()
	}
}
func (a *App) popOut(t workspace.Tab) {
	if a.connection.Address == "" || a.client == nil {
		a.toast = "Codex is still connecting"
		return
	}
	if a.popping[t.ID] {
		return
	}
	a.popping[t.ID] = true
	payload := a.tabSnapshot(t)
	client := a.client
	connection := a.connection
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 20*time.Second)
		defer cancel()
		var offer struct{ Ticket string }
		err := client.Call(ctx, "fastrock/offer", map[string]any{"data": payload}, &offer)
		if err == nil {
			a.post(func() { a.transfers[offer.Ticket] = t.ID })
			var binary string
			binary, err = os.Executable()
			if err == nil {
				cmd := exec.Command(binary, "--popout", offer.Ticket)
				cmd.Env = append(windowEnvironment(), "FASTROCK_BROKER="+connection.Address, "FASTROCK_BROKER_TOKEN="+connection.Token)
				err = cmd.Start()
				if err == nil {
					go func() { _ = cmd.Wait() }()
				}
			}
		}
		if err != nil {
			_ = client.Notify("fastrock/cancel", map[string]string{"ticket": offer.Ticket})
			a.post(func() { delete(a.popping, t.ID); a.report(fmt.Errorf("open window: %w", err)) })
			return
		}
		a.watchTransfer(t.ID, offer.Ticket)
	})
}

// Cancellation and timeout always retain the source document.
func (a *App) abortTransfer(id, ticket string, err error) {
	delete(a.popping, id)
	delete(a.transfers, ticket)
	if ticket != "" {
		a.rpc("fastrock/cancel", map[string]string{"ticket": ticket}, nil)
	}
	a.report(err)
}
func (a *App) watchTransfer(id, ticket string) {
	timer := time.NewTimer(30 * time.Second)
	defer timer.Stop()
	select {
	case <-timer.C:
		a.post(func() {
			if a.transfers[ticket] == id {
				a.abortTransfer(id, ticket, fmt.Errorf("destination window did not accept the tab; the source is still here"))
			}
		})
	case <-a.ctx.Done():
	}

}
func (a *App) transferEvent(m codex.Message) bool {
	if !strings.HasPrefix(m.Method, "fastrock/") {
		return false
	}
	switch m.Method {
	case "fastrock/delivery":
		var message mailMessage
		if json.Unmarshal(m.Params, &message) == nil && message.To != "" {
			if a.mailbox == nil {
				a.mailbox = map[string][]mailMessage{}
			}
			messages := append(a.mailbox[message.To], message)
			a.mailbox[message.To] = messages[max(0, len(messages)-50):]
		}
	case "fastrock/serverStopped":
		var p struct{ Message string }
		_ = json.Unmarshal(m.Params, &p)
		a.status, a.toast = p.Message, p.Message
		a.serverPaused = true
		for id, v := range a.chats {
			if c := a.state.Chats[id]; c != nil {
				c.Draft = text(v.Editor)
				c.DraftAttachments = append([]string(nil), v.Attachments...)
				if c.Ephemeral {
					c.EphemeralLost = true
					continue
				}
			}
			delete(a.chats, id)
		}
		a.approvals = nil
		for _, c := range a.state.Chats {
			c.Status, c.TurnID = "idle", ""
			if c.Ephemeral {
				c.EphemeralLost = true
				c.Status = "ended"
			}
		}
		if a.assistant != nil {
			a.assistant.Busy = false
			a.assistant.ThreadID = ""
			a.assistant.TurnID = ""
		}
	case "fastrock/serverReady":
		var p struct{ Version string }
		_ = json.Unmarshal(m.Params, &p)
		a.status = p.Version + " · Connected"
		a.serverPaused = false
		a.toast = "Codex restarted; unsent drafts retained"
		client, cwd := a.client, a.prefs.WorkingDirectory
		a.work(func() { a.loadCatalog(client, cwd); a.loadThreads(client, false) })
		if t := a.state.Current(); t != nil && t.Kind == workspace.Chat {
			a.resumeThread(t.Target)
		}
	case "fastrock/tabRefresh":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		if id := a.transfers[p.Ticket]; id != "" {
			for _, t := range a.state.Tabs {
				if t.ID == id {
					payload := a.tabSnapshot(t)
					a.rpc("fastrock/finalize", map[string]any{"ticket": p.Ticket, "data": payload}, nil)
					break
				}
			}
		}
	case "fastrock/tabCancelled":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		if id := a.transfers[p.Ticket]; id != "" {
			delete(a.transfers, p.Ticket)
			delete(a.popping, id)
			a.toast = "The destination window closed; the tab is still here"
		}
		if a.incomingTicket == p.Ticket {
			a.cancelIncoming()
		}
	case "fastrock/finalized":
		var p struct {
			Ticket string
			Data   tabTransfer
		}
		if json.Unmarshal(m.Params, &p) == nil && p.Ticket == a.incomingTicket {
			a.installTransfer(p.Data)
			a.rpcResult("fastrock/applied", map[string]string{"ticket": p.Ticket}, func(_ json.RawMessage) {
				a.transferPending = false
				a.incomingTicket, a.incomingTab = "", ""
				buffer := a.transferBuffer
				a.transferBuffer = nil
				for _, messages := range buffer {
					for _, message := range messages {
						a.event(message)
					}
				}
			}, func(error) { a.cancelIncoming() })
		}

	case "fastrock/preferences":
		var p settings.Preferences
		if json.Unmarshal(m.Params, &p) == nil {
			a.prefs = p
			a.p = colors(p.Theme == "light")
			a.window.SetStyle(makeStyle(a.p, p.FontSize))
		}
	case "fastrock/threadMoved":
		var p struct{ ThreadID string }
		_ = json.Unmarshal(m.Params, &p)
		if a.detached == nil {
			a.detached = map[string]bool{}
		}
		a.detached[p.ThreadID] = true
		for _, t := range append([]workspace.Tab(nil), a.state.Tabs...) {
			if t.Kind == workspace.Chat && t.Target == p.ThreadID {
				a.closeTabNow(t.ID)
			}
		}
		if c := a.state.Chats[p.ThreadID]; c != nil {
			c.Status = "idle"
			c.TurnID = ""
			c.Queue = nil
			c.Draft = ""
			c.DraftAttachments = nil
			c.ReleaseTranscript()
		}
		delete(a.chats, p.ThreadID)
		for i := len(a.approvals) - 1; i >= 0; i-- {
			params := codex.Decode(a.approvals[i].Message.Params)
			if str(params, "threadId") == p.ThreadID {
				a.approvals = append(a.approvals[:i], a.approvals[i+1:]...)
			}
		}
	case "fastrock/tabAvailable":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		a.claimTab(p.Ticket)
	case "fastrock/tabClaimed":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		if id := a.transfers[p.Ticket]; id != "" {
			delete(a.transfers, p.Ticket)
			delete(a.popping, id)
			a.closeTabNow(id)
		}
	}
	return true
}
func (a *App) claimTab(ticket string) {
	if a.transferPending || a.readyTicket != "" {
		a.toast = "Finish the current window transfer before moving another tab"
		return
	}
	a.transferPending = true
	a.incomingTicket = ticket
	a.work(func() {
		var payload tabTransfer
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		err := a.client.Call(ctx, "fastrock/claim", map[string]string{"ticket": ticket}, &payload)
		a.post(func() {
			if err != nil {
				a.transferPending = false
				a.incomingTicket = ""
				a.report(err)
			} else {
				a.installTransfer(payload)
				a.transferPending = true
				if payload.Chat != nil {
					a.transferBuffer = map[string][]codex.Message{payload.Chat.ID: nil}
				}
				a.readyTicket = ticket
			}
		})
	})
}
func (a *App) moveTab(t workspace.Tab) {
	a.rpc("fastrock/windows", map[string]any{}, func(raw json.RawMessage) {
		var list struct{ Windows []struct{ ID, Name string } }
		_ = json.Unmarshal(raw, &list)
		a.window.PopupOpen("Move tab to window", nucular.WindowTitle|nucular.WindowClosable, dialogBounds(), true, func(w *nucular.Window) {
			if len(list.Windows) == 0 {
				muted(w, "No other Fastrock windows are open.", a.p)
			}
			for _, other := range list.Windows {
				w.Row(30).Dynamic(1)
				if w.ButtonText(fallback(other.Name, other.ID[:8])) {
					a.popping[t.ID] = true
					payload := a.tabSnapshot(t)
					destination := other.ID
					a.rpcResult("fastrock/offer", map[string]any{"data": payload}, func(raw json.RawMessage) {
						var offer struct{ Ticket string }
						_ = json.Unmarshal(raw, &offer)
						a.transfers[offer.Ticket] = t.ID
						a.rpcResult("fastrock/move", map[string]string{"ticket": offer.Ticket, "window": destination}, func(json.RawMessage) {
							a.work(func() { a.watchTransfer(t.ID, offer.Ticket) })
						}, func(err error) { a.abortTransfer(t.ID, offer.Ticket, err) })
					}, func(err error) { a.abortTransfer(t.ID, "", err) })
					w.Close()
				}
			}
		})
	})
}
func windowEnvironment() []string {
	env := os.Environ()
	out := make([]string, 0, len(env))
	for _, item := range env {
		if !strings.HasPrefix(item, "FASTROCK_AUTOMATION=") && !strings.HasPrefix(item, "FASTROCK_BROKER=") && !strings.HasPrefix(item, "FASTROCK_BROKER_TOKEN=") {
			out = append(out, item)
		}
	}
	return out
}

func (a *App) cancelIncoming() {
	if a.incomingTab != "" {
		a.closeTabNow(a.incomingTab)
	}
	a.transferPending = false
	a.incomingTicket, a.incomingTab, a.readyTicket = "", "", ""
	a.transferBuffer = nil
	a.toast = "Window transfer was cancelled; the source tab is intact"
}
