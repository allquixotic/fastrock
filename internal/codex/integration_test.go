package codex_test

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/assistant"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/mockmodel"
	"github.com/allquixotic/fastrock/internal/mockrally"
	"github.com/allquixotic/fastrock/internal/rally"
)

func TestInstalledCodexRallyRoundTrip(t *testing.T) {
	if os.Getenv("FASTROCK_CODEX_INTEGRATION") != "1" {
		t.Skip("set FASTROCK_CODEX_INTEGRATION=1 with Codex 0.162+ on PATH; uses loopback mocks only")
	}
	model := &mockmodel.Server{}
	modelHTTP := httptest.NewServer(model)
	defer modelHTTP.Close()
	rallyHTTP := httptest.NewServer(mockrally.New())
	defer rallyHTTP.Close()
	rc, e := rally.New(rallyHTTP.URL, "mock-token", nil)
	if e != nil {
		t.Fatal(e)
	}
	home := t.TempDir()
	t.Setenv("CODEX_HOME", home)
	config := fmt.Sprintf("model = \"gpt-6.1-sol\"\nmodel_provider = \"mock\"\napproval_policy = \"never\"\nsandbox_mode = \"read-only\"\n[model_providers.mock]\nname = \"Local test fixture\"\nbase_url = %q\nwire_api = \"responses\"\nrequires_openai_auth = false\nrequest_max_retries = 0\nstream_max_retries = 0\n", modelHTTP.URL+"/v1")
	if e = os.WriteFile(filepath.Join(home, "config.toml"), []byte(config), 0600); e != nil {
		t.Fatal(e)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	c, e := codex.Start(ctx)
	if e != nil {
		t.Fatal(e)
	}
	defer c.Close()
	catalog, e := c.Catalog(ctx, home)
	if e != nil || catalog.DefaultModel() != "gpt-6.1-sol" {
		t.Fatalf("catalog: %v %v", catalog.DefaultModel(), e)
	}
	var started struct {
		Thread struct {
			ID string `json:"id"`
		} `json:"thread"`
	}
	if e = c.Call(ctx, "thread/start", map[string]any{"cwd": home, "ephemeral": true, "dynamicTools": assistant.Specs(), "developerInstructions": assistant.Instructions}, &started); e != nil {
		t.Fatal(e)
	}
	if started.Thread.ID == "" {
		t.Fatal("no thread ID")
	}
	if e = c.Call(ctx, "turn/start", map[string]any{"threadId": started.Thread.ID, "input": codex.TextInput("Show blocked stories and group by owner")}, nil); e != nil {
		t.Fatal(e)
	}
	shown := false
	toolCount := 0
	delta := ""
	tools := assistant.Tools{Client: rc, Show: func(v assistant.View) error { shown = v.Page == "teamboard"; return nil }}
	for {
		select {
		case m := <-c.Events:
			switch m.Method {
			case "item/tool/call":
				p := codex.Decode(m.Params)
				args, _ := json.Marshal(p["arguments"])
				result, e := tools.Execute(ctx, p["tool"].(string), args)
				if e != nil {
					t.Fatal(e)
				}
				b, _ := json.Marshal(result)
				if e = c.Respond(m.ID, map[string]any{"success": true, "contentItems": []any{map[string]any{"type": "inputText", "text": string(b)}}}); e != nil {
					t.Fatal(e)
				}
				toolCount++
			case "item/agentMessage/delta":
				p := codex.Decode(m.Params)
				s, _ := p["delta"].(string)
				delta += s
			case "error":
				t.Fatalf("Codex: %s", m.Params)
			case "turn/completed":
				if !shown || toolCount != 2 || delta == "" {
					t.Fatalf("tool round trip incomplete: shown=%v calls=%d text=%q; event=%s", shown, toolCount, delta, m.Params)
				}
				return
			}
		case <-ctx.Done():
			t.Fatalf("timed out, calls=%d model requests=%d", toolCount, model.Requests.Load())
		}
	}
}
