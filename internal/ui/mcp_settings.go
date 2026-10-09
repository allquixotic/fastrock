package ui

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"github.com/aarzilli/nucular"
)

type mcpForm struct {
	Open                                           bool
	Transport                                      int
	Name, Command, Args, Env, URL, Bearer, Headers *nucular.TextEditor
}

func newMCPForm() *mcpForm {
	return &mcpForm{Name: textEditor("", false), Command: textEditor("", false), Args: textEditor("[]", false), Env: textEditor("{}", true), URL: textEditor("https://", false), Bearer: textEditor("", false), Headers: textEditor("{}", true)}
}
func mcpConfig(f *mcpForm) (map[string]any, error) {
	if strings.TrimSpace(text(f.Name)) == "" {
		return nil, fmt.Errorf("enter a server name")
	}
	value := map[string]any{}
	if f.Transport == 0 {
		if strings.TrimSpace(text(f.Command)) == "" {
			return nil, fmt.Errorf("enter a command")
		}
		var args []string
		if err := json.Unmarshal([]byte(text(f.Args)), &args); err != nil {
			return nil, fmt.Errorf("arguments must be a JSON string array")
		}
		var env map[string]string
		if err := json.Unmarshal([]byte(text(f.Env)), &env); err != nil {
			return nil, fmt.Errorf("environment must be a JSON object with string values")
		}
		value["command"] = text(f.Command)
		value["args"] = args
		if len(env) > 0 {
			value["env"] = env
		}
	} else {
		u, err := url.Parse(strings.TrimSpace(text(f.URL)))
		if err != nil || u.Host == "" || u.User != nil || (u.Scheme != "https" && u.Scheme != "http") {
			return nil, fmt.Errorf("enter an HTTP(S) URL without embedded credentials")
		}
		value["url"] = u.String()
		if token := strings.TrimSpace(text(f.Bearer)); token != "" {
			value["bearer_token_env_var"] = token
		}
		var headers map[string]string
		if err := json.Unmarshal([]byte(text(f.Headers)), &headers); err != nil {
			return nil, fmt.Errorf("headers must be a JSON object with string values")
		}
		if len(headers) > 0 {
			value["http_headers"] = headers
		}
	}
	return value, nil
}
func (a *App) drawMCPSettings(w *nucular.Window, s *settingsView) {
	if s.MCP == nil {
		s.MCP = newMCPForm()
	}
	f := s.MCP
	w.Row(30).Static(130, 130)
	if w.ButtonText("Add server…") {
		f.Open = true
	}
	if w.ButtonText("Reload servers") {
		a.settingsRequest("config/mcpServer/reload", map[string]any{})
	}
	if !f.Open {
		return
	}
	a.field(w, "Server name", f.Name, false)
	w.Row(30).Dynamic(1)
	f.Transport = w.ComboSimple([]string{"Command (stdio)", "Streamable HTTP"}, f.Transport, 28)
	if f.Transport == 0 {
		a.field(w, "Command", f.Command, false)
		a.field(w, "Arguments (JSON array)", f.Args, false)
		a.field(w, "Environment (JSON object)", f.Env, true)
	} else {
		a.field(w, "Server URL", f.URL, false)
		a.field(w, "Bearer token environment variable", f.Bearer, false)
		a.field(w, "HTTP headers (JSON object)", f.Headers, true)
	}
	w.Row(30).Static(120, 120)
	if primary(w, "Add server", a.p) {
		value, err := mcpConfig(f)
		if err != nil {
			a.report(err)
		} else {
			a.settingsRequest("config/value/write", map[string]any{"keyPath": "mcp_servers." + configKey(strings.TrimSpace(text(f.Name))), "value": value, "mergeStrategy": "replace"})
			f.Open = false
		}
	}
	if w.ButtonText("Cancel") {
		f.Open = false
	}
}
