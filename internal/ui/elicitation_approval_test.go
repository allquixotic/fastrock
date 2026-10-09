//go:build nucular_headless

package ui

import (
	"encoding/json"
	"image"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

func TestV38MCPToolChoicesUseAdvertisedMetadata(t *testing.T) {
	p := map[string]any{"mode": "form", "serverName": "Docs", "message": "Find pages", "requestedSchema": map[string]any{"properties": map[string]any{}}, "_meta": map[string]any{"codex_approval_kind": "mcp_tool_call", "tool_name": "search", "tool_title": "Search pages", "persist": []any{"session", "always"}, "tool_params_display": []any{map[string]any{"name": "query", "display_name": "Search", "value": "my query"}}}}
	r := approval{Params: p}
	configureElicitation(&r)
	if r.Title != "Allow Docs to run Search pages?" || !strings.Contains(r.Details, "Search: my query") || len(r.Choices) != 4 {
		t.Fatal(r)
	}
	for i, want := range []string{"Allow", "Allow for this session", "Always allow", "Cancel"} {
		if r.Choices[i].Title != want {
			t.Fatal(r.Choices)
		}
	}
	want := elicitationResponse("accept", map[string]any{"persist": "always"})
	if !reflect.DeepEqual(r.Choices[2].Result, want) || !reflect.DeepEqual(r.Choices[approvalCancelIndex(r.Choices)].Result, elicitationResponse("cancel", nil)) {
		t.Fatal(r.Choices)
	}
	meta := p["_meta"].(map[string]any)
	delete(meta, "persist")
	configureElicitation(&r)
	if len(r.Choices) != 2 {
		t.Fatal("unadvertised persistence allowed", r.Choices)
	}
}
func TestV38MCPParameterPreviewIsBoundedAndRedacted(t *testing.T) {
	params := map[string]any{"z": 9, "f": 6, "e": 5, "d": 4, "c": 3, "b": 2, "a": 1}
	rows := elicitationParams(map[string]any{"tool_params": params})
	if len(rows) != 6 || rows[0][0] != "a" || rows[5][0] != "f" {
		t.Fatal(rows)
	}
	rows = elicitationParams(map[string]any{"tool_params_display": []any{map[string]any{"name": "api_key", "display_name": "Key", "value": "top-secret"}, map[string]any{"name": "text", "value": strings.Repeat("x", 200)}}})
	if rows[0][1] != "[redacted]" || len([]rune(rows[1][1])) > 121 || params["a"] != 1 {
		t.Fatal(rows)
	}
}
func TestV38MCPFormsSuggestionsAndUnsafeLinks(t *testing.T) {
	form := approval{Params: map[string]any{"mode": "form", "serverName": "Docs", "requestedSchema": map[string]any{"properties": map[string]any{"name": map[string]any{"type": "string"}}}, "_meta": map[string]any{"persist": "always"}}}
	configureElicitation(&form)
	if len(form.Choices) != 0 || form.Title != "Docs needs some information" {
		t.Fatal(form)
	}
	suggestion := approval{Params: map[string]any{"mode": "form", "serverName": "Tools", "_meta": map[string]any{"codex_approval_kind": "tool_suggestion", "tool_name": "Calendar", "suggest_type": "install", "suggest_reason": "Needed for the requested meeting", "install_url": "https://example.test/install"}}}
	configureElicitation(&suggestion)
	if suggestion.Title != "Install Calendar?" || len(suggestion.Choices) != 2 || suggestion.Choices[0].OpenURL != "https://example.test/install" || !strings.Contains(suggestion.Details, "requested meeting") {
		t.Fatal(suggestion)
	}
	for _, target := range []string{"file:///tmp/tool", "javascript:alert(1)", "https://user:secret@example.test", "https://example.test/\nother"} {
		if elicitationLink(target) {
			t.Fatal("unsafe suggestion link accepted", target)
		}
	}
	url := approval{Params: map[string]any{"mode": "url", "message": "Finish authorization", "url": "https://example.test/login"}}
	configureElicitation(&url)
	if url.Title != "Finish authorization" || url.URL != "https://example.test/login" || len(url.Choices) != 0 {
		t.Fatal("ordinary URL authorization must retain separate open and submit", url)
	}
}
func TestV38MCPEscapeCancelsAndSessionChoiceReplies(t *testing.T) {
	for _, form := range []bool{false, true} {
		t.Run(map[bool]string{false: "tool", true: "form"}[form], func(t *testing.T) {
			a := infoFixture(t)
			c := &workspace.Conversation{ID: "thread"}
			a.state.Chats[c.ID] = c
			a.state.Open(workspace.Chat, "Thread", c.ID, "")
			a.chats[c.ID] = newChatView()
			properties := map[string]any{}
			if form {
				properties["name"] = map[string]any{"type": "string"}
			}
			p := map[string]any{"threadId": c.ID, "mode": "form", "serverName": "Docs", "message": "Use search", "requestedSchema": map[string]any{"properties": properties}, "_meta": map[string]any{"codex_approval_kind": "mcp_tool_call", "tool_name": "search", "persist": "session"}}
			a.serverRequest(codex.Message{ID: json.RawMessage(`"elicitation"`), Method: "mcpServer/elicitation/request", Origin: a.client}, p)
			if len(a.approvals) != 1 {
				t.Fatal(a.approvals)
			}
			a.approvals[0].Armed = time.Time{}
			h := nucular.NewHeadlessHarness(0, image.Pt(500, 700), func(w *nucular.Window) {
				for event := range w.Input().Keyboard.Events() {
					a.approvalKey(event)
				}
			})
			a.window = h.Master()
			a.window.SetStyle(makeStyle(a.p, 13))
			h.Key(key.CodeEscape, 0)
			h.Frame(false)
			drain(t, a, func() bool { return len(a.approvals) == 0 })
			var answers []map[string]any
			if err := a.client.Call(a.ctx, "fixture/answers", nil, &answers); err != nil {
				t.Fatal(err)
			}
			if len(answers) != 1 || answers[0]["action"] != "cancel" {
				t.Fatal(answers)
			}
		})
	}
	a := infoFixture(t)
	c := &workspace.Conversation{ID: "thread"}
	a.state.Chats[c.ID] = c
	a.state.Open(workspace.Chat, "Thread", c.ID, "")
	p := map[string]any{"threadId": c.ID, "mode": "form", "serverName": "Docs", "_meta": map[string]any{"codex_approval_kind": "mcp_tool_call", "persist": []any{"session", "always"}}}
	a.serverRequest(codex.Message{ID: json.RawMessage(`"persist"`), Method: "mcpServer/elicitation/request", Origin: a.client}, p)
	a.answerApproval(a.approvals[0].Choices[1].Result)
	drain(t, a, func() bool { return len(a.approvals) == 0 })
	var answers []map[string]any
	if err := a.client.Call(a.ctx, "fixture/answers", nil, &answers); err != nil {
		t.Fatal(err)
	}
	if len(answers) != 1 || answers[0]["action"] != "accept" || answers[0]["_meta"].(map[string]any)["persist"] != "session" {
		t.Fatal(answers)
	}
}

func TestV38MCPApprovalCardUsesMetadata(t *testing.T) {
	a := infoFixture(t)
	c := &workspace.Conversation{ID: "t"}
	a.state.Chats[c.ID] = c
	a.state.Open(workspace.Chat, "T", c.ID, "")
	p := map[string]any{"threadId": c.ID, "mode": "form", "serverName": "Docs", "_meta": map[string]any{"codex_approval_kind": "mcp_tool_call", "tool_name": "search", "persist": []any{"session", "always"}}}
	a.serverRequest(codex.Message{ID: json.RawMessage(`"card"`), Method: "mcpServer/elicitation/request", Origin: a.client}, p)
	var labels []string
	h := nucular.NewHeadlessHarness(0, image.Pt(600, 1000), func(w *nucular.Window) {
		a.drawApprovalFor(w, c.ID)
		for _, cmd := range w.Commands().Commands {
			if cmd.Kind == command.TextCmd {
				labels = append(labels, cmd.Text.String)
			}
		}
	})
	a.window = h.Master()
	a.window.SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	label := strings.Join(labels, "\n")
	for _, want := range []string{"Allow Docs to run search?", "Allow for this session", "Always allow", "Cancel"} {
		if !strings.Contains(label, want) {
			t.Fatal(label, want)
		}
	}
	if strings.Contains(label, "Allow / Submit") {
		t.Fatal("tool call still presented as empty form")
	}
	a.rejectClosingApprovals(c.ID)
	var result []map[string]any
	until := time.Now().Add(3 * time.Second)
	for time.Now().Before(until) {
		if err := a.client.Call(a.ctx, "fixture/answers", nil, &result); err != nil {
			t.Fatal(err)
		}
		if len(result) > 0 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if len(result) != 1 || result[0]["action"] != "cancel" {
		t.Fatal(result)
	}
}
