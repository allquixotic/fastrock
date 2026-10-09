// Package mockmodel supplies a local Responses API fixture for end-to-end Codex
// tests. It never makes outgoing requests or loads provider credentials.
package mockmodel

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync/atomic"
	"time"
)

type Server struct {
	Requests    atomic.Int64
	ToolOutputs atomic.Int64
	Observe     func(map[string]any)
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if r.Method == "GET" && strings.HasSuffix(r.URL.Path, "/models") {
		_ = json.NewEncoder(w).Encode(map[string]any{"object": "list", "data": []any{map[string]any{"id": "mock-model", "object": "model"}}})
		return
	}
	if r.Method != "POST" || !strings.HasSuffix(r.URL.Path, "/responses") {
		http.NotFound(w, r)
		return
	}
	s.Requests.Add(1)
	var body map[string]any
	if e := json.NewDecoder(r.Body).Decode(&body); e != nil {
		w.WriteHeader(400)
		return
	}
	if s.Observe != nil {
		s.Observe(body)
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	flusher, _ := w.(http.Flusher)
	send := func(event map[string]any) {
		b, _ := json.Marshal(event)
		_, _ = fmt.Fprintf(w, "event: %s\ndata: %s\n\n", event["type"], b)
		if flusher != nil {
			flusher.Flush()
		}
	}
	id := fmt.Sprintf("response-%d", time.Now().UnixNano())
	send(map[string]any{"type": "response.created", "response": map[string]any{"id": id}})
	inputs, _ := body["input"].([]any)
	outputs := 0
	lastUser := ""
	for _, item := range inputs {
		o, _ := item.(map[string]any)
		if o["role"] == "user" {
			outputs = 0
			lastUser = ""
			if content, ok := o["content"].([]any); ok {
				for _, part := range content {
					p, _ := part.(map[string]any)
					if text, ok := p["text"].(string); ok {
						lastUser += text
					}
				}
			}
		}
		if o["type"] == "function_call_output" || o["type"] == "custom_tool_call_output" {
			outputs++
			s.ToolOutputs.Add(1)
		}
	}
	queryName, namespace := findTool(body["input"], "rally_query", "")
	if queryName == "" {
		queryName, namespace = findTool(body["tools"], "rally_query", "")
	}
	serialized, _ := json.Marshal(body)
	codeMode := queryName == "" && strings.Contains(string(serialized), "rally_query")
	if codeMode {
		queryName = "rally_query"
	}
	if queryName != "" && strings.Contains(strings.ToLower(lastUser), "blocked") && outputs < 2 {
		name := queryName
		args := map[string]any{"kind": "HierarchicalRequirement", "query": "(Blocked = true)"}
		if outputs == 1 {
			if n, ns := findTool(body["tools"], "rally_show_view", ""); n != "" {
				name = n
				namespace = ns
				args = map[string]any{"page": "teamboard", "query": "(Blocked = true)", "group": "Owner", "mode": "board"}
			}
		}
		data, _ := json.Marshal(args)
		item := map[string]any{"type": "function_call", "call_id": "call-" + id, "name": name, "arguments": string(data)}
		if namespace != "" {
			item["namespace"] = namespace
		}
		if codeMode {
			name := "rally_query"
			if outputs == 1 {
				name = "rally_show_view"
				data, _ = json.Marshal(map[string]any{"page": "teamboard", "query": "(Blocked = true)", "group": "Owner", "mode": "board"})
			}
			item = map[string]any{"type": "custom_tool_call", "call_id": "call-" + id, "name": "exec", "namespace": "functions", "input": "text(await tools." + name + "(" + string(data) + "))"}
		}
		send(map[string]any{"type": "response.output_item.done", "item": item})
	} else {
		text := "## Native Codex conversation\n\nStreaming works through the installed Codex app-server.\n\n- Conversations keep independent state.\n- Queue, steer, and stop stay available.\n\n```go\nfmt.Println(\"Hello, Fastrock\")\n```"
		if queryName != "" {
			text = "## Blocked work\n\nThe Rally query returned the blocked stories in your selected scope. I opened the native Team Board, filtered it to blocked work, and grouped it by owner. No Rally records were modified."
		}
		mid := "message-" + id
		send(map[string]any{"type": "response.output_item.added", "item": map[string]any{"id": mid, "type": "message", "role": "assistant", "content": []any{map[string]any{"type": "output_text", "text": ""}}}})
		for i := 0; i < len(text); i += 16 {
			send(map[string]any{"type": "response.output_text.delta", "item_id": mid, "delta": text[i:min(i+16, len(text))]})
			select {
			case <-r.Context().Done():
				return
			case <-time.After(15 * time.Millisecond):
			}
		}
		send(map[string]any{"type": "response.output_item.done", "item": map[string]any{"id": mid, "type": "message", "role": "assistant", "content": []any{map[string]any{"type": "output_text", "text": text}}}})
	}
	send(map[string]any{"type": "response.completed", "response": map[string]any{"id": id, "usage": map[string]any{"input_tokens": 800, "output_tokens": 80, "total_tokens": 880}}})
}
func findTool(v any, wanted, namespace string) (string, string) {
	if list, ok := v.([]any); ok {
		for _, v := range list {
			m, _ := v.(map[string]any)
			name, _ := m["name"].(string)
			if name == wanted || strings.HasSuffix(name, "."+wanted) {
				return name, namespace
			}
			if tools, ok := m["tools"]; ok {
				if found, ns := findTool(tools, wanted, name); found != "" {
					return found, ns
				}
			}
		}
	}
	if m, ok := v.(map[string]any); ok {
		for k, child := range m {
			ns := namespace
			if k == "tools" {
				if n, ok := m["name"].(string); ok {
					ns = n
				}
			}
			if name, namespace := findTool(child, wanted, ns); name != "" {
				return name, namespace
			}
		}
	}
	return "", ""
}
