package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"strconv"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/label"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type conversationInfo struct {
	Goal                                 map[string]any
	Terminals                            []map[string]any
	Loaded                               bool
	CrossTab                             bool
	TerminalsCollapsed, TerminalsLoading bool
	TerminalNote                         string
	TerminalUpdated                      time.Time
	Stopping                             map[string]bool
}

func (a *App) extraInfo(w *nucular.Window, c *workspace.Conversation) {
	if a.infoViews == nil {
		a.infoViews = map[string]*conversationInfo{}
	}
	v := a.infoViews[c.ID]
	if v == nil {
		v = &conversationInfo{}
		a.infoViews[c.ID] = v
	}
	if !v.Loaded {
		v.Loaded = true
		a.loadInfo(c, v)
	}
	w.Row(28).Dynamic(2)
	if w.ButtonText("Open folder") {
		a.openPath(c.Cwd, false)
	}
	if w.ButtonText("Changes") {
		a.showDiff(c)
	}
	title(w, "Goal", a.p)
	if v.Goal != nil {
		w.Row(65).Dynamic(1)
		w.LabelWrap(str(v.Goal, "objective"))
		muted(w, str(v.Goal, "status"), a.p)
		muted(w, fmt.Sprintf("%v tokens used", v.Goal["tokensUsed"]), a.p)
	}
	w.Row(27).Dynamic(3)
	if w.ButtonText("Set…") {
		a.inputDialog("Goal objective", "", func(objective string) {
			a.inputDialog("Token budget (empty = no limit)", "", func(budget string) {
				p := map[string]any{"threadId": c.ID, "origin": "user", "objective": objective, "status": "active"}
				if budget != "" {
					n, e := strconv.ParseInt(budget, 10, 64)
					if e != nil || n <= 0 {
						a.toast = "Enter a positive token budget"
						return
					}
					p["tokenBudget"] = n
				}
				a.rpc("thread/goal/set", p, func(raw json.RawMessage) { a.loadInfo(c, v) })
			})
		})
	}
	if w.ButtonText("Pause") {
		a.goalStatus(c, v, "paused")
	}
	if w.ButtonText("Resume") {
		a.goalStatus(c, v, "active")
	}
	w.Row(27).Dynamic(2)
	if w.ButtonText("Clear goal") {
		a.rpc("thread/goal/clear", map[string]any{"threadId": c.ID, "origin": "user"}, func(raw json.RawMessage) { a.loadInfo(c, v) })
	}
	if w.ButtonText("Refresh") {
		a.loadInfo(c, v)
	}
	a.drawTerminals(w, c, v)
	title(w, "Agents", a.p)
	a.drawAgents(w, c, 0, map[string]bool{})
	title(w, "Other conversations", a.p)
	w.Row(28).Dynamic(1)
	w.CheckboxText("Disable agent messages", &c.NoMessages)
	w.Row(28).Dynamic(1)
	if w.ButtonText("Read mailbox") {
		b, _ := json.MarshalIndent(a.mailbox[c.ID], "", "  ")
		a.openText("Messages · "+c.Title, string(b))
	}
	for _, peer := range a.state.Sidebar("", false) {
		if peer.ID == c.ID {
			continue
		}
		w.Row(27).Dynamic(1)
		if w.ButtonText(cut(peer.Title, 38)) {
			a.resumeThread(peer.ID)
		}
	}
}
func (a *App) loadInfo(c *workspace.Conversation, v *conversationInfo) {
	a.rpc("thread/goal/get", map[string]any{"threadId": c.ID}, func(raw json.RawMessage) { p := codex.Decode(raw); v.Goal, _ = p["goal"].(map[string]any) })
	a.loadTerminals(c, v)
}

func (a *App) loadTerminals(c *workspace.Conversation, v *conversationInfo) {
	if v.TerminalsLoading || a.client == nil {
		return
	}
	v.TerminalsLoading = true
	v.TerminalUpdated = time.Now()
	client, id := a.client, c.ID
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		var rows []map[string]any
		cursor := ""
		var err error
		for {
			var page struct {
				Data []map[string]any
				Next string `json:"nextCursor"`
			}
			params := map[string]any{"threadId": id, "limit": 100}
			if cursor != "" {
				params["cursor"] = cursor
			}
			err = client.Call(ctx, "thread/backgroundTerminals/list", params, &page)
			if err != nil {
				break
			}
			rows = append(rows, page.Data...)
			if page.Next == "" || page.Next == cursor || len(rows) >= 1000 {
				break
			}
			cursor = page.Next
		}
		a.post(func() {
			v.TerminalsLoading = false
			v.TerminalNote = ""
			if err != nil {
				v.TerminalNote = err.Error()
				return
			}
			v.Terminals = rows
			for id := range v.Stopping {
				found := false
				for _, row := range rows {
					found = found || str(row, "processId") == id
				}
				if !found {
					delete(v.Stopping, id)
				}
			}
		})
	})
}
func (a *App) stopTerminal(c *workspace.Conversation, v *conversationInfo, id string) {
	if v.Stopping == nil {
		v.Stopping = map[string]bool{}
	}
	v.Stopping[id] = true
	client := a.client
	if client == nil {
		delete(v.Stopping, id)
		return
	}
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		err := client.Call(ctx, "thread/backgroundTerminals/terminate", map[string]any{"threadId": c.ID, "processId": id}, nil)
		a.post(func() { delete(v.Stopping, id); a.report(err); a.loadTerminals(c, v) })
	})
}
func (a *App) drawTerminals(w *nucular.Window, c *workspace.Conversation, v *conversationInfo) {
	if len(v.Terminals) > 0 && time.Since(v.TerminalUpdated) > 5*time.Second {
		a.loadTerminals(c, v)
	}
	w.Row(28).Dynamic(1)
	arrow := "▾ "
	if v.TerminalsCollapsed {
		arrow = "▸ "
	}
	if w.ButtonText(fmt.Sprintf("%sBackground terminals · %d running", arrow, len(v.Terminals))) {
		v.TerminalsCollapsed = !v.TerminalsCollapsed
	}
	if v.TerminalsCollapsed {
		return
	}
	if v.TerminalNote != "" {
		muted(w, v.TerminalNote, a.p)
	}
	for _, terminal := range v.Terminals {
		id, command := str(terminal, "processId"), str(terminal, "command")
		w.Row(28).Ratio(.76, .24)
		w.LabelColored("● "+cut(strings.SplitN(command, "\n", 2)[0], 60), "LC", a.p.Text)
		if menu := w.ContextualOpen(0, image.Pt(210, 110), w.LastWidgetBounds, nil); menu != nil {
			if menu.MenuItem(label.T("Copy command")) {
				a.copyText(command)
			}
			if menu.MenuItem(label.T("View details")) {
				b, _ := json.MarshalIndent(terminal, "", "  ")
				a.openText("Terminal "+id, string(b))
			}
			if menu.MenuItem(label.T("Open working folder")) {
				a.openPath(str(terminal, "cwd"), false)
			}
		}
		if v.Stopping[id] {
			w.Label("Stopping…", "LC")
		} else if w.ButtonText("Stop") {
			a.stopTerminal(c, v, id)
		}
		if cwd := str(terminal, "cwd"); cwd != "" && cwd != c.Cwd {
			muted(w, cwd, a.p)
		}
	}
	if len(v.Terminals) == 0 && v.TerminalNote == "" {
		muted(w, "No commands are running in the background.", a.p)
	}
	w.Row(27).Dynamic(2)
	if w.ButtonText("Refresh") {
		a.loadTerminals(c, v)
	}
	if len(v.Terminals) > 1 && w.ButtonText("Stop all…") {
		ids := make([]string, 0, len(v.Terminals))
		for _, t := range v.Terminals {
			ids = append(ids, str(t, "processId"))
		}
		a.confirm("Stop background commands?", fmt.Sprintf("Stop %d running commands in this conversation?", len(ids)), func() {
			for _, id := range ids {
				a.stopTerminal(c, v, id)
			}
		})
	}
}
func (a *App) goalStatus(c *workspace.Conversation, v *conversationInfo, status string) {
	a.rpc("thread/goal/set", map[string]any{"threadId": c.ID, "origin": "user", "status": status}, func(_ json.RawMessage) { a.loadInfo(c, v) })
}

func (a *App) drawAgents(w *nucular.Window, c *workspace.Conversation, depth int, seen map[string]bool) {
	if depth >= 4 || seen[c.ID] {
		return
	}
	seen[c.ID] = true
	for _, agent := range c.Agents {
		w.Row(27).Dynamic(1)
		name := agent.Name
		if name == "" {
			name = cut(agent.ID, 20)
		}
		if w.ButtonText(strings.Repeat("  ", depth) + name + " · " + agent.Status) {
			if a.state.Chats[agent.ID] == nil {
				a.state.Chats[agent.ID] = &workspace.Conversation{ID: agent.ID, Title: name, Cwd: c.Cwd}
			}
			a.resumeThread(agent.ID)
		}
		if child := a.state.Chats[agent.ID]; child != nil {
			a.drawAgents(w, child, depth+1, seen)
		}
	}
}
