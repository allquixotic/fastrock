package codex

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestVersionCompatibility(t *testing.T) {
	for _, s := range []string{"codex-cli 0.162.0", "codex-cli 0.163.0-alpha.2", "1.0.0"} {
		if e := CheckVersion(s); e != nil {
			t.Error(e)
		}
	}
	for _, s := range []string{"codex-cli 0.161.9", "not codex"} {
		if e := CheckVersion(s); e == nil {
			t.Fatalf("accepted %s", s)
		}
	}
}
func TestCatalogUsesAdvertisedBedrockTiers(t *testing.T) {
	c := Catalog{IndependentSpeedModes: true, Features: map[string]bool{"fast_mode": false}, Requirements: map[string]bool{}}
	m := Model{Model: "us.openai.gpt-6.1-sol", Tiers: []Tier{{ID: "priority", Name: "Fast"}, {ID: "ultrafast", Name: "Ultrafast"}}}
	c.Models = []Model{m}
	tiers := c.Speeds(m)
	if len(tiers) != 2 || tiers[1].ID != "ultrafast" {
		t.Fatalf("%v", tiers)
	}
	c.Requirements["ultrafast_mode"] = false
	if len(c.Speeds(m)) != 1 {
		t.Fatal("ignored managed policy")
	}
	delete(c.Requirements, "ultrafast_mode")
	m.Tiers = nil
	c.Models = []Model{m}
	if len(c.Speeds(m)) != 1 || c.SpeedWarning(m.Model, "ultrafast") == "" {
		t.Fatal("invented premium support")
	}
	if c.SpeedWarning(m.Model, "default") != "" {
		t.Fatal("default rejected")
	}
}
func TestStandardExplicitAndLegacyCatalog(t *testing.T) {
	c := Catalog{}
	m := Model{AdditionalTiers: []string{"ultrafast", "flex"}}
	if x := c.Speeds(m); len(x) != 3 || x[0].ID != "default" {
		t.Fatal(x)
	}
	c.Config = map[string]any{"model": "configured-model"}
	if c.DefaultModel() != "configured-model" {
		t.Fatal("overrode Codex configuration")
	}
}
func TestHelperProcess(t *testing.T) {
	if os.Getenv("FASTROCK_RPC_HELPER") != "1" {
		return
	}
	scanner := bufio.NewScanner(os.Stdin)
	writer := json.NewEncoder(os.Stdout)
	var mu sync.Mutex
	send := func(v any) { mu.Lock(); defer mu.Unlock(); _ = writer.Encode(v) }
	var wg sync.WaitGroup
	for scanner.Scan() {
		var m Message
		if json.Unmarshal(scanner.Bytes(), &m) != nil {
			os.Exit(2)
		}
		if len(m.ID) == 0 {
			continue
		}
		if m.Method == "initialize" {
			switch os.Getenv("FASTROCK_RPC_INIT_FAILURE") {
			case "sqlite":
				send(map[string]any{"id": m.ID, "error": RPCError{Code: -32603, Message: "failed to initialize sqlite state runtime: database is locked"}})
				continue
			case "eof":
				os.Exit(1)
			}
		}
		if m.Method == "never" {
			continue
		}
		if m.Method == "exit" {
			os.Exit(0)
		}
		if m.Method == "events" {
			send(map[string]any{"method": "item/agentMessage/delta", "params": map[string]any{"delta": "hello"}})
			send(map[string]any{"id": "server-request", "method": "item/tool/requestUserInput", "params": map[string]any{}})
		}
		message := m
		wg.Add(1)
		go func() {
			defer wg.Done()
			time.Sleep(2 * time.Millisecond)
			send(map[string]any{"id": json.RawMessage(message.ID), "result": map[string]any{"method": message.Method}})
		}()
	}
	wg.Wait()
	os.Exit(0)
}
func helper(t *testing.T) *Client {
	t.Helper()
	t.Setenv("FASTROCK_RPC_HELPER", "1")
	c, e := StartCommand(context.Background(), os.Args[0], []string{"-test.run=^TestHelperProcess$"})
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(c.Close)
	return c
}

func TestV75InitializationFailurePreservesCause(t *testing.T) {
	for _, mode := range []string{"sqlite", "eof"} {
		t.Run(mode, func(t *testing.T) {
			t.Setenv("FASTROCK_RPC_INIT_FAILURE", mode)
			c := helper(t)
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			err := c.initialize(ctx)
			if err == nil || strings.Contains(err.Error(), "incompatible") || strings.Contains(err.Error(), "update Codex") {
				t.Fatal("startup failure blamed the accepted CLI version", err)
			}
			var rpc *RPCError
			if !errors.As(err, &rpc) {
				t.Fatal("lost underlying RPC failure", err)
			}
			if mode == "sqlite" && (rpc.Code != -32603 || !strings.Contains(rpc.Message, "database is locked")) || mode == "eof" && (rpc.Code != CodeDisconnected || !strings.Contains(rpc.Message, "EOF")) {
				t.Fatal("lost original startup cause", err)
			}
			select {
			case <-c.done:
			default:
				t.Fatal("failed initialization retained a live process")
			}
		})
	}
}
func TestConcurrentRPCNotificationsAndServerRequest(t *testing.T) {
	c := helper(t)
	var wg sync.WaitGroup
	for i := range 32 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			method := fmt.Sprintf("echo-%d", i)
			var result map[string]any
			if e := c.Call(context.Background(), method, map[string]any{}, &result); e != nil || result["method"] != method {
				t.Errorf("wrong response: %v %v", result, e)
			}
		}()
	}
	wg.Wait()
	var result map[string]any
	if e := c.Call(context.Background(), "events", nil, &result); e != nil {
		t.Fatal(e)
	}
	for range 2 {
		select {
		case event := <-c.Events:
			if len(event.ID) > 0 {
				if e := c.Respond(event.ID, map[string]any{"answers": map[string]any{}}); e != nil {
					t.Fatal(e)
				}
			}
		case <-time.After(time.Second):
			t.Fatal("lost notification")
		}
	}
}
func TestRPCCancelAndDisconnect(t *testing.T) {
	c := helper(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Millisecond)
	defer cancel()
	if e := c.Call(ctx, "never", nil, nil); e == nil {
		t.Fatal("no timeout")
	}
	c.mu.Lock()
	n := len(c.pending)
	c.mu.Unlock()
	if n != 0 {
		t.Fatalf("leaked %d requests", n)
	}
	ctx2, cancel2 := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel2()
	if e := c.Call(ctx2, "exit", nil, nil); e == nil || !strings.Contains(e.Error(), "stopped") && !strings.Contains(e.Error(), "disconnected") {
		t.Fatalf("bad disconnect: %v", e)
	}
}
