package codex

import (
	"context"
	"encoding/json"
	"sync"
	"testing"
	"time"
)

func TestBrokerMultiplexingAndTransfer(t *testing.T) {
	server := helper(t)
	b, err := NewBroker(server)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if bad, err := Dial(ctx, b.Address(), "wrong"); err == nil {
		bad.Close()
		t.Fatal("accepted unauthenticated window")
	}
	first, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	second, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()
	var wg sync.WaitGroup
	for _, c := range []*Client{first, second} {
		wg.Go(func() {
			for range 8 {
				var result struct{ Method string }
				if err := c.Call(ctx, "echo", nil, &result); err != nil || result.Method != "echo" {
					t.Errorf("multiplex: %s %v", result.Method, err)
				}
			}
		})
	}
	wg.Wait()
	var offer struct{ Ticket string }
	if err := first.Call(ctx, "fastrock/offer", map[string]any{"data": map[string]string{"draft": "unsent 🚀"}}, &offer); err != nil {
		t.Fatal(err)
	}
	var payload map[string]string
	if err := second.Call(ctx, "fastrock/claim", map[string]string{"ticket": offer.Ticket}, &payload); err != nil || payload["draft"] != "unsent 🚀" {
		t.Fatalf("transfer: %v %v", payload, err)
	}
	if err := second.Call(ctx, "fastrock/ready", map[string]string{"ticket": offer.Ticket}, nil); err != nil {
		t.Fatal(err)
	}
	select {
	case event := <-first.Events:
		if event.Method != "fastrock/tabRefresh" {
			t.Fatal(event.Method)
		}
	case <-ctx.Done():
		t.Fatal("source snapshot was not requested")
	}
	if err := first.Call(ctx, "fastrock/finalize", map[string]any{"ticket": offer.Ticket, "data": payload}, nil); err != nil {
		t.Fatal(err)
	}
	select {
	case event := <-second.Events:
		if event.Method != "fastrock/finalized" {
			t.Fatal(event.Method)
		}
	case <-ctx.Done():
		t.Fatal("latest snapshot was not delivered")
	}
	if err := second.Call(ctx, "fastrock/applied", map[string]string{"ticket": offer.Ticket}, nil); err != nil {
		t.Fatal(err)
	}
	select {
	case event := <-first.Events:
		if event.Method != "fastrock/tabClaimed" {
			t.Fatal(event.Method)
		}
	case <-ctx.Done():
		t.Fatal("source not notified")
	}
	if err := second.Call(ctx, "fastrock/claim", map[string]string{"ticket": offer.Ticket}, &payload); err == nil {
		t.Fatal("transfer replay allowed")
	}
	if err := second.Call(ctx, "fastrock/own", map[string]string{"threadId": "test"}, nil); err != nil {
		t.Fatal(err)
	}
	b.mu.Lock()
	if len(b.peers) != 2 || len(b.tickets) != 0 {
		t.Fatal("wrong window lifecycle")
	}
	b.mu.Unlock()
	first.Close()
	var echo json.RawMessage
	if err := second.Call(ctx, "echo", nil, &echo); err != nil {
		t.Fatal("closing one window stopped another", err)
	}
	second.Close()
	select {
	case <-b.done:
	case <-ctx.Done():
		t.Fatal("server leaked after last window closed")
	}
}

func TestBrokerRestartPreservesWindows(t *testing.T) {
	b, err := NewBroker(helper(t))
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	b.SetStarter(func() (*Client, error) { return helper(t), nil })
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	if err = c.Call(ctx, "fastrock/restart", map[string]any{}, nil); err != nil {
		t.Fatal(err)
	}
	for _, expected := range []string{"fastrock/serverStopped", "fastrock/serverReady"} {
		select {
		case m := <-c.Events:
			if m.Method != expected {
				t.Fatalf("%s != %s", m.Method, expected)
			}
		case <-ctx.Done():
			t.Fatal("restart event missing")
		}
	}
	var result struct{ Method string }
	if err = c.Call(ctx, "echo", nil, &result); err != nil || result.Method != "echo" {
		t.Fatalf("%s %v", result.Method, err)
	}
}

func TestBrokerCancellationReturnsSourceOwnership(t *testing.T) {
	b, e := NewBroker(helper(t))
	if e != nil {
		t.Fatal(e)
	}
	defer b.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	source, e := Dial(ctx, b.Address(), b.Token())
	if e != nil {
		t.Fatal(e)
	}
	defer source.Close()
	dest, e := Dial(ctx, b.Address(), b.Token())
	if e != nil {
		t.Fatal(e)
	}
	defer dest.Close()
	var offer struct{ Ticket string }
	if e = source.Call(ctx, "fastrock/offer", map[string]any{"data": map[string]string{"draft": "unsaved"}}, &offer); e != nil {
		t.Fatal(e)
	}
	if e = dest.Call(ctx, "fastrock/claim", map[string]string{"ticket": offer.Ticket}, nil); e != nil {
		t.Fatal(e)
	}
	if e = source.Call(ctx, "fastrock/cancel", map[string]string{"ticket": offer.Ticket}, nil); e != nil {
		t.Fatal(e)
	}
	select {
	case m := <-dest.Events:
		if m.Method != "fastrock/tabCancelled" {
			t.Fatal(m.Method)
		}
	case <-ctx.Done():
		t.Fatal("destination did not receive cancellation")
	}
	if e = dest.Call(ctx, "fastrock/applied", map[string]string{"ticket": offer.Ticket}, nil); e == nil {
		t.Fatal("cancelled ticket committed")
	}
}

func TestBrokerSharesOnlyOpenChatMetadataAndRoutesDelivery(t *testing.T) {
	b, err := NewBroker(helper(t))
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	a, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	c, err := Dial(ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	rows := []OpenThread{{ID: "second-chat", Title: "Other window", AcceptsMessages: true}}
	if err = c.Call(ctx, "fastrock/publishThreads", map[string]any{"data": rows}, nil); err != nil {
		t.Fatal(err)
	}
	var got []OpenThread
	if err = a.Call(ctx, "fastrock/threads", nil, &got); err != nil || len(got) != 1 || got[0].ID != "second-chat" {
		t.Fatalf("registry: %+v %v", got, err)
	}
	if err = a.Call(ctx, "fastrock/delivery", map[string]any{"threadId": "second-chat", "data": map[string]string{"Text": "approved message"}}, nil); err != nil {
		t.Fatal(err)
	}
	select {
	case event := <-c.Events:
		if event.Method != "fastrock/delivery" {
			t.Fatal(event.Method)
		}
	case <-ctx.Done():
		t.Fatal("delivery missed recipient")
	}
	if err = c.Call(ctx, "fastrock/publishThreads", map[string]any{"data": []OpenThread{}}, nil); err != nil {
		t.Fatal(err)
	}
	if err = a.Call(ctx, "fastrock/threads", nil, &got); err != nil || len(got) != 0 {
		t.Fatalf("closed chat remained in registry: %v %v", got, err)
	}
}
