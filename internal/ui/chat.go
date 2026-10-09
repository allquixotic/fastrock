package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image/color"
	"path/filepath"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type chatView struct {
	RichSelection transcriptSelection
	SelectID      string
	Selection     *nucular.TextEditor
	Suggest       []string
	SuggestQuery  string
	SuggestIndex  int
	SuggestCancel context.CancelFunc
	Layouts       map[string]*transcriptLayout
	ShowBlocks    int
	Scroll        int
	RestoreScroll bool

	Editor         *nucular.TextEditor
	Attachments    []string
	EditQueue      string
	QueuePage      int
	Follow         bool
	PreviousScroll int
	Expanded       map[string]bool
}

func newChatView() *chatView {
	return &chatView{ShowBlocks: 200, Editor: textEditor("", true), Follow: true, Expanded: map[string]bool{}}
}
func (a *App) loadThreads(client *codex.Client, archived bool) {
	a.fetchThreads(client, archived, "")
}
func (a *App) fetchThreads(client *codex.Client, archived bool, cursor string) {
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	defer cancel()
	var r struct {
		Data []map[string]any `json:"data"`
		Next string           `json:"nextCursor"`
	}
	params := map[string]any{"limit": 100, "archived": archived, "sortKey": "updated_at", "sortDirection": "desc"}
	if cursor != "" {
		params["cursor"] = cursor
	}
	e := client.Call(ctx, "thread/list", params, &r)
	a.post(func() {
		if e != nil {
			a.report(e)
			return
		}
		if a.historyCursor == nil {
			a.historyCursor = map[bool]string{}
		}
		a.historyCursor[archived] = r.Next
		for _, t := range r.Data {
			id := str(t, "id")
			if id == "" {
				continue
			}
			title, cwd, updated := threadTitle(t), str(t, "cwd"), integer(t, "updatedAt")
			if c := a.state.Chats[id]; c != nil {
				if c.Title != title || c.Cwd != cwd || c.Updated != updated || c.Archived != archived {
					c.Title, c.Cwd, c.Updated, c.Archived = title, cwd, updated, archived
					a.invalidateSidebar()
				}
			} else {
				a.state.Chats[id] = &workspace.Conversation{ID: id, Title: title, Cwd: cwd, Updated: updated, Archived: archived, Status: "idle"}
			}
		}
	})
}
func str(m map[string]any, k string) string    { s, _ := m[k].(string); return s }
func integer(m map[string]any, k string) int64 { n, _ := m[k].(float64); return int64(n) }
func threadTitle(t map[string]any) string {
	for _, k := range []string{"name", "preview", "id"} {
		if s := str(t, k); s != "" {
			return cut(s, 65)
		}
	}
	return "New conversation"
}
func cut(s string, n int) string {
	if len(s) <= n {
		return s
	}
	count := 0
	for i := range s {
		if count == n {
			return s[:i] + "…"
		}
		count++
	}
	return s
}

func (a *App) newThread(cwd string) {
	if cwd != "" && !filepath.IsAbs(cwd) {
		a.toast = "Choose an absolute project folder"
		return
	}
	params := map[string]any{"experimentalRawEvents": false}
	if a.prefs.AgentMessages {
		params["dynamicTools"] = crossTabSpecs()
	}
	if cwd != "" {
		params["cwd"] = cwd
	}
	a.rpc("thread/start", params, func(raw json.RawMessage) {
		r := codex.Decode(raw)
		t, _ := r["thread"].(map[string]any)
		id := str(t, "id")
		if id == "" {
			a.toast = "Codex returned no thread ID"
			return
		}
		c := &workspace.Conversation{ID: id, Title: "New conversation", Cwd: str(t, "cwd"), Model: str(r, "model"), Tier: str(r, "serviceTier"), Effort: str(r, "reasoningEffort"), Status: "idle", Updated: time.Now().Unix()}
		c.NoMessages = !a.prefs.AgentMessages
		if c.Cwd == "" {
			c.Cwd = cwd
		}
		if c.Model == "" {
			c.Model = a.catalog.DefaultModel()
		}
		if c.Effort == "" {
			if m, ok := a.catalog.Find(c.Model); ok {
				c.Effort = m.DefaultEffort
			}
		}
		a.state.Chats[id] = c
		a.chats[id] = newChatView()
		delete(a.detached, id)
		a.state.Open(workspace.Chat, c.Title, id, "")
		if cwd != "" {
			a.prefs.WorkingDirectory = cwd
			a.rememberFolder(cwd)
		}
		a.savePrefs()
	})
}
func (a *App) resumeThread(id string) {
	wasDetached := a.detached[id]
	delete(a.detached, id)
	if wasDetached {
		delete(a.chats, id)
	}
	if a.client != nil {
		c := a.client
		a.work(func() { _ = c.Notify("fastrock/own", map[string]string{"threadId": id}) })
	}
	c := a.state.Chats[id]
	if c == nil {
		return
	}
	a.state.Open(workspace.Chat, c.Title, id, "")
	if a.chats[id] != nil {
		return
	}
	a.chats[id] = newChatView()
	setText(a.chats[id].Editor, c.Draft)
	a.chats[id].Attachments = c.DraftAttachments
	if a.serverPaused || c.EphemeralLost {
		return
	}
	c.Status = "starting"
	client := a.client
	view := a.chats[id]
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var raw json.RawMessage
		err := client.Call(ctx, "thread/resume", map[string]any{"threadId": id}, &raw)
		r := codex.Decode(raw)
		prepared := &workspace.Conversation{Status: "idle", Model: str(r, "model"), Effort: str(r, "reasoningEffort"), Tier: str(r, "serviceTier")}
		t, _ := r["thread"].(map[string]any)
		if turns, ok := t["turns"].([]any); ok {
			for _, turn := range turns {
				tr, _ := turn.(map[string]any)
				if items, ok := tr["items"].([]any); ok {
					for _, value := range items {
						if it, ok := value.(map[string]any); ok {
							a.upsertItem(prepared, it)
						}
					}
				}
				if str(tr, "status") == "inProgress" {
					prepared.TurnID = str(tr, "id")
					prepared.Status = "running"
				}
			}
		}
		a.post(func() {
			if a.chats[id] != view {
				return
			}
			if err != nil {
				c.Status = "error"
				delete(a.chats, id)
				a.report(err)
				return
			}
			c.Model, c.Effort, c.Tier = prepared.Model, prepared.Effort, prepared.Tier
			if c.Model == "" {
				c.Model = a.catalog.DefaultModel()
			}
			if c.Status == "starting" {
				c.Status, c.TurnID = prepared.Status, prepared.TurnID
			}
			c.Blocks = mergeTranscript(prepared.Blocks, c.Blocks)
			if len(c.Agents) == 0 {
				c.Agents = prepared.Agents
			}
			c.TrimTranscript()
		})
	})

}
func (a *App) drawSidebar(w *nucular.Window) {
	w.Row(28).Ratio(.72, .28)
	w.LabelColored("CONVERSATIONS", "LC", a.p.Muted)
	if iconButton(w, "plus", false, a.p) {
		a.newThread(a.prefs.WorkingDirectory)
	}
	w.Row(28).Dynamic(1)
	a.sidebarSearch.Edit(w)
	w.Row(26).Dynamic(2)
	if flatRow(w, "Recent", "", !a.archived, color.RGBA{}, a.p) {
		a.archived = false
	}
	if flatRow(w, "Archived", "", a.archived, color.RGBA{}, a.p) {
		a.archived = true
		if a.client != nil {
			client := a.client
			a.work(func() { a.loadThreads(client, true) })
		}
	}
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	rowHeight := int(28 * w.Master().Style().Scaling)
	stride := rowHeight + spacing
	for _, folder := range a.sidebarFolders() {
		w.RowScaled(rowHeight).Dynamic(1)
		if folderRow(w, folder.title, !a.collapsed[folder.path], a.p) {
			a.collapsed[folder.path] = !a.collapsed[folder.path]
		}
		a.folderContext(w, folder.path)
		if a.collapsed[folder.path] {
			continue
		}
		top := w.WidgetBounds().Y
		first, last := sidebarVisible(top, w.Bounds.Y, w.Bounds.Y+w.Bounds.H, stride, len(folder.rows))
		sidebarSkip(w, first, stride, spacing)
		for _, c := range folder.rows[first:last] {
			w.RowScaled(rowHeight).Dynamic(1)
			var dot color.RGBA
			if c.Busy() {
				dot = a.p.Accent
			} else if c.Status == "error" {
				dot = a.p.Danger
			}
			active := false
			if tab := a.state.Current(); tab != nil {
				active = tab.Target == c.ID
			}
			if flatRow(w, c.Title, "", active, dot, a.p) {
				a.resumeThread(c.ID)
			}
			a.sidebarContext(w, c)
		}
		sidebarSkip(w, len(folder.rows)-last, stride, spacing)
	}
	if a.sidebarCache.count == 0 {
		muted(w, "No conversations", a.p)
	}
	if cursor := a.historyCursor[a.archived]; cursor != "" && a.client != nil {
		w.Row(28).Dynamic(1)
		if w.ButtonText("Load older conversations") {
			client, archived := a.client, a.archived
			a.historyCursor[archived] = ""
			a.work(func() { a.fetchThreads(client, archived, cursor) })
		}
	}
	w.Row(26).Dynamic(1)
	if flatRow(w, "Refresh history", "", false, color.RGBA{}, a.p) {
		if a.client != nil {
			client := a.client
			archived := a.archived
			a.work(func() { a.loadThreads(client, archived) })
		}
	}
}
func (a *App) send(c *workspace.Conversation, mode string) {
	if mode == "send" && a.prefs.BusyInput == "steer" && c.Busy() {
		mode = "steer"
	}
	v := a.chats[c.ID]
	if a.serverPaused || c.EphemeralLost {
		a.toast = "This conversation is unavailable; your unsent draft is retained"
		return
	}
	if a.client == nil {
		a.toast = "Codex is disconnected. Reconnect in Settings."
		return
	}
	value := strings.TrimSpace(text(v.Editor))
	if value == "" {
		return
	}
	if strings.HasPrefix(value, "/") && a.slash(c, value) {
		setText(v.Editor, "")
		return
	}
	if v.EditQueue != "" {
		c.EditQueued(v.EditQueue, value)
		v.EditQueue = ""
		setText(v.Editor, "")
		return
	}
	if c.Busy() && mode != "steer" {
		c.Enqueue(value, v.Attachments)
		v.Attachments = nil
		setText(v.Editor, "")
		return
	}
	if warning := a.catalog.SpeedWarning(c.Model, fallback(c.Tier, "default")); warning != "" {
		a.toast = warning
		return
	}
	if c.Title == "New conversation" {
		a.invalidateSidebar()
		c.Title = cut(strings.SplitN(value, "\n", 2)[0], 65)
		for i := range a.state.Tabs {
			if a.state.Tabs[i].Target == c.ID {
				a.state.Tabs[i].Title = c.Title
			}
		}
	}
	a.startTurn(c, value, v.Attachments, mode)
	v.Attachments = nil
	setText(v.Editor, "")
}
func (a *App) startTurn(c *workspace.Conversation, value string, attachments []string, mode string) {
	if a.serverPaused || c.EphemeralLost {
		a.toast = "Codex is restarting; your draft is retained"
		return
	}
	tier := c.Tier
	if tier == "" {
		tier = "default"
	}
	if warning := a.catalog.SpeedWarning(c.Model, tier); warning != "" {
		a.toast = warning
		a.chats[c.ID].Editor.Buffer = []rune(value)
		return
	}
	input := []map[string]any{}
	for _, path := range attachments {
		input = append(input, map[string]any{"type": "localImage", "path": path})
	}
	input = append(input, codex.TextInput(value)...)
	params := map[string]any{"threadId": c.ID, "input": input, "clientUserMessageId": fmt.Sprintf("fastrock-%d", time.Now().UnixNano())}
	method := "turn/start"
	if c.Busy() && mode == "steer" {
		if c.TurnID == "" {
			c.Enqueue(value, attachments)
			return
		}
		method = "turn/steer"
		params["expectedTurnId"] = c.TurnID
	} else {
		params["model"] = c.Model
		params["serviceTier"] = tier
		if c.Effort != "" {
			params["effort"] = c.Effort
		}
		if c.Plan {
			params["collaborationMode"] = map[string]any{"mode": "plan", "settings": map[string]any{"model": c.Model, "reasoning_effort": c.Effort, "developer_instructions": nil}}
		}
		c.Status = "starting"
	}
	client := a.client
	if client == nil {
		a.toast = "Codex is disconnected"
		return
	}
	id, cwd := c.ID, c.Cwd
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var raw json.RawMessage
		var e error
		if strings.Contains(value, "$") {
			var skills skillList
			e = client.Call(ctx, "skills/list", map[string]any{"cwds": []string{cwd}}, &skills)
			if e == nil {
				params["input"] = append(input, skillInputs(value, skills)...)
			}
		}
		if e == nil {
			e = client.Call(ctx, method, params, &raw)
		}
		a.post(func() {
			chat := a.state.Chats[id]
			if chat == nil || a.detached[id] {
				return
			}
			if e != nil {
				chat.Status = "error"
				a.report(e)
				if v := a.chats[id]; v != nil && text(v.Editor) == "" {
					setText(v.Editor, value)
					v.Attachments = attachments
				}
				return
			}
			if method == "turn/start" {
				r := codex.Decode(raw)
				turn, _ := r["turn"].(map[string]any)
				if chat.Status == "starting" {
					chat.TurnID = str(turn, "id")
					chat.Status = "running"
				}
			}
			chat.Updated = time.Now().Unix()
			a.invalidateSidebar()
		})
	})
}
func (a *App) consume(client *codex.Client) {
	for m := range client.Events {
		message := m
		if len(m.ID) > 0 && m.Method == "item/tool/call" {
			a.runDynamicTool(client, m)
			continue
		}
		params := codex.Decode(message.Params)
		a.post(func() { a.eventDecoded(message, params) })
	}
	a.post(func() {
		if a.ctx.Err() == nil {
			if a.client != client {
				return
			}
			a.client = nil
			a.status = "Codex disconnected"
			a.toast = "Codex app-server stopped. Reconnect in Settings."
			if a.assistant != nil {
				a.assistant.Busy = false
				a.assistant.ThreadID = ""
				a.assistant.TurnID = ""
				a.assistant.Status = a.toast
			}
			for _, c := range a.state.Chats {
				if c.Busy() {
					c.Status = "error"
				}
			}
		}
	})
}
func (a *App) event(m codex.Message) {
	a.eventDecoded(m, codex.Decode(m.Params))
}
func (a *App) eventDecoded(m codex.Message, p map[string]any) {
	if m.Method == "fastrock/updateStatus" {
		json.Unmarshal(m.Params, &a.updateStatus)
		if a.updateStatus.State == "ready" || a.updateStatus.State == "error" {
			a.toast = a.updateStatus.Message
		}
		return
	}
	if a.transferEvent(m) {
		return
	}
	id := str(p, "threadId")
	if len(m.ID) == 0 && (m.Sequence == 0 || m.Sequence > a.crossSequence) {
		a.crossTabEvent(m.Method, p)
		if m.Sequence > 0 {
			a.crossSequence = m.Sequence
		}
	}
	if _, waiting := a.transferBuffer[id]; waiting {
		a.transferBuffer[id] = append(a.transferBuffer[id], m)
		return
	}
	if a.detached[id] {
		return
	}
	if len(m.ID) > 0 {
		a.serverRequest(m, p)
		return
	}
	if a.assistant != nil && id == a.assistant.ThreadID {
		a.assistantEvent(m, p)
		return
	}
	c := a.state.Chats[id]
	if c == nil {
		return
	}
	if m.Sequence > 0 {
		if m.Sequence <= c.EventSequence {
			return
		}
		c.EventSequence = m.Sequence
	}
	switch m.Method {
	case "turn/started":
		t, _ := p["turn"].(map[string]any)
		c.TurnID = str(t, "id")
		c.Status = "running"
	case "turn/completed":
		t, _ := p["turn"].(map[string]any)
		c.Status = "idle"
		if str(t, "status") == "failed" {
			c.Status = "error"
			if err, ok := t["error"].(map[string]any); ok {
				a.toast = str(err, "message")
			}
		}
		c.TurnID = ""
		if d, ok := c.Pop(); ok {
			a.startTurn(c, d.Text, d.Attachments, "send")
		}
	case "item/started", "item/completed":
		if it, ok := p["item"].(map[string]any); ok {
			a.upsertItem(c, it)
		}
	case "item/agentMessage/delta":
		c.Append(str(p, "itemId"), "agentMessage", "assistant", str(p, "delta"))
	case "item/reasoning/summaryTextDelta", "item/reasoning/textDelta":
		c.Append(str(p, "itemId"), "reasoning", "reasoning", str(p, "delta"))
	case "item/commandExecution/outputDelta":
		c.Append(str(p, "itemId"), "commandExecution", "tool", str(p, "delta"))
	case "thread/name/updated":
		if name := str(p, "threadName"); name != "" {
			a.invalidateSidebar()
			c.Title = name
			for i := range a.state.Tabs {
				if a.state.Tabs[i].Target == c.ID {
					a.state.Tabs[i].Title = name
				}
			}
		}
	case "thread/tokenUsage/updated":
		if usage, ok := p["tokenUsage"].(map[string]any); ok {
			if total, ok := usage["total"].(map[string]any); ok {
				c.Tokens = int(integer(total, "totalTokens"))
			}
		}
	case "error":
		c.Status = "error"
		if err, ok := p["error"].(map[string]any); ok {
			a.toast = str(err, "message")
		}
	}
	if m.Method == "turn/started" || m.Method == "turn/completed" {
		a.invalidateSidebar()
	}
	c.Updated = time.Now().Unix()
}
func (a *App) upsertItem(c *workspace.Conversation, it map[string]any) {
	id, kind := str(it, "id"), str(it, "type")
	body := str(it, "text")
	role := "assistant"
	switch kind {
	case "userMessage":
		role = "you"
		var parts []string
		if content, ok := it["content"].([]any); ok {
			for _, v := range content {
				p, _ := v.(map[string]any)
				if s := str(p, "text"); s != "" {
					parts = append(parts, s)
				} else if s := str(p, "path"); s != "" {
					parts = append(parts, "[Image: "+s+"]")
				}
			}
		}
		body = strings.Join(parts, "\n")
	case "reasoning":
		role = "reasoning"
		for _, k := range []string{"summary", "content"} {
			if lines, ok := it[k].([]any); ok {
				for _, v := range lines {
					if s, ok := v.(string); ok {
						body += s + "\n"
					}
				}
			}
		}
	case "commandExecution":
		role = "tool"
		body = str(it, "command") + "\n" + str(it, "aggregatedOutput")
	case "fileChange":
		role = "changes"
		b, _ := json.MarshalIndent(it["changes"], "", "  ")
		body = string(b)
	case "collabAgentToolCall":
		role = "tool"
		body = str(it, "tool") + " · " + str(it, "status")
		c.Agents = append([]workspace.Agent(nil), c.Agents...)
		if states, ok := it["agentsStates"].(map[string]any); ok {
			for id, value := range states {
				state, _ := value.(map[string]any)
				agent := workspace.Agent{ID: id, Name: str(state, "nickname"), Status: str(state, "status")}
				found := false
				for i := range c.Agents {
					if c.Agents[i].ID == id {
						c.Agents[i] = agent
						found = true
						break
					}
				}
				if !found && len(c.Agents) < 64 {
					c.Agents = append(c.Agents, agent)
				}
			}
		}
	case "mcpToolCall", "dynamicToolCall", "webSearch":
		role = "tool"
		b, _ := json.MarshalIndent(it, "", "  ")
		body = string(b)
	}
	for i := range c.Blocks {
		if c.Blocks[i].ID == id {
			if body != "" {
				c.Blocks[i].Text = body
			}
			c.Blocks[i].Status = str(it, "status")
			return
		}
	}
	c.Append(id, kind, role, body)
}
func (a *App) drawChat(w *nucular.Window, id string) {
	c := a.state.Chats[id]
	v := a.chats[id]
	if c == nil || v == nil {
		muted(w, "Loading conversation…", a.p)
		return
	}
	w.Row(30).Ratio(.62, .38)
	w.Label(c.Title, "LC")
	w.LabelColored(filepath.Base(c.Cwd)+" · "+c.Status, "RC", a.p.Muted)
	if c.EphemeralLost {
		muted(w, "This temporary side chat ended when Codex restarted. Its text is retained; start a new chat to continue.", a.p)
	}
	w.Row(24).Static(95, 95, 95, 90, 110)
	if w.ButtonText("Top") {
		v.Follow = false
		v.ShowBlocks = len(c.Blocks)
		v.PreviousScroll = -1
	}
	if w.ButtonText("Latest") {
		v.Follow = true
	}
	if w.ButtonText("Expand all") {
		for _, b := range c.Blocks {
			v.Expanded[b.ID] = true
		}
	}
	if w.ButtonText("Collapse all") {
		clear(v.Expanded)
	}
	if w.ButtonText("Copy last reply") {
		for i := len(c.Blocks) - 1; i >= 0; i-- {
			if c.Blocks[i].Role == "assistant" {
				a.copyText(c.Blocks[i].Text)
				break
			}
		}
	}
	approvalHeight := 0
	if len(a.approvals) > 0 {
		approvalHeight = 190
	}
	queueHeight := min(len(c.Queue), 4)*32 + min(8, len(v.Suggest))*24
	h := max(120, w.LayoutAvailableHeight()-190-queueHeight-approvalHeight)
	w.Row(h).Dynamic(1)
	if tr := w.GroupBegin("transcript-"+id, nucular.WindowNoHScrollbar); tr != nil {
		if v.RestoreScroll {
			tr.Scrollbar.Y = v.Scroll
			v.RestoreScroll = false
		}
		if v.PreviousScroll == -1 {
			tr.Scrollbar.Y = 0
			v.PreviousScroll = 0
		}
		input := a.window.Input()
		if input.Mouse.ScrollDelta > 0 && input.Mouse.HoveringRect(tr.Bounds) {
			v.Follow = false
		}

		if len(c.Blocks) == 0 {
			muted(tr, "Ask Codex to explore, build, or fix something in this project.", a.p)
		}
		if len(c.Blocks) > v.ShowBlocks {
			tr.Row(28).Dynamic(1)
			if tr.ButtonText("Load older messages") {
				v.ShowBlocks += 200
				v.Follow = false
			}
		}
		blocks := c.Blocks[max(0, len(c.Blocks)-v.ShowBlocks):]
		offset := 0
		for _, b := range blocks {
			folded := (b.Role == "tool" || b.Role == "reasoning" || b.Role == "changes") && !v.Expanded[b.ID]
			layout := a.transcriptLayout(v, b, tr.LayoutAvailableWidth())
			height := 32
			if v.SelectID == b.ID && v.Selection != nil {
				height += min(500, max(120, layout.Height)) + 24
			} else if !folded {
				height += layout.Height + 8
			}
			visible := offset+height >= tr.Scrollbar.Y-100 && offset <= tr.Scrollbar.Y+tr.Bounds.H+100
			offset += height
			if !visible {
				tr.Row(height).Dynamic(1)
				tr.Spacing(1)
				continue
			}
			tr.Row(24).Dynamic(1)
			if b.Role == "tool" || b.Role == "reasoning" || b.Role == "changes" {
				if button(tr, b.Role+" · "+cut(b.Text, 80), v.Expanded[b.ID], a.p) {
					v.Expanded[b.ID] = !v.Expanded[b.ID]
				}
			} else {
				tr.LabelColored(strings.ToUpper(b.Role), "LC", a.p.Muted)
			}
			a.transcriptMenu(tr, c, v, b)
			if folded {
				continue
			}
			if v.SelectID == b.ID && v.Selection != nil {
				tr.Row(min(500, max(120, layout.Height))).Dynamic(1)
				v.Selection.Edit(tr)
				a.transcriptMenu(tr, c, v, b)
				tr.Row(24).Static(160)
				if tr.ButtonText("Done selecting text") {
					v.SelectID = ""
					v.Selection = nil
				}
				continue
			}
			for _, line := range layout.Lines {
				tr.Row(line.Height).Dynamic(1)
				if !a.drawTranscriptLine(tr, v, b.ID, layout, line) {
					a.transcriptMenu(tr, c, v, b)
				}
			}
			tr.Row(8).Dynamic(1)
			tr.Spacing(1)
		}

		previous := tr.Scrollbar.Y
		if !input.Mouse.Down(1) {
			v.RichSelection.Dragging = false
		}
		if v.RichSelection.BlockID != "" && !v.Editor.Active {
			a.transcriptSelectionKeys(tr, v)
		}
		if v.Follow {
			tr.Scrollbar.Y = 100000000
		}
		tr.GroupEnd()
		v.Scroll = tr.Scrollbar.Y
		if v.Follow && tr.Scrollbar.Y != previous {
			a.window.Changed()
		}
	}
	if !v.Follow {
		w.Row(25).Static(145)
		if w.ButtonText("Jump to latest ↓") {
			v.Follow = true
		}
	}
	if len(a.approvals) > 0 {
		a.drawApproval(w)
	}
	if len(c.Queue) > 4 {
		w.Row(25).Dynamic(3)
		if w.ButtonText("Earlier queued") {
			v.QueuePage = max(0, v.QueuePage-1)
		}
		w.Label(fmt.Sprintf("%d queued", len(c.Queue)), "LC")
		if w.ButtonText("Later queued") {
			v.QueuePage++
		}
	}
	v.QueuePage = min(v.QueuePage, max(0, (len(c.Queue)-1)/4))
	start := v.QueuePage * 4
	for _, d := range c.Queue[start:min(start+4, len(c.Queue))] {
		w.Row(28).Ratio(.42, .12, .12, .10, .10, .14)
		w.LabelColored("Queued: "+cut(d.Text, 90), "LC", a.p.Muted)
		if w.ButtonText("Edit") {
			v.EditQueue = d.ID
			setText(v.Editor, d.Text)
		}
		if w.ButtonText("Delete") {
			c.DeleteQueued(d.ID)
		}
		if w.ButtonText("↑") {
			for i := 1; i < len(c.Queue); i++ {
				if c.Queue[i].ID == d.ID {
					c.Queue[i], c.Queue[i-1] = c.Queue[i-1], c.Queue[i]
					break
				}
			}
		}
		if w.ButtonText("↓") {
			for i := 0; i+1 < len(c.Queue); i++ {
				if c.Queue[i].ID == d.ID {
					c.Queue[i], c.Queue[i+1] = c.Queue[i+1], c.Queue[i]
					break
				}
			}
		}
		if w.ButtonText("Send now") {
			c.DeleteQueued(d.ID)
			a.startTurn(c, d.Text, d.Attachments, "steer")
		}
	}
	a.suggestions(w, c, v)
	w.Row(76).Dynamic(1)
	v.Editor.Edit(w)
	w.Row(29).Static(210, 112, 110, 82, 110, 36)
	models := a.catalog.Models
	labels := []string{}
	selection := 0
	for i, m := range models {
		labels = append(labels, m.Name)
		if m.Model == c.Model || m.ID == c.Model {
			selection = i
		}
	}
	if len(labels) > 0 {
		next := w.ComboSimple(labels, selection, 28)
		if next != selection {
			c.Model = models[next].Model
			c.Effort = models[next].DefaultEffort
			if a.catalog.SpeedWarning(c.Model, c.Tier) != "" {
				c.Tier = "default"
			}
		}
	} else {
		w.Label("Model catalog loading…", "LC")
	}
	m, _ := a.catalog.Find(c.Model)
	efforts := []string{}
	ei := 0
	for i, e := range m.Efforts {
		efforts = append(efforts, e.ID)
		if e.ID == c.Effort {
			ei = i
		}
	}
	if len(efforts) > 0 {
		c.Effort = efforts[w.ComboSimple(efforts, ei, 28)]
	} else {
		w.Label("Default effort", "LC")
	}
	tiers := a.catalog.Speeds(m)
	names := []string{}
	ti := 0
	for i, t := range tiers {
		names = append(names, t.Name)
		if t.ID == c.Tier {
			ti = i
		}
	}
	next := w.ComboSimple(names, ti, 28)
	if next != ti {
		c.Tier = tiers[next].ID
	}
	if button(w, "Plan", c.Plan, a.p) {
		c.Plan = !c.Plan
	}
	if c.Busy() {
		if w.ButtonText("Stop") {
			a.rpc("turn/interrupt", map[string]any{"threadId": c.ID, "turnId": c.TurnID}, nil)
		}
	} else {
		w.Label("Ready", "LC")
	}
	if w.ButtonText("+") {
		a.attachDialog(v)
	}
	if len(v.Attachments) > 0 {
		for i, path := range v.Attachments {
			w.Row(26).Ratio(.85, .15)
			w.Label(filepath.Base(path), "LC")
			if w.ButtonText("Remove") {
				v.Attachments = append(v.Attachments[:i], v.Attachments[i+1:]...)
				break
			}
		}
	}
	w.Row(32).Ratio(.66, .17, .17)
	w.LabelColored(fmt.Sprintf("%s · %d tokens", c.Model, c.Tokens), "LC", a.p.Faint)
	if c.Busy() {
		if w.ButtonText("Steer") {
			a.send(c, "steer")
		}
		if primary(w, "Queue", a.p) {
			a.send(c, "queue")
		}
	} else {
		if v.EditQueue != "" {
			if w.ButtonText("Cancel edit") {
				v.EditQueue = ""
				setText(v.Editor, "")
			}
		} else {
			w.Spacing(1)
		}
		caption := "Send ↑"
		if v.EditQueue != "" {
			caption = "Save queued"
		}
		if primary(w, caption, a.p) {
			a.send(c, "send")
		}
	}
}
func (a *App) drawInfo(w *nucular.Window) {
	tab := a.state.Current()
	if tab == nil {
		return
	}
	c := a.state.Chats[tab.Target]
	if c == nil {
		return
	}
	title(w, "Conversation", a.p)
	muted(w, c.Title, a.p)
	title(w, "Project", a.p)
	w.Row(56).Dynamic(1)
	w.LabelWrap(c.Cwd)
	title(w, "Session", a.p)
	muted(w, c.Status, a.p)
	muted(w, c.Model, a.p)
	muted(w, c.Effort+" · "+c.Tier, a.p)
	w.Row(28).Dynamic(1)
	if w.ButtonText("Rename…") {
		a.inputDialog("Rename conversation", c.Title, func(name string) {
			a.rpc("thread/name/set", map[string]any{"threadId": c.ID, "name": name}, func(_ json.RawMessage) { a.invalidateSidebar(); c.Title = name; tab.Title = name })
		})
	}
	w.Row(28).Dynamic(1)
	if w.ButtonText("Fork conversation") {
		a.rpc("thread/fork", map[string]any{"threadId": c.ID}, func(raw json.RawMessage) {
			r := codex.Decode(raw)
			t, _ := r["thread"].(map[string]any)
			id := str(t, "id")
			if id != "" {
				a.state.Chats[id] = &workspace.Conversation{ID: id, Title: threadTitle(t), Cwd: c.Cwd, Updated: time.Now().Unix()}
				a.resumeThread(id)
			}
		})
	}
	w.Row(28).Dynamic(1)
	archiveLabel, archiveMethod := "Archive", "thread/archive"
	if c.Archived {
		archiveLabel, archiveMethod = "Unarchive", "thread/unarchive"
	}
	if w.ButtonText(archiveLabel) {
		a.rpc(archiveMethod, map[string]any{"threadId": c.ID}, func(_ json.RawMessage) { a.invalidateSidebar(); c.Archived = !c.Archived; a.state.Close(tab.ID) })
	}
	w.Row(28).Dynamic(1)
	if w.ButtonText("Export Markdown…") {
		a.exportChat(c)
	}
	a.extraInfo(w, c)
	title(w, "Document tabs", a.p)
	w.Row(28).Dynamic(2)
	if w.ButtonText("← Move") {
		a.state.Move(tab.ID, -1)
	}
	if w.ButtonText("Move →") {
		a.state.Move(tab.ID, 1)
	}
}

func mergeTranscript(history, live []workspace.Block) []workspace.Block {
	if len(live) == 0 {
		return history
	}
	positions := make(map[string]int, len(history))
	for i, b := range history {
		positions[b.ID] = i
	}
	for _, b := range live {
		if i, ok := positions[b.ID]; ok {
			if len(b.Text) >= len(history[i].Text) {
				history[i] = b
			}
		} else {
			history = append(history, b)
		}
	}
	return history
}
