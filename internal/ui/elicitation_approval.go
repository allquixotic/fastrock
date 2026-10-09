package ui

import (
	"encoding/json"
	"net/url"
	"sort"
	"strings"
)

func elicitationResponse(action string, meta any) map[string]any {
	return map[string]any{"action": action, "content": nil, "_meta": meta}
}
func elicitationLink(target string) bool {
	u, err := url.Parse(target)
	return err == nil && u.Host != "" && u.User == nil && (u.Scheme == "https" || u.Scheme == "http") && !strings.ContainsAny(target, "\r\n")
}
func supportsElicitationPersist(meta map[string]any, mode string) bool {
	switch value := meta["persist"].(type) {
	case string:
		return value == mode
	case []any:
		for _, choice := range value {
			if choice == mode {
				return true
			}
		}
	}
	return false
}
func elicitationParamValue(name string, value any) string {
	// Work on a copy, preserving the request's actual parameters for the server.
	raw, err := json.Marshal(map[string]any{name: value})
	if err != nil {
		return "[unavailable]"
	}
	var copied map[string]any
	if json.Unmarshal(raw, &copied) != nil {
		return "[unavailable]"
	}
	scrub(copied)
	value = copied[name]
	s, ok := value.(string)
	if !ok {
		encoded, _ := json.Marshal(value)
		s = string(encoded)
	}
	return cut(strings.Join(strings.Fields(s), " "), 120)
}
func elicitationParams(meta map[string]any) [][2]string {
	var rows [][2]string
	if display, ok := meta["tool_params_display"].([]any); ok {
		for _, entry := range display {
			param, _ := entry.(map[string]any)
			name := strings.TrimSpace(str(param, "name"))
			label := strings.TrimSpace(fallback(str(param, "display_name"), name))
			value, present := param["value"]
			if name == "" || label == "" || !present {
				continue
			}
			rows = append(rows, [2]string{cut(label, 80), elicitationParamValue(name, value)})
			if len(rows) == 6 {
				return rows
			}
		}
	}
	if len(rows) > 0 {
		return rows
	}
	params, _ := meta["tool_params"].(map[string]any)
	// Keep the first six sorted names without allocating all raw parameter keys.
	var names []string
	for name := range params {
		i := sort.SearchStrings(names, name)
		if i >= 6 {
			continue
		}
		names = append(names, "")
		copy(names[i+1:], names[i:])
		names[i] = name
		if len(names) > 6 {
			names = names[:6]
		}
	}
	for _, name := range names {
		rows = append(rows, [2]string{cut(name, 80), elicitationParamValue(name, params[name])})
	}
	return rows
}
func configureElicitation(r *approval) {
	defer func() {
		r.Title = cut(r.Title, 160)
		r.Details = cut(strings.TrimSpace(r.Details), 32000)
	}()
	p := r.Params
	server := fallback(str(p, "serverName"), "MCP server")
	mode := str(p, "mode")
	r.Elicitation = true
	message := strings.TrimSpace(str(p, "message"))
	if mode == "url" {
		// Opening the page does not establish that authorization completed.
		// Preserve this mode's separate Open and Submit actions.
		r.Title, r.URL = message, str(p, "url")
		return
	}
	r.Title = server + " needs some information"
	r.Details = message
	if mode == "form" {
		schema, _ := p["requestedSchema"].(map[string]any)
		props, _ := schema["properties"].(map[string]any)
		if len(props) > 0 {
			return
		}
		meta, _ := p["_meta"].(map[string]any)
		kind := str(meta, "codex_approval_kind")
		tool := strings.TrimSpace(fallback(str(meta, "tool_title"), str(meta, "tool_name")))
		if kind == "tool_suggestion" {
			link, suggestion := str(meta, "install_url"), str(meta, "suggest_type")
			if tool != "" && elicitationLink(link) && (suggestion == "install" || suggestion == "enable") {
				verb, label := "Install", "Open the install page and continue"
				if suggestion == "enable" {
					verb, label = "Enable", "Open the page and continue"
				}
				r.Title = verb + " " + tool + "?"
				r.URL = link
				if reason := strings.TrimSpace(str(meta, "suggest_reason")); reason != "" {
					r.Details = reason + "\n\n" + r.Details
				}
				r.Choices = []approvalChoice{{Title: label, Key: 'y', Result: elicitationResponse("accept", nil), OpenURL: link}, {Title: "Not now", Key: 'n', Result: elicitationResponse("decline", nil)}}
				return
			}
		}
		r.Title = "Allow this " + server + " request?"
		if kind == "mcp_tool_call" {
			r.Title = "Allow this " + server + " tool call?"
			if tool != "" {
				r.Title = "Allow " + server + " to run " + tool + "?"
			}
			for _, row := range elicitationParams(meta) {
				r.Details += "\n" + row[0] + ": " + row[1]
			}
		}
		r.Choices = []approvalChoice{{Title: "Allow", Key: 'y', Result: elicitationResponse("accept", nil)}}
		if supportsElicitationPersist(meta, "session") {
			r.Choices = append(r.Choices, approvalChoice{Title: "Allow for this session", Key: 'a', Result: elicitationResponse("accept", map[string]any{"persist": "session"})})
		}
		if supportsElicitationPersist(meta, "always") {
			r.Choices = append(r.Choices, approvalChoice{Title: "Always allow", Key: 'p', Result: elicitationResponse("accept", map[string]any{"persist": "always"})})
		}
		if kind != "mcp_tool_call" {
			r.Choices = append(r.Choices, approvalChoice{Title: "Deny", Key: 'd', Result: elicitationResponse("decline", nil)})
		}
	}
	r.Choices = append(r.Choices, approvalChoice{Title: "Cancel", Key: 'n', Result: elicitationResponse("cancel", nil)})
}
