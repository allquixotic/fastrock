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
	Cwd                         string
	HistoryCursor, HistoryError string
	HistoryLoading              bool
	HistoryWindowed             bool
	LayoutBlockCount            int
	LayoutFirstID               string
	LayoutClock                 uint64
	LoadError                   string
	RichSelection               transcriptSelection
	SelectID                    string
	Selection                   *nucular.TextEditor
	Suggest                     []string
	SuggestQuery                string
	SuggestGeneration           uint64
	SuggestIndex                int
	SuggestCancel               context.CancelFunc
	Layouts                     map[string]*transcriptLayout
	ShowBlocks                  int
	Scroll                      int
	RestoreScroll               bool

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
func (a *App) fetchThreads(client *codex.Client, archived bool, cursor string, page *threadPageState, request, revision uint64) {
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
		if a.client != client || page.request != request {
			return
		}
		if e != nil {
			page.loading, page.err = false, e.Error()
			return
		}
		if r.Next == cursor {
			r.Next = ""
		}
		a.applyThreadPage(archived, cursor, r.Next, r.Data, page, revision)
	})
}
func str(m map[string]any, k string) string    { s, _ := m[k].(string); return s }
func integer(m map[string]any, k string) int64 { n, _ := m[k].(float64); return int64(n) }
func threadTitle(t map[string]any) string {
	for _, k := range []string{"name", "preview", "id"} {
		if s := str(t, k); s != "" {
			if k != "preview" {
				return s
			}
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
	if a.newThreadPending {
		a.toast = "A new conversation is already starting"
		return
	}
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
	origin := ""
	if current := a.state.Current(); current != nil && current.Kind == workspace.New {
		origin = current.ID
	}
	a.newThreadPending = true
	a.rpcResult("thread/start", params, func(raw json.RawMessage) {
		a.newThreadPending = false
		r := codex.Decode(raw)
		t, _ := r["thread"].(map[string]any)
		id := str(t, "id")
		if id == "" {
			a.toast = "Codex returned no thread ID"
			return
		}
		c := &workspace.Conversation{ID: id, Title: "New conversation", Cwd: str(t, "cwd"), Model: str(r, "model"), Tier: str(r, "serviceTier"), Effort: str(r, "reasoningEffort"), Status: "idle", Updated: time.Now().Unix()}
		c.NoMessages = !a.prefs.AgentMessages
		c.Settings = threadSettings(r)
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
		a.invalidateSidebar(id)
		a.chats[id] = newChatView()
		delete(a.detached, id)
		a.state.CompleteNew(origin, c.Title, id)
		if cwd != "" {
			a.prefs.WorkingDirectory = cwd
			a.rememberFolder(cwd)
		}
		a.savePrefs()
	}, func(error) { a.newThreadPending = false })
}
func (a *App) resumeThread(id string) {
	a.invalidateSidebar(id)
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
	c.Unread = false
	if a.chats[id] != nil {
		return
	}
	a.chats[id] = newChatView()
	setText(a.chats[id].Editor, c.Draft)
	a.chats[id].Attachments = c.DraftAttachments
	if a.serverPaused || c.EphemeralLost || a.client == nil {
		a.chats[id].LoadError = "Codex is disconnected. Reconnect in Settings, then retry."
		return
	}
	c.Status = "starting"
	client := a.client
	view := a.chats[id]
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var raw json.RawMessage
		err := client.Call(ctx, "thread/resume", map[string]any{"threadId": id, "excludeTurns": true}, &raw)
		r := codex.Decode(raw)
		prepared := &workspace.Conversation{Status: "idle", Model: str(r, "model"), Effort: str(r, "reasoningEffort"), Tier: str(r, "serviceTier")}
		prepared.Settings = threadSettings(r)
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
		var history itemPage
		var historyErr error
		if err == nil {
			history, historyErr = readItemPage(ctx, client, id, "")
			prepared.Blocks = a.prepareItemPage(history)
			var turns struct{ Data []struct{ ID, Status string } }
			if client.Call(ctx, "thread/turns/list", map[string]any{"threadId": id, "limit": 1, "sortDirection": "desc", "itemsView": "notLoaded"}, &turns) == nil && len(turns.Data) > 0 && turns.Data[0].Status == "inProgress" {
				prepared.TurnID, prepared.Status = turns.Data[0].ID, "running"
			}
		}
		a.post(func() {
			if a.chats[id] != view || a.client != client {
				return
			}
			if err != nil {
				c.Status = "error"
				view.LoadError = err.Error()
				a.report(err)
				return
			}
			c.Model, c.Effort, c.Tier = prepared.Model, prepared.Effort, prepared.Tier
			c.Settings = prepared.Settings
			if c.Model == "" {
				c.Model = a.catalog.DefaultModel()
			}
			if c.Status == "starting" {
				c.Status, c.TurnID = prepared.Status, prepared.TurnID
			}
			c.Blocks = mergeTranscript(prepared.Blocks, c.Blocks)
			view.HistoryCursor = history.Next
			if historyErr != nil {
				view.HistoryError = "History unavailable: " + historyErr.Error()
			}
			if len(c.Agents) == 0 {
				c.Agents = prepared.Agents
			}
			c.TrimTranscript()
			a.dispatchQueue(c)
		})
	}, func() {
		if a.chats[id] == view && a.client == client {
			c.Status = "error"
			view.LoadError = errWorkQueueFull.Error()
		}
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
	if query := text(a.sidebarSearch); query != a.threadSearch.query || a.archived != a.threadSearch.archived {
		a.searchThreads(query, a.archived, "")
	}
	if a.threadSearch.loading {
		muted(w, "Searching conversation history…", a.p)
	}
	if a.threadSearch.err != "" {
		muted(w, a.threadSearch.err, a.p)
		w.Row(26).Dynamic(1)
		if w.ButtonText("Retry search") {
			a.searchThreads(text(a.sidebarSearch), a.archived, "")
		}
	}
	w.Row(26).Dynamic(2)
	if flatRow(w, "Recent", "", !a.archived, color.RGBA{}, a.p) {
		a.archived = false
	}
	if flatRow(w, "Archived", "", a.archived, color.RGBA{}, a.p) {
		a.archived = true
		a.requestThreads(true, "")
	}
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	rowHeight := int(28 * w.Master().Style().Scaling)
	stride := rowHeight + spacing
	a.sidebarFolders()
	cache := a.sidebarLayout()
	top := w.WidgetBounds().Y
	first, last := sidebarVisible(top, w.Bounds.Y, w.Bounds.Y+w.Bounds.H, stride, cache.totalRows)
	sidebarSkip(w, first, stride, spacing)
	folderIndex := cache.folderAt(first)
	for row := first; row < last; row++ {
		for folderIndex < len(cache.folders) && row >= cache.starts[folderIndex+1] {
			folderIndex++
		}
		folder := cache.folders[folderIndex]
		w.RowScaled(rowHeight).Dynamic(1)
		if row == cache.starts[folderIndex] {
			if folderRow(w, folder.title, !a.collapsed[folder.path], a.p) {
				a.collapseFolder(folder.path, !a.collapsed[folder.path])
			}
			a.folderContext(w, folder.path)
			continue
		}
		c := a.state.Chats[folder.rows[row-cache.starts[folderIndex]-1].ID]
		if c == nil {
			w.Spacing(1)
			continue
		}
		dot := a.conversationDot(c, a.chatIsOpen(c.ID))
		active := false
		if tab := a.state.Current(); tab != nil {
			active = tab.Target == c.ID
		}
		if flatStatusRow(w, c.Title, conversationAge(c.Updated, time.Now()), active, dot, 22, a.p) {
			a.resumeThread(c.ID)
		}
		a.sidebarContext(w, c)
	}
	sidebarSkip(w, cache.totalRows-last, stride, spacing)
	if a.sidebarCache.err != "" {
		muted(w, "Could not prepare conversations: "+a.sidebarCache.err, a.p)
		w.Row(26).Dynamic(1)
		if w.ButtonText("Retry preparing conversations") {
			a.invalidateSidebarView()
		}
	} else if !a.sidebarCache.ready {
		muted(w, "Preparing conversations…", a.p)
	} else if message := a.sidebarEmptyMessage(); message != "" {
		muted(w, message, a.p)
	}
	a.sidebarPaging(w)
	w.Row(26).Dynamic(1)
	if flatRow(w, "Refresh history", "", false, color.RGBA{}, a.p) {
		a.requestThreads(a.archived, "")
	}

}
func (a *App) send(c *workspace.Conversation, mode string) {
	if mode == "send" && a.prefs.BusyInput == "steer" && c.Busy() {
		mode = "steer"
	}
	v := a.chats[c.ID]
	if v == nil {
		return
	}
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
	if c.EditQueue != "" {
		for i := range c.Queue {
			if c.Queue[i].ID == c.EditQueue {
				c.Queue[i].Text = value
				c.Queue[i].Attachments = append([]string(nil), v.Attachments...)
			}
		}
		finishQueueEdit(c, v)
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
		a.invalidateSidebar(c.ID)
		c.Title = cut(strings.SplitN(value, "\n", 2)[0], 65)
		for i := range a.state.Tabs {
			if a.state.Tabs[i].Target == c.ID {
				a.state.Tabs[i].Title = c.Title
			}
		}
	}
	if a.startTurn(c, value, v.Attachments, mode) {
		v.Attachments = nil
		setText(v.Editor, "")
	}
}
func (a *App) startTurn(c *workspace.Conversation, value string, attachments []string, mode string) bool {
	return a.submitTurn(c, value, attachments, mode, "")
}
func (a *App) submitTurn(c *workspace.Conversation, value string, attachments []string, mode, queueID string) bool {
	if a.serverPaused || c.EphemeralLost || a.client == nil {
		a.toast = "Codex is disconnected or restarting; your draft is retained"
		return false
	}
	if c.Busy() && mode != "steer" {
		return false
	}
	if warning := a.catalog.SpeedWarning(c.Model, fallback(c.Tier, "default")); warning != "" {
		a.toast = warning
		return false
	}
	if c.Busy() && c.TurnID == "" {
		c.Enqueue(value, attachments)
		return true
	}
	attachments = append([]string(nil), attachments...)
	input := codex.TextInput(value)
	id := workspace.NewID("fastrock")
	pending := workspace.Draft{ID: id, Text: value, Attachments: append([]string(nil), attachments...), Status: "pending"}
	c.Outbox = append(c.Outbox, pending)
	if queueID != "" {
		for i := range c.Queue {
			if c.Queue[i].ID == queueID {
				c.Queue[i].Status = "pending"
			}
		}
	}
	params := map[string]any{"threadId": c.ID, "input": input, "clientUserMessageId": id}
	method := "turn/start"
	if c.Busy() {
		method = "turn/steer"
		params["expectedTurnId"] = c.TurnID
	} else {
		if c.Model != "" {
			params["model"] = c.Model
		}
		if c.Tier != "" {
			params["serviceTier"] = c.Tier
		}
		if c.Effort != "" {
			params["effort"] = c.Effort
		}
		if c.Plan {
			params["collaborationMode"] = map[string]any{"mode": "plan", "settings": map[string]any{"model": c.Model, "reasoning_effort": c.Effort, "developer_instructions": nil}}
		}
		c.Status = "starting"
	}
	client, chatID, cwd := a.client, c.ID, c.Cwd
	a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var raw json.RawMessage
		attachmentInput, err := attachmentInputs(attachments)
		if err == nil {
			params["input"] = append(attachmentInput, input...)
			if strings.Contains(value, "$") {
				var skills skillList
				var skillErr error
				skills, skillErr = a.skillCache.get(ctx, a.ctx, client, cwd)
				if skillErr == nil {
					params["input"] = append(params["input"].([]map[string]any), skillInputs(value, skills)...)
				}
			}
		}
		if err == nil {
			err = client.Call(ctx, method, params, &raw)
		}
		a.post(func() {
			chat := a.state.Chats[chatID]
			if chat == nil || a.detached[chatID] {
				return
			}
			if err != nil {
				if method == "turn/start" && chat.Status == "starting" {
					chat.Status = "error"
				}
				chat.QueuePaused = true
				for i := range chat.Outbox {
					if chat.Outbox[i].ID == id {
						chat.Outbox[i].Status = "unconfirmed"
						chat.Outbox[i].Error = err.Error()
					}
				}
				for i := range chat.Queue {
					if chat.Queue[i].ID == queueID {
						chat.Queue[i].Status = "unconfirmed"
					}
				}
				a.report(err)
				return
			}
			for i := range chat.Outbox {
				if chat.Outbox[i].ID == id {
					chat.Outbox = append(chat.Outbox[:i], chat.Outbox[i+1:]...)
					break
				}
			}
			if queueID != "" {
				chat.DeleteQueued(queueID)
			}
			if method == "turn/start" && chat.Status == "starting" {
				r := codex.Decode(raw)
				turn, _ := r["turn"].(map[string]any)
				chat.TurnID = str(turn, "id")
				chat.Status = "running"
			}
			chat.Updated = time.Now().Unix()
			a.invalidateSidebar(chat.ID)
		})
	}, func() {
		c.QueuePaused = true
		if c.Status == "starting" {
			c.Status = "error"
		}
		for i := range c.Outbox {
			if c.Outbox[i].ID == id {
				c.Outbox[i].Status = "not sent"
				c.Outbox[i].Error = errWorkQueueFull.Error()
			}
		}
	})
	return true
}
func finishQueueEdit(c *workspace.Conversation, v *chatView) {
	setText(v.Editor, c.QueueDraft.Text)
	v.Attachments = append([]string(nil), c.QueueDraft.Attachments...)
	c.EditQueue = ""
	c.QueueDraft = workspace.Draft{}
}
func beginQueueEdit(c *workspace.Conversation, v *chatView, d workspace.Draft) {
	if c.EditQueue == "" {
		c.QueueDraft = workspace.Draft{Text: text(v.Editor), Attachments: append([]string(nil), v.Attachments...)}
	}
	c.EditQueue = d.ID
	setText(v.Editor, d.Text)
	v.Attachments = append([]string(nil), d.Attachments...)
}
func (a *App) dispatchQueue(c *workspace.Conversation) {
	if a.windowReturn != nil || c.Busy() || c.QueuePaused || c.EditQueue != "" || len(c.Queue) == 0 || a.client == nil {
		return
	}
	for _, t := range a.state.Tabs {
		if t.Target == c.ID && a.popping[t.ID] {
			return
		}
	}
	d := c.Queue[0]
	if d.Status != "" {
		return
	}
	a.submitTurn(c, d.Text, d.Attachments, "send", d.ID)
}

func (a *App) consume(client *codex.Client) {
	for m := range client.Events {
		message := m

		if !a.consumeEvent(client, message) {
			return
		}
	}
	a.events.push(a.ctx, decodedEvent{client: client, disconnected: true})
	if a.window != nil {
		a.window.Changed()
	}
}
func (a *App) disconnected(client *codex.Client) {
	if a.ctx.Err() == nil {
		if a.client != client {
			return
		}
		a.client = nil
		a.cancelRecaps()
		a.resetSettingsConnection()
		a.serverGeneration++
		a.catalogGeneration++
		a.catalog.PolicyLoaded = false
		a.serverStarting = false
		a.serverError = "Connection to the Codex service was lost."
		a.approvals = nil
		a.approvalDiffs = nil
		a.clearReplyWaits()
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
}

func (a *App) event(m codex.Message) {
	a.eventDecoded(m, codex.Decode(m.Params))
}
func (a *App) eventDecoded(m codex.Message, p map[string]any) {
	if a.recapEvent(m, p) {
		return
	}
	a.approvalEvent(m.Method, p)
	a.settingsEvent(m.Method, p)
	if m.Method == "configWarning" || m.Method == "warning" || m.Method == "deprecationNotice" {
		message := fallback(str(p, "message"), str(p, "summary"))
		if message == "" {
			message = m.Method + ": see Codex logs for details"
		}
		if !contains(a.warnings, message) {
			a.warnings = append(a.warnings, message)
			if len(a.warnings) > 16 {
				a.warnings = a.warnings[len(a.warnings)-16:]
			}
		}
		return
	}
	if m.Method == "fastrock/frameError" {
		a.toast = str(p, "message")
		return
	}
	if m.Method == "serverRequest/resolved" {
		a.pruneApprovals("", fmt.Sprint(p["requestId"]))
		return
	}
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
	a.infoEvent(c, m.Method, p)
	switch m.Method {
	case "thread/goal/updated", "thread/goal/cleared":
		if v := a.infoViews[id]; v != nil {
			v.GoalGeneration++
			v.Goal, _ = p["goal"].(map[string]any)
			v.GoalLoading, v.GoalUnsupported, v.GoalAvailable, v.GoalNote = false, false, true, ""
		}
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
		if status := str(t, "status"); status == "failed" || status == "interrupted" {
			c.QueuePaused = true
		}
		a.pruneApprovals(c.ID, "")
		a.dispatchQueue(c)
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
			a.invalidateSidebar(c.ID)
			c.Title = name
			for i := range a.state.Tabs {
				if a.state.Tabs[i].Target == c.ID {
					a.state.Tabs[i].Title = name
				}
			}
		}
	case "turn/plan/updated":
		if plan, ok := p["plan"].([]any); ok {
			var lines []string
			for _, raw := range plan {
				if step, ok := raw.(map[string]any); ok {
					lines = append(lines, str(step, "status")+": "+str(step, "step"))
				}
			}
			body := strings.Join(lines, "\n")
			if !c.ReplaceBlock("plan-"+c.TurnID, body, "") {
				c.Append("plan-"+c.TurnID, "plan", "activity", body)
			}
		}
	case "thread/tokenUsage/updated":
		if usage, ok := p["tokenUsage"].(map[string]any); ok {
			if total, ok := usage["total"].(map[string]any); ok {
				c.Tokens = int(integer(total, "totalTokens"))
				c.InputTokens = int(integer(total, "inputTokens"))
				c.CachedTokens = int(integer(total, "cachedInputTokens"))
				c.OutputTokens = int(integer(total, "outputTokens"))
			}
			if last, ok := usage["last"].(map[string]any); ok {
				c.ContextTokens = int(integer(last, "totalTokens"))
			}
			c.ContextWindow = int(integer(usage, "modelContextWindow"))
		}
	case "error":
		if retry, _ := p["willRetry"].(bool); retry {
			break
		}
		c.QueuePaused = true
		c.Status = "error"
		if err, ok := p["error"].(map[string]any); ok {
			a.toast = str(err, "message")
			c.Append(workspace.NewID("error"), "error", "activity", a.toast)
		}
	}
	if m.Method == "turn/started" || m.Method == "turn/completed" {
		a.invalidateSidebar(c.ID)
	}
	if m.Method == "item/completed" || m.Method == "turn/completed" || m.Method == "error" {
		current := a.state.Current()
		if current == nil || current.Kind != workspace.Chat || current.Target != id {
			c.Unread = true
		}
	}
	c.Updated = time.Now().Unix()
}
func formatItem(it map[string]any) workspace.Block {
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

	case "mcpToolCall", "dynamicToolCall", "webSearch":
		role = "tool"
		b, _ := json.MarshalIndent(it, "", "  ")
		body = string(b)
	default:
		if kind != "agentMessage" && body == "" {
			role = "activity"
			b, _ := json.MarshalIndent(it, "", "  ")
			body = string(b)
		}
	}
	return workspace.Block{ID: id, Kind: kind, Role: role, Text: body, Status: str(it, "status")}
}
func (a *App) upsertItem(c *workspace.Conversation, it map[string]any) {
	b := formatItem(it)
	updateAgents(c, it)
	if c.ReplaceBlock(b.ID, b.Text, b.Status) {
		return
	}
	c.Append(b.ID, b.Kind, b.Role, b.Text)
	c.FinishBlock(b.ID)
}
func (a *App) drawChat(w *nucular.Window, id string) {
	c := a.state.Chats[id]
	v := a.chats[id]
	if v != nil && v.LoadError != "" {
		w.Row(48).Dynamic(1)
		w.LabelWrap(v.LoadError)
		w.Row(28).Static(100)
		if w.ButtonText("Retry") {
			if c != nil {
				c.Draft = text(v.Editor)
				c.DraftAttachments = append([]string(nil), v.Attachments...)
			}
			delete(a.chats, id)
			a.resumeThread(id)
		}
	}
	if c == nil || v == nil {
		muted(w, "Loading conversation…", a.p)
		return
	}
	v.Cwd = c.Cwd
	v.pruneLayouts(c.Blocks)
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
		if v.HistoryWindowed || v.HistoryError != "" {
			a.latestMessages(c, v)
		} else {
			v.Follow = true
		}
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
	a.drawRecapStatus(w, c)
	queueHeight := min(len(c.Queue), 4)*32 + min(8, len(v.Suggest))*24
	composer := composerHeight(v.Editor, w.LayoutAvailableWidth(), a.prefs.FontSize)
	scale := approvalScale(w)
	remaining := w.LayoutAvailableHeight()
	approvalHeight := 0
	if len(a.approvals) > 0 {
		budget := remaining - int(float64(120+114+composer+queueHeight)*scale)
		approvalHeight = a.approvalHeight(w, c.ID, budget)
	}
	h := max(120, int(float64(remaining-approvalHeight)/scale)-114-composer-queueHeight)
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
			if tr.ButtonText("Show earlier loaded messages") {
				v.ShowBlocks += 200
				v.Follow = false
			}
		}
		if v.HistoryError != "" {
			muted(tr, v.HistoryError, a.p)
			tr.Row(28).Dynamic(1)
			if tr.ButtonText("Retry history") {
				a.latestMessages(c, v)
			}
		}
		if v.HistoryCursor != "" {
			tr.Row(28).Dynamic(1)
			if v.HistoryLoading {
				muted(tr, "Loading earlier messages…", a.p)
			} else if tr.ButtonText("Load earlier messages") {
				a.olderMessages(c, v)
			}
		}
		blocks := c.Blocks[max(0, len(c.Blocks)-v.ShowBlocks):]
		offset := 0
		for _, b := range blocks {
			b = displayBlock(b)
			folded := (b.Role == "tool" || b.Role == "reasoning" || b.Role == "changes") && !v.Expanded[b.ID]
			layout := v.Layouts[b.ID]
			if layout == nil {
				layout = &transcriptLayout{Height: 48}
			}
			scale, spacing := tr.Master().Style().Scaling, tr.Master().Style().GroupWindow.Spacing.Y
			height := int(24*scale) + spacing
			if v.SelectID == b.ID && v.Selection != nil {
				height += int(float64(min(500, max(120, layout.Height))+24)*scale) + 2*spacing
			} else if !folded {
				height += int(float64(layout.Height+8)*scale) + (len(layout.Lines)+1)*spacing
			}
			visible := offset+height >= tr.Scrollbar.Y-100 && offset <= tr.Scrollbar.Y+tr.Bounds.H+100
			offset += height
			if !visible {
				tr.RowScaled(max(1, height-spacing)).Dynamic(1)
				tr.Spacing(1)
				continue
			}
			if !folded {
				layout = a.transcriptLayout(v, b, tr.LayoutAvailableWidth())
			}
			tr.Row(24).Dynamic(1)
			if b.Role == "tool" || b.Role == "reasoning" || b.Role == "changes" {
				if button(tr, b.Role+" · "+cut(b.Text, 80), v.Expanded[b.ID], a.p) {
					v.Expanded[b.ID] = !v.Expanded[b.ID]
				}
			} else {
				if b.Kind == "crossTabMessage" {
					tr.LabelColored(b.Role, "LC", a.p.Accent)
				} else {
					tr.LabelColored(strings.ToUpper(b.Role), "LC", a.p.Muted)
				}
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
			lineFirst, lineLast, leading, trailing := visibleTranscriptLines(layout, tr.WidgetBounds().Y, tr.Bounds.Y, tr.Bounds.Y+tr.Bounds.H, tr.Master().Style().Scaling, tr.Master().Style().GroupWindow.Spacing.Y)
			if leading > 0 {
				tr.RowScaled(leading - tr.Master().Style().GroupWindow.Spacing.Y).Dynamic(1)
				tr.Spacing(1)
			}
			for _, line := range layout.Lines[lineFirst:lineLast] {
				tr.Row(line.Height).Dynamic(1)
				if !a.drawTranscriptLine(tr, v, b.ID, layout, line) {
					a.transcriptMenu(tr, c, v, b)
				}
			}
			if trailing > 0 {
				tr.RowScaled(trailing - tr.Master().Style().GroupWindow.Spacing.Y).Dynamic(1)
				tr.Spacing(1)
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
		if !v.HistoryWindowed && !v.Follow && input.Mouse.ScrollDelta < 0 && tr.Scrollbar.Y >= max(0, offset-tr.Bounds.H-6) {
			v.Follow = true
		}
		if v.Follow {
			tr.Scrollbar.Y = max(0, offset-tr.Bounds.H)
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
			beginQueueEdit(c, v, d)
		}
		if w.ButtonText("Delete") {
			c.DeleteQueued(d.ID)
		}
		if tooltipButton(w, "↑", "Move queued message earlier") {
			for i := 1; i < len(c.Queue); i++ {
				if c.Queue[i].ID == d.ID {
					c.Queue[i], c.Queue[i-1] = c.Queue[i-1], c.Queue[i]
					break
				}
			}
		}
		if tooltipButton(w, "↓", "Move queued message later") {
			for i := 0; i+1 < len(c.Queue); i++ {
				if c.Queue[i].ID == d.ID {
					c.Queue[i], c.Queue[i+1] = c.Queue[i+1], c.Queue[i]
					break
				}
			}
		}
		if w.ButtonText("Send now") {
			if d.Status == "" {
				a.submitTurn(c, d.Text, d.Attachments, "steer", d.ID)
			}
		}
	}
	if c.QueuePaused && len(c.Queue) > 0 {
		w.Row(28).Dynamic(2)
		w.Label("Queue paused", "LC")
		if w.ButtonText("Resume queue") {
			c.QueuePaused = false
			a.dispatchQueue(c)
		}
	}
	for _, d := range c.Outbox {
		w.Row(48).Dynamic(1)
		w.LabelWrap("Send " + d.Status + ": " + cut(d.Text, 120))
		if d.Status != "pending" {
			w.Row(28).Dynamic(2)
			if w.ButtonText("Recover as queued draft") {
				c.Enqueue(d.Text, d.Attachments)
				c.QueuePaused = true
				removeOutbox(c, d.ID)
				a.toast = "Check conversation history before resending an unconfirmed message."
				break
			}
			if w.ButtonText("Discard saved send") {
				removeOutbox(c, d.ID)
				break
			}
		}
	}
	if c.EditQueue != "" {
		w.Row(28).Dynamic(2)
		if w.ButtonText("Save queued edit") {
			for i := range c.Queue {
				if c.Queue[i].ID == c.EditQueue {
					c.Queue[i].Text = text(v.Editor)
					c.Queue[i].Attachments = append([]string(nil), v.Attachments...)
				}
			}
			finishQueueEdit(c, v)
		}
		if w.ButtonText("Cancel queued edit") {
			finishQueueEdit(c, v)
		}
	}
	a.suggestions(w, c, v)
	if c.Busy() {
		v.Editor.Placeholder = "Working… type to queue or steer, Esc to stop"
	} else {
		v.Editor.Placeholder = "Ask Codex anything. @ to mention files, / for commands"
	}
	w.Row(composer).Dynamic(1)
	v.Editor.Edit(w)
	w.Row(29).Static(210, 112, 110, 82, 110, 36)
	a.modelPickers(w, &c.Model, &c.Effort, &c.Tier, c.Busy())
	if button(w, "Plan", c.Plan, a.p) {
		c.Plan = !c.Plan
	}
	if c.Busy() {
		if w.ButtonText("Stop") {
			a.rpc("turn/interrupt", map[string]any{"threadId": c.ID, "turnId": c.TurnID}, nil)
		}
	} else {
		w.Spacing(1)
	}
	if tooltipButton(w, "+", "Attach files or images") {
		a.attachDialog(v)
	}
	if warning := a.catalog.SpeedWarning(c.Model, fallback(c.Tier, "default")); warning != "" {
		muted(w, warning, a.p)
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
	w.LabelColored(contextLabel(c), "LC", a.p.Faint)
	if c.Busy() {
		if w.ButtonText("Steer") {
			a.send(c, "steer")
		}
		if primary(w, "Queue", a.p) {
			a.send(c, "queue")
		}
	} else {
		if c.EditQueue != "" {
			if w.ButtonText("Cancel edit") {
				finishQueueEdit(c, v)
			}
		} else {
			w.Spacing(1)
		}
		caption := "Send ↑"
		if c.EditQueue != "" {
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
	if a.infoSection(w, "conversation", "Conversation", c.Title) {
		w.Row(28).Dynamic(1)
		if w.ButtonText("Rename…") {
			a.inputDialog("Rename conversation", c.Title, func(name string) {
				a.rpc("thread/name/set", map[string]any{"threadId": c.ID, "name": name}, func(_ json.RawMessage) {
					a.invalidateSidebar(c.ID)
					c.Title = name
					for i := range a.state.Tabs {
						if a.state.Tabs[i].Kind == workspace.Chat && a.state.Tabs[i].Target == c.ID {
							a.state.Tabs[i].Title = name
						}
					}
				})
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
			a.rpc(archiveMethod, map[string]any{"threadId": c.ID}, func(_ json.RawMessage) { a.invalidateSidebar(c.ID); c.Archived = !c.Archived; a.closeChatTabs(c.ID) })
		}
		w.Row(28).Dynamic(1)
		if w.ButtonText("Export Markdown…") {
			a.exportChat(c)
		}
	}
	if c.Cwd != "" && a.infoSection(w, "project", "Project", c.Cwd) {
		w.Row(56).Dynamic(1)
		w.LabelWrap(c.Cwd)
		w.Row(28).Dynamic(2)
		if w.ButtonText("Open folder") {
			a.openPath(c.Cwd, false)
		}
		if w.ButtonText("Changes") {
			a.showDiff(c)
		}
	}
	a.drawInfoSummary(w, c)
	a.extraInfo(w, c)
	if len(a.state.Tabs) > 1 && a.infoSection(w, "tabs", "Document tabs", "Move tab") {
		w.Row(28).Dynamic(2)
		if w.ButtonText("← Move") {
			a.state.Move(tab.ID, -1)
		}
		if w.ButtonText("Move →") {
			a.state.Move(tab.ID, 1)
		}
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

func removeOutbox(c *workspace.Conversation, id string) {
	for i := range c.Outbox {
		if c.Outbox[i].ID == id {
			c.Outbox = append(c.Outbox[:i], c.Outbox[i+1:]...)
			return
		}
	}
}
