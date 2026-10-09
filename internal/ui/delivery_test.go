//go:build nucular_headless

package ui

import (
	"context"
	"encoding/json"
	"image"
	"strings"
	"testing"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

func TestV42UIBrokerDeliveryRoundTrip(t *testing.T) {
	a := infoFixture(t)
	backend := a.client
	broker, err := codex.NewBroker(backend)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(broker.Close)
	ctx, cancel := context.WithTimeout(a.ctx, 10*time.Second)
	defer cancel()
	source, err := codex.Dial(ctx, broker.Address(), broker.Token())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(source.Close)
	target, err := codex.Dial(ctx, broker.Address(), broker.Token())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(target.Close)
	a.client = source
	b := transferFixture()
	b.ctx, b.client, b.updates = ctx, target, make(chan func(), 128)
	b.p = colors(false)
	settings := threadSettings(codex.Decode([]byte(`{"approvalPolicy":"on-request","sandboxPolicy":{"type":"read-only"}}`)))
	a.state.Chats["a"] = &workspace.Conversation{ID: "a", Title: "Review", Settings: settings}
	b.state.Chats["b"] = &workspace.Conversation{ID: "b", Title: "Build", Settings: settings}
	a.state.Open(workspace.Chat, "Review", "a", "")
	b.state.Open(workspace.Chat, "Build", "b", "")
	a.publishThreads()
	b.publishThreads()
	for {
		var rows []codex.OpenThread
		if err := source.Call(ctx, "fastrock/threads", nil, &rows); err != nil {
			t.Fatal(err)
		}
		if len(rows) == 2 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	params, _ := json.Marshal(map[string]any{"threadId": "a", "turnId": "turn-a", "namespace": "codex_gui", "tool": "send_message_to_thread", "arguments": map[string]any{"target": "b", "message": "please review this literal `code`", "wait_for_reply": false}})
	event := codex.Message{ID: json.RawMessage(`"delivery-tool"`), Method: "item/tool/call", Params: params}
	if err := source.Call(ctx, "fixture/serverRequest", event, nil); err != nil {
		t.Fatal(err)
	}
	var received codex.Message
	select {
	case received = <-source.Events:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	received.Origin = source
	if !a.crossTabTool(received, codex.Decode(received.Params)) {
		t.Fatal("dynamic delivery tool not handled")
	}
	for len(b.approvals) == 0 {
		select {
		case m := <-target.Events:
			b.transferEvent(m)
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	if len(a.approvals) != 0 || b.approvals[0].ThreadID != "b" {
		t.Fatal("consent appeared in sender window")
	}
	var calls []string
	if err := source.Call(ctx, "fixture/calls", nil, &calls); err != nil || len(calls) != 0 {
		t.Fatal("queued before human consent", calls, err)
	}
	b.answerApproval(b.approvals[0].Choices[0].Result)
	for len(a.deliveryConsents) > 0 {
		select {
		case m := <-source.Events:
			a.transferEvent(m)
		case f := <-a.updates:
			f()
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	for len(b.approvals) > 0 {
		select {
		case m := <-target.Events:
			b.transferEvent(m)
		case f := <-b.updates:
			f()
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	var answers []map[string]any
	for {
		if err := source.Call(ctx, "fixture/answers", nil, &answers); err != nil {
			t.Fatal(err)
		}
		if len(answers) > 0 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	if len(answers) != 1 || answers[0]["success"] != true || len(b.mailbox["b"]) != 1 || len(a.mailbox["a"]) != 1 {
		t.Fatal("delivery did not finish exactly once", answers, a.mailbox, b.mailbox)
	}
}

func TestV42DeliveryCardTargetsRecipientAndShowsPermissions(t *testing.T) {
	a := infoFixture(t)
	a.state.Chats["a"] = &workspace.Conversation{ID: "a", Title: "Review"}
	a.state.Chats["b"] = &workspace.Conversation{ID: "b", Title: "Build"}
	first := a.state.Open(workspace.Chat, "Review", "a", "")
	a.state.Open(workspace.Chat, "Build", "b", "")
	a.state.Active = first
	d := codex.DeliveryRequest{ID: "delivery", Revision: 1, From: codex.OpenThread{ID: "a", Title: "Review"}, To: codex.OpenThread{ID: "b", Title: "Build"}, Text: "literal `code` <html>", Wait: true, AlwaysAsk: "the target has network access"}
	raw, _ := json.Marshal(d)
	a.deliveryEvent(codex.Message{Method: "fastrock/deliveryRequest", Params: raw})
	if len(a.approvals) != 1 || a.approvals[0].ThreadID != "b" || len(a.approvals[0].Choices) != 2 {
		t.Fatal(a.approvals)
	}
	var labels []string
	h := nucular.NewHeadlessHarness(0, image.Pt(800, 1000), func(w *nucular.Window) {
		if a.approvalHeight(w, "a") != 0 {
			t.Fatal("sender received the target card")
		}
		a.drawApprovalFor(w, "b")
		for _, c := range w.Commands().Commands {
			if c.Kind == command.TextCmd {
				labels = append(labels, c.Text.String)
			}
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	h.Frame(false)
	all := strings.Join(labels, "\n")
	for _, want := range []string{"Agent in tab “Review” wants to send this message", "literal `code` <html>", "The sending agent waits", "Always asks:", "network access", "Deliver", "Decline"} {
		if !strings.Contains(all, want) {
			t.Fatal(want, all)
		}
	}
	if e := a.approvals[0].CodeEditor; e == nil || e.Flags&nucular.EditReadOnly == 0 {
		t.Fatal("delivery message is not a selectable literal")
	}
	d.Revision = 2
	d.AlwaysAsk = ""
	raw, _ = json.Marshal(d)
	a.deliveryEvent(codex.Message{Method: "fastrock/deliveryRequest", Params: raw})
	if len(a.approvals) != 1 || len(a.approvals[0].Choices) != 3 || a.approvals[0].Delivery.Revision != 2 {
		t.Fatal(a.approvals)
	}
	d.Revision = 1
	raw, _ = json.Marshal(d)
	a.deliveryEvent(codex.Message{Method: "fastrock/deliveryRequest", Params: raw})
	if a.approvals[0].Delivery.Revision != 2 {
		t.Fatal("stale card replaced revised permissions")
	}
}
func TestV42DeliveryEscapeAndResolution(t *testing.T) {
	a := infoFixture(t)
	a.state.Chats["b"] = &workspace.Conversation{ID: "b", Title: "Build"}
	a.state.Open(workspace.Chat, "Build", "b", "")
	d := codex.DeliveryRequest{ID: "delivery", Revision: 1, From: codex.OpenThread{ID: "a", Title: "Review"}, To: codex.OpenThread{ID: "b", Title: "Build"}, Text: "message"}
	a.approvals = []approval{deliveryApproval(d, a.client)}
	a.approvals[0].Armed = time.Time{}
	a.approvals[0].Focused = true
	h := nucular.NewHeadlessHarness(0, image.Pt(700, 700), func(w *nucular.Window) {
		for event := range w.Input().Keyboard.Events() {
			a.approvalKey(event)
		}
	})
	h.Key(key.CodeEscape, 0)
	h.Frame(false)
	if !a.approvals[0].Submitting {
		t.Fatal("Escape did not decline target request")
	}
	result, _ := json.Marshal(codex.DeliveryResult{ID: d.ID, Request: d, Error: "the user declined delivery"})
	a.deliveryEvent(codex.Message{Method: "fastrock/deliveryResolved", Params: result})
	if len(a.approvals) != 0 || len(a.state.Chats["b"].Blocks) != 1 {
		t.Fatal("resolved target card retained")
	}
	if !strings.Contains(a.state.Chats["b"].Blocks[0].Text, "declined a message from tab “Review”") {
		t.Fatal(a.state.Chats["b"].Blocks)
	}
}
func TestV42ReplyBeforeDeliveryAcknowledgment(t *testing.T) {
	a := infoFixture(t)
	d := a.beginConsent(codex.Message{ID: json.RawMessage(`"tool"`), Origin: a.client}, "a", "b")
	d.Timeout = time.Minute
	d.Waiter = &replyWait{Message: d.Message, Target: "b", MarkerID: d.ID, Blocks: map[string]string{}}
	a.replyWaits = []*replyWait{d.Waiter}
	a.crossTabEvent("item/started", map[string]any{"threadId": "b", "turnId": "unrelated", "item": map[string]any{"type": "userMessage", "content": []any{map[string]any{"type": "inputImage"}}}})
	if d.Waiter.Turn != "" {
		t.Fatal("empty text matched an absent delivery marker")
	}
	wrapped := codex.WrapAgentMessage("a", "Review", "question", true, d.ID)
	a.crossTabEvent("item/started", map[string]any{"threadId": "b", "turnId": "reply-turn", "item": map[string]any{"type": "userMessage", "content": []any{map[string]any{"text": wrapped}}}})
	a.crossTabEvent("item/completed", map[string]any{"threadId": "b", "turnId": "reply-turn", "item": map[string]any{"id": "answer", "type": "agentMessage", "text": "Already finished"}})
	a.crossTabEvent("turn/completed", map[string]any{"threadId": "b", "turn": map[string]any{"id": "reply-turn", "status": "completed"}})
	if !d.Waiter.Completed || d.Waiter.Ready || len(a.replyWaits) != 1 {
		t.Fatal("early reply was lost", d.Waiter)
	}
	result, _ := json.Marshal(codex.DeliveryResult{ID: d.ID, Success: true, Request: codex.DeliveryRequest{To: codex.OpenThread{ID: "b"}}})
	a.deliveryEvent(codex.Message{Method: "fastrock/deliveryResult", Params: result})
	if len(a.replyWaits) != 0 || len(a.deliveryConsents) != 0 {
		t.Fatal("finished reply remains pending")
	}
	var answers []map[string]any
	until := time.Now().Add(3 * time.Second)
	for time.Now().Before(until) {
		if err := a.client.Call(a.ctx, "fixture/answers", nil, &answers); err != nil {
			t.Fatal(err)
		}
		if len(answers) > 0 {
			break
		}
		time.Sleep(time.Millisecond)
	}
	encoded, _ := json.Marshal(answers)
	if len(answers) != 1 || !strings.Contains(string(encoded), "Already finished") {
		t.Fatal(string(encoded))
	}
}
func TestV42PublishedPermissionRootsUseWireNames(t *testing.T) {
	s := threadSettings(codex.Decode([]byte(`{"approvalPolicy":"on-request","sandboxPolicy":{"type":"workspace-write","networkAccess":true,"writableRoots":["/shared"]}}`)))
	if !s.Current || !s.Network || s.WritableRoots != `["/shared"]` || s.Sandbox != "workspace-write" {
		t.Fatal(s)
	}
	stored, _ := json.Marshal(s)
	var restored workspace.ThreadSettings
	if err := json.Unmarshal(stored, &restored); err != nil {
		t.Fatal(err)
	}
	if deliveryScope(&workspace.Conversation{Settings: restored}) != (codex.PermissionScope{}) {
		t.Fatal("persisted permissions authorized a session allowance")
	}
	for _, field := range []string{"networkAccess", "network_access"} {
		external := threadSettings(codex.Decode([]byte(`{"approvalPolicy":"on-request","sandboxPolicy":{"type":"externalSandbox","` + field + `":"enabled"}}`)))
		if !external.Network {
			t.Fatalf("external sandbox %s=enabled lost network access", field)
		}
	}
}
