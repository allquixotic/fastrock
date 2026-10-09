//go:build fltk_headless

package ui

import (
	"bufio"
	"context"
	"encoding/json"
	"image"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestInfoRPCFixture(t *testing.T) {
	if len(os.Args) == 0 || os.Args[len(os.Args)-1] != "fastrock-info-fixture" {
		return
	}
	scanner := bufio.NewScanner(os.Stdin)
	writer := json.NewEncoder(os.Stdout)
	var calls []string
	lists := 0
	cleaned := false
	responses := 0
	var answers []json.RawMessage
	for scanner.Scan() {
		var request codex.Message
		if json.Unmarshal(scanner.Bytes(), &request) != nil {
			continue
		}
		if request.Method == "" {
			responses++
			answers = append(answers, request.Result)
			continue
		}
		params := codex.Decode(request.Params)
		var result any = map[string]any{}
		switch request.Method {
		case "fixture/serverRequest":
			var event codex.Message
			_ = json.Unmarshal(request.Params, &event)
			_ = writer.Encode(event)
		case "thread/queue/add", "thread/queue/start":
			calls = append(calls, request.Method)
		case "thread/items/list":
			calls = append(calls, request.Method)
			result = map[string]any{"data": []any{map[string]any{"item": map[string]any{"id": "patch", "type": "fileChange", "changes": []any{map[string]any{"path": "/work/new.go", "kind": map[string]any{"type": "add"}, "diff": "package main\n"}}}}}}
		case "thread/backgroundTerminals/list":
			calls = append(calls, request.Method)
			lists++
			if str(params, "threadId") == "cycle" {
				result = map[string]any{"data": []any{}, "nextCursor": "same"}
				break
			}
			var data []any
			if lists > 1 && !cleaned {
				data = append(data, map[string]any{"processId": "p1", "command": "go test ./...", "cwd": "/work/project", "osPid": 42, "cpuPercent": 12.5, "rssKb": 2048})
			}
			result = map[string]any{"data": data}
		case "thread/backgroundTerminals/clean":
			calls = append(calls, request.Method)
			cleaned = true
		case "thread/backgroundTerminals/terminate":
			calls = append(calls, request.Method)
		case "fixture/calls":
			result = calls
		case "fixture/responses":
			result = responses
		case "fixture/answers":
			result = answers
		}
		encoded, _ := json.Marshal(result)
		_ = writer.Encode(codex.Message{ID: request.ID, Result: encoded})
	}
	os.Exit(0)
}
func infoFixture(t *testing.T) *App {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client, err := codex.StartCommand(ctx, executable, []string{"-test.run=^TestInfoRPCFixture$", "--", "fastrock-info-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	a := transferFixture()
	a.ctx, a.client, a.updates = ctx, client, make(chan func(), 128)
	a.p = colors(false)
	return a
}
func TestV35TerminalDiscoveryCleanAndReconnect(t *testing.T) {
	a := infoFixture(t)
	c := &workspace.Conversation{ID: "thread"}
	v := &conversationInfo{}
	a.state.Chats[c.ID] = c
	a.infoViews = map[string]*conversationInfo{c.ID: v}
	a.loadTerminals(c, v)
	// An event racing the first (empty) snapshot must trigger another request.
	a.infoEvent(c, "item/started", map[string]any{"item": map[string]any{"type": "commandExecution", "processId": "p1"}})
	drain(t, a, func() bool { return !v.TerminalsLoading && len(v.Terminals) == 1 })
	if terminalDetail(v.Terminals[0]) != "PID 42 · CPU 12.5% · Memory 2.0 MiB" {
		t.Fatal(v.Terminals)
	}
	a.cleanTerminals(c, v)
	drain(t, a, func() bool { return !v.StoppingAll && !v.TerminalsLoading && len(v.Terminals) == 0 })
	var calls []string
	if err := a.client.Call(a.ctx, "fixture/calls", nil, &calls); err != nil {
		t.Fatal(err)
	}
	want := []string{"thread/backgroundTerminals/list", "thread/backgroundTerminals/list", "thread/backgroundTerminals/clean", "thread/backgroundTerminals/list"}
	if !reflect.DeepEqual(calls, want) {
		t.Fatal(calls)
	}
	a.loadTerminals(c, v)
	select {
	case publish := <-a.updates:
		a.resetInfoConnection()
		v.TerminalNote = "new connection"
		publish()
	case <-time.After(3 * time.Second):
		t.Fatal("missing pending read")
	}
	if v.TerminalNote != "new connection" || v.TerminalsLoading {
		t.Fatal("old result survived reset", v)
	}
}
func TestV35TerminalPagingAndPresentation(t *testing.T) {
	a := infoFixture(t)
	if _, err := terminalList(a.ctx, a.client, "cycle"); err == nil || !strings.Contains(err.Error(), "repeated") {
		t.Fatal(err)
	}
	root := t.TempDir()
	project := filepath.Join(root, "project")
	for _, tc := range []struct{ path, want string }{{project, "."}, {filepath.Join(project, "src"), "." + string(filepath.Separator) + "src"}, {filepath.Join(root, "elsewhere"), "~" + string(filepath.Separator) + "elsewhere"}} {
		if got := terminalLocation(tc.path, project, root); got != tc.want {
			t.Fatal(got, tc.want)
		}
	}
	if terminalEvent("item/started", map[string]any{"item": map[string]any{"type": "agentMessage"}}) || !terminalEvent("item/commandExecution/terminalInteraction", nil) {
		t.Fatal("wrong refresh events")
	}
	message := terminalStopMessage([]backgroundTerminal{{Command: "go test ./...\nsecret second line"}, {Command: "sleep 30"}})
	if !strings.Contains(message, "go test ./...") || strings.Contains(message, "second line") || !strings.Contains(message, "sleep 30") {
		t.Fatal(message)
	}
}
func TestV35EffectiveSummaryAndLiveSettings(t *testing.T) {
	a := infoFixture(t)
	c := &workspace.Conversation{Model: "old", SideParentID: "parent", SideParentTitle: "Parent"}
	p := map[string]any{"model": "new", "effort": "high", "modelProvider": "ollama", "approvalPolicy": "on-request", "approvalsReviewer": "auto_review", "sandboxPolicy": map[string]any{"type": "workspace-write", "network_access": true}}
	a.infoEvent(c, "thread/settings/updated", map[string]any{"threadSettings": p})
	rows := conversationSummary(c)
	s := fmtRows(rows)
	for _, want := range []string{"new", "high", "ollama", "On request · auto-review", "Workspace write + network", "Parent"} {
		if !strings.Contains(s, want) {
			t.Fatal(s, want)
		}
	}
	if approvalSummary(workspace.ThreadSettings{Approval: "never", Reviewer: "auto_review"}) != "Never ask" {
		t.Fatal("unnecessary review label")
	}
	cp := a.checkpointConversation(c)
	c.Settings.Provider = "changed"
	if a.checkpointConversation(c) == cp || cp.Settings.Provider != "ollama" {
		t.Fatal("settings lost checkpoint isolation")
	}
}
func fmtRows(rows [][2]string) string {
	var b strings.Builder
	for _, row := range rows {
		b.WriteString(row[0] + ": " + row[1] + "\n")
	}
	return b.String()
}
func TestV35AgentRowsPreserveDetails(t *testing.T) {
	c := &workspace.Conversation{ID: "parent"}
	upsertAgent(c, workspace.Agent{ID: "child", Name: "Ada", Role: "reviewer", Status: "running"})
	old := append([]workspace.Agent(nil), c.Agents...)
	updateAgents(c, map[string]any{"type": "subAgentActivity", "agentThreadId": "child", "agentPath": "root/review", "kind": "completed"})
	updateAgents(c, map[string]any{"type": "collabAgentToolCall", "agentsStates": map[string]any{"child": map[string]any{"status": "errored", "message": "Build failed\nmore"}}})
	if c.Agents[0].Name != "Ada" || agentDetail(c.Agents[0]) != "reviewer · root/review · Build failed" || agentStatusLabel(c.Agents[0].Status) != "Failed" || old[0].Status != "running" {
		t.Fatal(c.Agents, old)
	}
	updateAgents(c, map[string]any{"type": "collabAgentToolCall", "tool": "spawnAgent", "receiverThreadIds": []any{"new"}})
	if len(c.Agents) != 2 || c.Agents[1].Status != "running" {
		t.Fatal(c.Agents)
	}
}
func TestV35MailboxConsentDeclinesOnce(t *testing.T) {
	a := infoFixture(t)
	c := &workspace.Conversation{ID: "target", Title: "Target"}
	a.state.Chats[c.ID] = c
	pending := a.beginConsent(codex.Message{ID: json.RawMessage(`1`)}, "sender", c.ID)
	a.transferEvent(codex.Message{Method: "fastrock/messagingDisabled", Params: json.RawMessage(`{"threadIds":["target"]}`)})
	a.setMessagesAllowed(c, false)
	if !pending.Resolved || len(a.deliveryConsents) != 0 || a.finishConsent(pending) {
		t.Fatal("pending delivery survived opt out")
	}
	// Wait until the background response reaches the local server. There is no delivery RPC.
	ctx, cancel := context.WithTimeout(a.ctx, 3*time.Second)
	defer cancel()
	for {
		var n int
		if err := a.client.Call(ctx, "fixture/responses", nil, &n); err != nil {
			t.Fatal(err)
		}
		if n == 1 {
			break
		}
		select {
		case <-ctx.Done():
			t.Fatal("missing decline response")
		case <-time.After(time.Millisecond):
		}
	}
	a.recordMail(c.ID, mailMessage{From: "sender", To: c.ID, Text: "hello", At: 1})
	direction, id, name := a.mailPeer(c, a.mailbox[c.ID][0])
	if direction != "From" || id != "sender" || name != "sender" {
		t.Fatal(direction, id, name)
	}
	for i := 0; i < 60; i++ {
		a.recordMail(c.ID, mailMessage{From: c.ID, To: "sender"})
	}
	if len(a.mailbox[c.ID]) != 50 {
		t.Fatal("unbounded mailbox")
	}
	direction, id, _ = a.mailPeer(c, a.mailbox[c.ID][0])
	if direction != "To" || id != "sender" {
		t.Fatal(direction, id)
	}
}
func TestV35InfoSectionsShareStateAndHideEmpty(t *testing.T) {
	a := infoFixture(t)
	a.infoCollapsed = map[string]bool{"session": true}
	var text []string
	c := &workspace.Conversation{ID: "a", Title: "A", Model: "hidden-model", Status: "idle"}
	a.state.Chats[c.ID] = c
	a.state.Open(workspace.Chat, c.Title, c.ID, "")
	a.infoViews = map[string]*conversationInfo{c.ID: {Loaded: true, TerminalsAvailable: true}}
	h := desktop.NewHeadlessHarness(0, image.Pt(420, 1400), func(w *desktop.Window) {
		a.drawInfo(w)
		for _, cmd := range w.Commands().Commands {
			if cmd.Kind == command.TextCmd {
				text = append(text, cmd.Text.String)
			}
		}
	})
	a.window = h.Master()
	a.window.SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	joined := strings.Join(text, "\n")
	if strings.Contains(joined, "hidden-model") || strings.Contains(joined, "Sub-agents") || strings.Contains(joined, "Background terminals") || !strings.Contains(joined, "Session") {
		t.Fatal(joined)
	}
	c2 := &workspace.Conversation{ID: "b", Title: "B", Model: "other-hidden", Status: "idle"}
	a.state.Chats[c2.ID] = c2
	a.state.Open(workspace.Chat, c2.Title, c2.ID, "")
	a.infoViews[c2.ID] = &conversationInfo{Loaded: true, TerminalsAvailable: true}
	text = nil
	h.Frame(false)
	if strings.Contains(strings.Join(text, "\n"), "other-hidden") {
		t.Fatal("collapse state changed with tab")
	}
}
