//go:build fltk_headless

package ui

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type recapCall struct {
	Method string
	Params map[string]any
}

func TestRecapRPCFixture(t *testing.T) {
	if len(os.Args) == 0 || os.Args[len(os.Args)-1] != "fastrock-recap-fixture" {
		return
	}
	scanner, writer := bufio.NewScanner(os.Stdin), json.NewEncoder(os.Stdout)
	var calls []recapCall
	mode := ""
	for scanner.Scan() {
		var request codex.Message
		if json.Unmarshal(scanner.Bytes(), &request) != nil || request.Method == "" {
			continue
		}
		p := codex.Decode(request.Params)
		calls = append(calls, recapCall{request.Method, p})
		var result any = map[string]any{}
		switch request.Method {
		case "fixture/calls":
			result = calls
		case "config/read":
			result = map[string]any{"config": map[string]any{"mcp_servers": map[string]any{"calendar": map[string]any{}, "files": map[string]any{}}}}
		case "thread/start":
			mode = str(p, "cwd")
			if mode == "late-thread" {
				time.Sleep(500 * time.Millisecond)
			}
			sandbox := "read-only"
			if mode == "unsafe" {
				sandbox = "workspace-write"
			}
			result = map[string]any{"thread": map[string]any{"id": "temporary"}, "sandbox": map[string]any{"type": sandbox}}
		case "turn/start":
			if mode == "late-turn" {
				time.Sleep(500 * time.Millisecond)
			}
			if mode == "normal" {
				// Real servers can emit items and completion before the RPC reply.
				_ = writer.Encode(map[string]any{"method": "item/completed", "params": map[string]any{"threadId": "temporary", "turnId": "turn", "item": map[string]any{"type": "agentMessage", "text": `{"summary":"Implemented the change; validation remains.","next_action":"Run the tests."}`}}})
				_ = writer.Encode(map[string]any{"method": "turn/completed", "params": map[string]any{"threadId": "temporary", "turn": map[string]any{"id": "turn", "status": "completed"}}})
			}
			result = map[string]any{"turn": map[string]any{"id": "turn"}}
		}
		encoded, _ := json.Marshal(result)
		_ = writer.Encode(codex.Message{ID: request.ID, Result: encoded})
	}
	os.Exit(0)
}

func recapFixture(t *testing.T) *App {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client, err := codex.StartCommand(ctx, executable, []string{"-test.run=^TestRecapRPCFixture$", "--", "fastrock-recap-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	a := transferFixture()
	a.ctx, a.client, a.updates = ctx, client, make(chan func(), 128)
	go func() {
		for m := range client.Events {
			m.Origin = client
			a.post(func() { a.event(m) })
		}
	}()
	// Finish subprocess startup before short timeout tests begin.
	ready, stop := context.WithTimeout(ctx, 3*time.Second)
	defer stop()
	if err := client.Call(ready, "fixture/calls", nil, nil); err != nil {
		t.Fatal(err)
	}
	return a
}

func recapCalls(t *testing.T, a *App) []recapCall {
	t.Helper()
	ctx, cancel := context.WithTimeout(a.ctx, 3*time.Second)
	defer cancel()
	var calls []recapCall
	if err := a.client.Call(ctx, "fixture/calls", nil, &calls); err != nil {
		t.Fatal(err)
	}
	return calls
}

func TestV40RecapHistoryAndSchema(t *testing.T) {
	var blocks []workspace.Block
	for i := 0; i < 12; i++ {
		blocks = append(blocks, workspace.Block{Role: "you", Text: fmt.Sprintf("question-%02d", i)}, workspace.Block{Role: "tool", Text: "tool secret must be omitted"}, workspace.Block{Role: "assistant", Text: fmt.Sprintf("answer-%02d", i)})
	}
	history := recapHistory(blocks)
	if strings.Contains(history, "question-03") || strings.Contains(history, "tool secret") || !strings.Contains(history, "question-04") || !strings.Contains(history, "answer-11") || strings.Count(history, "User:") != 8 {
		t.Fatal(history)
	}
	blocks[len(blocks)-1].Text = "opening " + strings.Repeat("日本語", 20000) + " closing"
	blocks = append(blocks, workspace.Block{Role: "you", Text: "Latest correction: do not deploy"})
	history = recapHistory(blocks)
	if len(history) > recapHistoryBytes || !utf8.ValidString(history) || !strings.Contains(history, "opening") || !strings.Contains(history, "closing") || !strings.Contains(history, "Latest correction: do not deploy") {
		t.Fatal("unbounded or incomplete history", len(history))
	}
	if recapHistory([]workspace.Block{{Role: "assistant", Text: "orphan answer"}}) != "" {
		t.Fatal("orphan assistant answer treated as conversation")
	}
	for _, invalid := range []string{`{}`, `{"summary":null,"next_action":null}`, `{"summary":"ok"}`, `{"summary":"ok","next_action":null,"extra":true}`, `{"summary":"","next_action":null}`, `{"summary":"ok","next_action":7}`, `{"summary":"` + strings.Repeat("x", 701) + `","next_action":null}`, strings.Repeat("x", recapAnswerBytes+1)} {
		if _, err := parseRecap(invalid); err == nil {
			t.Fatal("accepted invalid recap", invalid[:min(60, len(invalid))])
		}
	}
	valid, err := parseRecap(`{"summary":" Done. ","next_action":"   "}`)
	if err != nil || valid.Summary != "Done." || valid.Next != nil {
		t.Fatal(valid, err)
	}
}

func TestV40RecapIsolatedToolFreeAndRoutedBeforeStartReply(t *testing.T) {
	a := recapFixture(t)
	c := &workspace.Conversation{ID: "parent", Cwd: "normal", Model: "model", Status: "running", TurnID: "main-turn", Settings: workspace.ThreadSettings{Provider: "provider"}, Blocks: []workspace.Block{{Role: "you", Text: "Build a thing"}, {Role: "assistant", Text: "Implemented."}}}
	a.state.Chats[c.ID] = c
	a.state.Open(workspace.Chat, "Parent", c.ID, "")
	a.startRecap(c)
	drain(t, a, func() bool { return len(a.recaps) == 0 })
	if len(c.Blocks) != 3 || c.Blocks[2].Kind != "recap" || !strings.Contains(c.Blocks[2].Text, "**Next:** Run the tests.") || c.Status != "running" || c.TurnID != "main-turn" || len(c.Outbox) != 0 || len(a.state.Tabs) != 1 || len(a.state.Chats) != 1 {
		t.Fatal("recap changed the main turn or did not render its card", c, a.state.Tabs)
	}
	calls := recapCalls(t, a)
	var methods []string
	for _, call := range calls {
		if call.Method == "fixture/calls" {
			continue
		}
		methods = append(methods, call.Method)
		switch call.Method {
		case "thread/start":
			p := call.Params
			if p["ephemeral"] != true || p["sandbox"] != "read-only" || p["model"] != "model" || p["modelProvider"] != "provider" || p["threadSource"] != "system" {
				t.Fatal(p)
			}
			config := p["config"].(map[string]any)
			for _, key := range []string{"features.shell_tool", "features.hooks", "features.multi_agent", "features.apps", "features.plugins", "skills.include_instructions", "tools.update_plan.enabled"} {
				if config[key] != false {
					t.Fatal("tool not disabled", key, config)
				}
			}
			servers := config["mcp_servers"].(map[string]any)
			if servers["calendar"].(map[string]any)["enabled"] != false || servers["files"].(map[string]any)["enabled"] != false || !reflect.DeepEqual(p["dynamicTools"], []any{}) {
				t.Fatal(p)
			}
		case "turn/start":
			if call.Params["threadId"] != "temporary" || call.Params["outputSchema"] == nil {
				t.Fatal("turn not isolated or structured", call)
			}
		}
	}
	if !reflect.DeepEqual(methods, []string{"config/read", "thread/start", "turn/start", "thread/unsubscribe"}) {
		t.Fatal(methods)
	}
}

func TestV40RecapCleansUnsafeAndLateStarts(t *testing.T) {
	for _, mode := range []string{"unsafe", "late-thread", "late-turn"} {
		t.Run(mode, func(t *testing.T) {
			a := recapFixture(t)
			r := &recapRun{ctx: a.ctx, client: a.client, events: make(chan map[string]any, 32)}
			_, err := generateRecap(r, a.ctx, make(chan struct{}, 4), &workspace.Conversation{Cwd: mode}, "User: hi", 100*time.Millisecond, func(string) bool { return true })
			if err == nil {
				t.Fatal("unsafe or late request succeeded")
			}
			// The next fixture request waits behind a delayed start; cleanup
			// may be delivered immediately after it, so wait for its receipt.
			deadline := time.Now().Add(3 * time.Second)
			for {
				calls := recapCalls(t, a)
				unsubscribed, interrupted, ran := false, false, false
				for _, call := range calls {
					unsubscribed = unsubscribed || call.Method == "thread/unsubscribe"
					interrupted = interrupted || call.Method == "turn/interrupt"
					ran = ran || call.Method == "turn/start"
				}
				if unsubscribed && (mode != "late-turn" || interrupted) {
					if mode == "unsafe" && ran {
						t.Fatal("unsafe thread ran inference")
					}
					break
				}
				if time.Now().After(deadline) {
					t.Fatal("temporary work leaked", calls)
				}
				time.Sleep(time.Millisecond)
			}
		})
	}
}

func TestV40ClosingRecapCancelsAndIgnoresStaleResult(t *testing.T) {
	a := recapFixture(t)
	c := &workspace.Conversation{ID: "parent", Cwd: "waiting", Blocks: []workspace.Block{{Role: "you", Text: "Build"}}}
	a.state.Chats[c.ID] = c
	tab := a.state.Open(workspace.Chat, "Parent", c.ID, "")
	a.startRecap(c)
	drain(t, a, func() bool { return a.recaps[c.ID].threadID == "temporary" })
	// Wait for turn/start to be acknowledged, then close the source tab.
	deadline := time.Now().Add(3 * time.Second)
	for {
		found := false
		for _, call := range recapCalls(t, a) {
			found = found || call.Method == "turn/start"
		}
		if found {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("turn not started")
		}
		time.Sleep(time.Millisecond)
	}
	a.closeTabNow(tab)
	if len(a.recaps) != 0 {
		t.Fatal("closed tab retained recap")
	}
	drain(t, a, func() bool {
		for _, call := range recapCalls(t, a) {
			if call.Method == "turn/interrupt" && call.Params["threadId"] == "temporary" {
				return true
			}
		}
		return false
	})
	if len(c.Blocks) > 1 || len(a.state.Tabs) != 0 {
		t.Fatal("closed recap appended a stale card", c.Blocks)
	}
}

func TestV40RecapEventsAreBoundedAndScoped(t *testing.T) {
	a := transferFixture()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	r := &recapRun{ctx: ctx, cancel: cancel, client: &codex.Client{}, threadID: "temporary", events: make(chan map[string]any, 4)}
	a.recaps = map[string]*recapRun{"parent": r}
	p := map[string]any{"threadId": "temporary", "turnId": "turn", "item": map[string]any{"type": "agentMessage", "text": strings.Repeat("x", recapAnswerBytes+1)}}
	message := codex.Message{Method: "item/completed", Origin: &codex.Client{}}
	if a.recapEvent(message, p) {
		t.Fatal("routed a different client's event")
	}
	message.Origin = r.client
	if !a.recapEvent(message, p) {
		t.Fatal("temporary event leaked to main transcript")
	}
	event := <-r.events
	if str(event, "method") != "recap/tooLarge" || len(event) != 1 {
		t.Fatal("retained oversized response", event)
	}
	p = map[string]any{"threadId": "temporary", "turn": map[string]any{"id": "turn", "status": "completed", "items": strings.Repeat("x", 100000)}}
	message.Method = "turn/completed"
	a.recapEvent(message, p)
	encoded, _ := json.Marshal(<-r.events)
	if len(encoded) > 100 {
		t.Fatal("retained irrelevant completion payload", len(encoded))
	}
}
