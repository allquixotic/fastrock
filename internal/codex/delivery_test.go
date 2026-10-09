package codex

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestDeliveryRPCProcess(t *testing.T) {
	if os.Args[len(os.Args)-1] != "fastrock-delivery-fixture" {
		return
	}
	scanner := bufio.NewScanner(os.Stdin)
	w := json.NewEncoder(os.Stdout)
	var queued []json.RawMessage
	for scanner.Scan() {
		var m Message
		if json.Unmarshal(scanner.Bytes(), &m) != nil {
			continue
		}
		var result any = map[string]any{}
		switch m.Method {
		case "thread/queue/add":
			queued = append(queued, m.Params)
		case "fixture/queued":
			result = queued
		case "fixture/emit":
			var event Message
			_ = json.Unmarshal(m.Params, &event)
			_ = w.Encode(event)
		}
		if len(m.ID) > 0 {
			_ = w.Encode(Message{ID: m.ID, Result: raw(result)})
		}
	}
	os.Exit(0)
}

type deliveryFixture struct {
	b                      *Broker
	source, target, server *Client
	ctx                    context.Context
	from, to               OpenThread
}

func newDeliveryFixture(t *testing.T) *deliveryFixture {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	t.Cleanup(cancel)
	server, err := StartCommand(ctx, os.Args[0], []string{"-test.run=^TestDeliveryRPCProcess$", "--", "fastrock-delivery-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	b, err := NewBroker(server)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(b.Close)
	source, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(source.Close)
	target, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(target.Close)
	scope := PermissionScope{Approval: "on-request", Sandbox: "workspace-write", Roots: "[]"}
	cwd := t.TempDir()
	f := &deliveryFixture{b: b, source: source, target: target, server: server, ctx: ctx, from: OpenThread{ID: "a", Title: "Review", Cwd: cwd, Status: "running", AcceptsMessages: true, Permissions: scope}, to: OpenThread{ID: "b", Title: "Build", Cwd: cwd, Status: "running", AcceptsMessages: true, Permissions: scope}}
	f.publish(t)
	return f
}
func (f *deliveryFixture) publish(t *testing.T) {
	t.Helper()
	for _, pair := range []struct {
		c   *Client
		row OpenThread
	}{{f.source, f.from}, {f.target, f.to}} {
		if err := pair.c.Call(f.ctx, "fastrock/publishThreads", map[string]any{"data": []OpenThread{pair.row}}, nil); err != nil {
			t.Fatal(err)
		}
	}
}
func (f *deliveryFixture) request(id, turn string) error {
	return f.source.Call(f.ctx, "fastrock/requestDelivery", map[string]any{"data": DeliveryStart{ID: id, From: "a", Target: "b", Text: "run `go test` <literally>", TurnID: turn, Wait: true}}, nil)
}
func deliveryNext(t *testing.T, ctx context.Context, c *Client, method string, out any) {
	t.Helper()
	for {
		select {
		case m := <-c.Events:
			if m.Method == method {
				if out != nil {
					if err := json.Unmarshal(m.Params, out); err != nil {
						t.Fatal(err)
					}
				}
				return
			}
		case <-ctx.Done():
			t.Fatalf("missing %s: %v", method, ctx.Err())
		}
	}
}
func (f *deliveryFixture) queues(t *testing.T) []json.RawMessage {
	t.Helper()
	var q []json.RawMessage
	if err := f.source.Call(f.ctx, "fixture/queued", nil, &q); err != nil {
		t.Fatal(err)
	}
	return q
}
func (f *deliveryFixture) answer(t *testing.T, r DeliveryRequest, choice string) {
	t.Helper()
	if err := f.target.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": DeliveryAnswer{r.ID, choice, r.Revision}}, nil); err != nil {
		t.Fatal(err)
	}
}
func TestV42DeliveryPermissionScopes(t *testing.T) {
	base := t.TempDir()
	cwd, shared := filepath.Join(base, "repo"), filepath.Join(base, "shared")
	roots := func(path string) string { return string(raw([]string{path})) }
	same := OpenThread{Cwd: cwd, Permissions: PermissionScope{Approval: "on-request", Sandbox: "workspace-write", Roots: roots(shared)}}
	for _, tc := range []struct {
		name     string
		from, to OpenThread
		want     string
	}{
		{"same", same, same, ""},
		{"subfolder", same, OpenThread{Cwd: filepath.Join(cwd, "sub"), Permissions: PermissionScope{Approval: "untrusted", Sandbox: "workspace-write", Roots: roots(filepath.Join(shared, "cache"))}}, ""},
		{"outside", same, OpenThread{Cwd: filepath.Join(base, "other"), Permissions: same.Permissions}, "can write"},
		{"sibling prefix", same, OpenThread{Cwd: cwd + "-other", Permissions: same.Permissions}, "can write"},
		{"relative", same, OpenThread{Cwd: "relative", Permissions: same.Permissions}, "can write"},
		{"unknown", same, OpenThread{}, "not known"},
		{"full", same, OpenThread{Permissions: PermissionScope{Approval: "never", Sandbox: "danger-full-access"}}, "broader filesystem"},
		{"network", OpenThread{Permissions: PermissionScope{Approval: "on-request", Sandbox: "read-only"}}, OpenThread{Permissions: PermissionScope{Approval: "on-request", Sandbox: "read-only", Network: true}}, "network"},
		{"approval", OpenThread{Permissions: PermissionScope{Approval: "untrusted", Sandbox: "read-only"}}, OpenThread{Permissions: PermissionScope{Approval: "never", Sandbox: "read-only"}}, "less often"},
		{"malformed roots", same, OpenThread{Cwd: cwd, Permissions: PermissionScope{Approval: "on-request", Sandbox: "workspace-write", Roots: "oops"}}, "not known"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := DeliveryEscalation(tc.from, tc.to)
			if tc.want == "" && got != "" || tc.want != "" && !strings.Contains(got, tc.want) {
				t.Fatal(got)
			}
		})
	}
}
func TestV42TargetConsentRoutingAndReplay(t *testing.T) {
	f := newDeliveryFixture(t)
	if err := f.request("d1", "t1"); err != nil {
		t.Fatal(err)
	}
	var r DeliveryRequest
	deliveryNext(t, f.ctx, f.source, "fastrock/deliveryPending", nil)
	deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", &r)
	if r.To.ID != "b" || r.From.Title != "Review" || r.AlwaysAsk != "" || len(f.queues(t)) != 0 {
		t.Fatal(r)
	}
	answer := DeliveryAnswer{r.ID, "deliver", r.Revision}
	if err := f.source.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": answer}, nil); err == nil {
		t.Fatal("sender approved its own delivery")
	}
	answer.Revision++
	if err := f.target.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": answer}, nil); err == nil {
		t.Fatal("accepted stale revision")
	}
	f.answer(t, r, "deliver")
	var result DeliveryResult
	deliveryNext(t, f.ctx, f.source, "fastrock/deliveryResult", &result)
	deliveryNext(t, f.ctx, f.target, "fastrock/deliveryResolved", nil)
	if !result.Success || !strings.Contains(result.Wrapped, "&lt;literally&gt;") || !strings.Contains(result.Wrapped, "<delivery_id>d1</delivery_id>") {
		t.Fatal(result)
	}
	if len(f.queues(t)) != 1 {
		t.Fatal("delivery not queued exactly once")
	}
	if err := f.target.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": DeliveryAnswer{r.ID, "deliver", r.Revision}}, nil); err == nil {
		t.Fatal("replayed delivery")
	}
	if len(f.queues(t)) != 1 {
		t.Fatal("duplicate queue")
	}
}
func TestV42SessionGrantsRevalidateAndOptOut(t *testing.T) {
	f := newDeliveryFixture(t)
	if err := f.request("d1", "t1"); err != nil {
		t.Fatal(err)
	}
	var r DeliveryRequest
	deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", &r)
	f.answer(t, r, "session")
	deliveryNext(t, f.ctx, f.source, "fastrock/deliveryResult", nil)
	if err := f.request("d2", "t1"); err != nil {
		t.Fatal(err)
	}
	var result DeliveryResult
	deliveryNext(t, f.ctx, f.source, "fastrock/deliveryResult", &result)
	if !result.Success || len(f.queues(t)) != 2 {
		t.Fatal("session grant did not deliver", result)
	}
	f.to.Permissions = PermissionScope{Approval: "never", Sandbox: "danger-full-access"}
	f.publish(t)
	if err := f.request("d3", "t1"); err != nil {
		t.Fatal(err)
	}
	deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", &r)
	if r.AlwaysAsk == "" {
		t.Fatal("escalation offered session allowance")
	}
	if err := f.target.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": DeliveryAnswer{r.ID, "session", r.Revision}}, nil); err == nil {
		t.Fatal("escalating session grant")
	}
	f.to.Permissions = PermissionScope{Approval: "on-request", Sandbox: "read-only"}
	f.publish(t)
	var changed DeliveryRequest
	deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", &changed)
	if changed.Revision <= r.Revision || changed.AlwaysAsk != "" {
		t.Fatal(changed)
	}
	if err := f.target.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": DeliveryAnswer{r.ID, "deliver", r.Revision}}, nil); err == nil {
		t.Fatal("old permission revision accepted")
	}
	f.to.AcceptsMessages = false
	f.publish(t)
	deliveryNext(t, f.ctx, f.source, "fastrock/deliveryResult", &result)
	if result.Success || len(f.queues(t)) != 2 {
		t.Fatal("opt-out still delivered", result)
	}
	if err := f.request("d4", "t2"); err == nil {
		t.Fatal("disabled target accepted")
	}
}
func TestV42DeliveryLimitsHopsAndExpiry(t *testing.T) {
	f := newDeliveryFixture(t)
	for i := 0; i < 5; i++ {
		if err := f.request(fmt.Sprint("limit", i), "t1"); err != nil {
			t.Fatal(err)
		}
	}
	if err := f.request("sixth", "t1"); err == nil {
		t.Fatal("sixth send accepted")
	}
	for i := 5; i < 10; i++ {
		if err := f.request(fmt.Sprint("limit", i), "t2"); err != nil {
			t.Fatal(err)
		}
	}
	if err := f.request("eleventh", "t3"); err == nil {
		t.Fatal("eleventh pending message accepted")
	}
	f.b.expireDelivery("limit0")
	if err := f.request("replacement", "t3"); err != nil {
		t.Fatal("expiry did not release target capacity", err)
	}
	f.b.clearDeliveries("reset fixture")
	f.b.mu.Lock()
	f.b.deliveryHops["a"] = deliveryHop{Turn: "chain", Hop: 3, Expires: time.Now().Add(time.Hour)}
	f.b.mu.Unlock()
	if err := f.request("hop4", "chain"); err != nil {
		t.Fatal(err)
	}
	var r DeliveryRequest
	for r.ID != "hop4" {
		deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", &r)
	}
	if r.Hop != 4 || !strings.Contains(r.AlwaysAsk, "hop 4") {
		t.Fatal(r)
	}
	f.answer(t, r, "deliver")
	var result DeliveryResult
	for result.ID != "hop4" {
		deliveryNext(t, f.ctx, f.source, "fastrock/deliveryResult", &result)
	}
	event := Message{Method: "item/started", Params: raw(map[string]any{"threadId": "b", "turnId": "target-turn", "item": map[string]any{"type": "userMessage", "content": []any{map[string]any{"text": result.Wrapped}}}})}
	if err := f.source.Call(f.ctx, "fixture/emit", event, nil); err != nil {
		t.Fatal(err)
	}
	deliveryNext(t, f.ctx, f.target, "item/started", nil)
	f.b.mu.Lock()
	hop := f.b.deliveryHops["b"]
	_, retained := f.b.deliveries["hop4"]
	f.b.mu.Unlock()
	if hop.Hop != 4 || hop.Turn != "target-turn" || retained {
		t.Fatal(hop, retained)
	}
}

func TestV42SourceTerminationRevokesConsent(t *testing.T) {
	for _, mode := range []string{"turn", "request", "closed"} {
		t.Run(mode, func(t *testing.T) {
			f := newDeliveryFixture(t)
			in := DeliveryStart{ID: "cancel", From: "a", Target: "b", Text: "must not run", TurnID: "source-turn", CallID: `"source-call"`}
			if err := f.source.Call(f.ctx, "fastrock/requestDelivery", map[string]any{"data": in}, nil); err != nil {
				t.Fatal(err)
			}
			var r DeliveryRequest
			deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", &r)
			if mode == "closed" {
				if err := f.source.Call(f.ctx, "fastrock/publishThreads", map[string]any{"data": []OpenThread{}}, nil); err != nil {
					t.Fatal(err)
				}
			} else {
				event := Message{Method: "turn/completed", Params: raw(map[string]any{"threadId": "a", "turn": map[string]string{"id": "source-turn"}})}
				if mode == "request" {
					event = Message{Method: "serverRequest/resolved", Params: raw(map[string]any{"threadId": "a", "requestId": "source-call"})}
				}
				if err := f.source.Call(f.ctx, "fixture/emit", event, nil); err != nil {
					t.Fatal(err)
				}
			}
			var result DeliveryResult
			deliveryNext(t, f.ctx, f.source, "fastrock/deliveryResult", &result)
			if result.Success || len(f.queues(t)) != 0 {
				t.Fatal("ended source delivered", result)
			}
			if err := f.target.Call(f.ctx, "fastrock/answerDelivery", map[string]any{"data": DeliveryAnswer{r.ID, "deliver", r.Revision}}, nil); err == nil {
				t.Fatal("revoked consent accepted")
			}
		})
	}
}

func TestV42ServerStopClearsPendingWithoutDeadlock(t *testing.T) {
	f := newDeliveryFixture(t)
	if err := f.request("stopped", "source-turn"); err != nil {
		t.Fatal(err)
	}
	deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", nil)
	f.server.Close()
	ctx, cancel := context.WithTimeout(f.ctx, time.Second)
	defer cancel()
	var result DeliveryResult
	deliveryNext(t, ctx, f.source, "fastrock/deliveryResult", &result)
	if result.Success || !strings.Contains(result.Error, "stopped") {
		t.Fatal(result)
	}
	deliveryNext(t, ctx, f.source, "fastrock/serverStopped", nil)
	if err := f.request("after-stop", "later-turn"); err == nil {
		t.Fatal("accepted a message after server shutdown")
	}
}

func TestV42DeliveryCardsNeverFollowResolution(t *testing.T) {
	f := newDeliveryFixture(t)
	for i := range 50 {
		id := fmt.Sprintf("ordering-%d", i)
		if err := f.request(id, id); err != nil {
			t.Fatal(err)
		}
		deliveryNext(t, f.ctx, f.target, "fastrock/deliveryRequest", nil)
		row := f.to
		row.Cwd = filepath.Join(f.from.Cwd, fmt.Sprintf("changed-%d", i))
		var wg sync.WaitGroup
		errors := make(chan error, 2)
		wg.Go(func() {
			errors <- f.target.Call(f.ctx, "fastrock/publishThreads", map[string]any{"data": []OpenThread{row}}, nil)
		})
		wg.Go(func() {
			errors <- f.source.Call(f.ctx, "fastrock/cancelDelivery", map[string]any{"data": map[string]string{"ID": id}}, nil)
		})
		wg.Wait()
		for range 2 {
			if err := <-errors; err != nil {
				t.Fatal(err)
			}
		}
		f.to = row
		// The reply on this connection follows every previously enqueued event.
		if err := f.target.Call(f.ctx, "fastrock/threads", nil, nil); err != nil {
			t.Fatal(err)
		}
		resolved := false
		for draining := true; draining; {
			select {
			case m := <-f.target.Events:
				if m.Method == "fastrock/deliveryResolved" {
					resolved = true
				}
				if m.Method == "fastrock/deliveryRequest" && resolved {
					t.Fatal("a cancelled card was republished")
				}
			default:
				draining = false
			}
		}
		if !resolved {
			t.Fatal("missing cancellation result")
		}
	}
}
