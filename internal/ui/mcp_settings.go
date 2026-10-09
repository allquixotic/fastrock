package ui

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"github.com/aarzilli/nucular"
)

type mcpForm struct {
	Generation                 uint64
	Open, Busy                 bool
	Error                      string
	Transport                  int
	Name, Command, URL, Bearer *nucular.TextEditor
	Args                       []*nucular.TextEditor
	Env, Headers               []settingPair
}

func newMCPForm() *mcpForm {
	return &mcpForm{Name: textEditor("", false), Command: textEditor("", false), URL: textEditor("https://", false), Bearer: textEditor("", false)}
}
func mcpConfig(f *mcpForm) (map[string]any, error) {
	if strings.TrimSpace(text(f.Name)) == "" {
		return nil, fmt.Errorf("enter a server name")
	}
	value := map[string]any{}
	if f.Transport == 0 {
		command := strings.TrimSpace(text(f.Command))
		if command == "" {
			return nil, fmt.Errorf("enter a command")
		}
		args := make([]string, len(f.Args))
		for i, arg := range f.Args {
			args[i] = text(arg)
		}
		env, err := settingPairs(f.Env, "environment")
		if err != nil {
			return nil, err
		}
		value["command"], value["args"] = command, args
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
			if !environmentName(token) {
				return nil, fmt.Errorf("bearer token must name an environment variable")
			}
			value["bearer_token_env_var"] = token
		}
		headers, err := settingPairs(f.Headers, "headers")
		if err != nil {
			return nil, err
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
	if f.Busy {
		muted(w, "Saving server…", a.p)
		return
	}
	a.field(w, "Server name", f.Name, false)
	w.Row(30).Dynamic(1)
	f.Transport = w.ComboSimple([]string{"Command (stdio)", "Streamable HTTP"}, f.Transport, 28)
	if f.Transport == 0 {
		a.codeField(w, "Command", f.Command, false)
		a.drawStringList(w, "Arguments (one per entry)", &f.Args)
		a.drawSettingPairs(w, "Environment variables", &f.Env)
	} else {
		a.field(w, "Server URL", f.URL, false)
		a.field(w, "Bearer token environment variable", f.Bearer, false)
		a.drawSettingPairs(w, "HTTP headers", &f.Headers)
	}
	if f.Error != "" {
		a.drawSettingsError(w, f.Error)
	}
	w.Row(30).Static(120, 120)
	if primary(w, "Add server", a.p) {
		a.addMCPServer(f)
	}
	if w.ButtonText("Cancel") {
		f.Open = false
		f.Error = ""
	}
}
func (a *App) addMCPServer(f *mcpForm) {
	if f.Busy {
		return
	}
	value, err := mcpConfig(f)
	if err != nil {
		f.Error = err.Error()
		return
	}
	if a.client == nil || a.serverPaused {
		f.Error = "Codex is not running. Reconnect, then retry."
		return
	}
	f.Generation++
	operation := f.Generation
	client, generation := a.client, a.serverGeneration
	current := func() bool {
		return operation == f.Generation && client == a.client && generation == a.serverGeneration
	}
	fail := func(err error) {
		if operation == f.Generation {
			f.Busy = false
			f.Error = err.Error()
		}
	}
	f.Busy, f.Error = true, ""
	name := strings.TrimSpace(text(f.Name))
	write := func() {
		if operation != f.Generation {
			return
		}
		if !current() {
			f.Busy = false
			f.Error = "Codex restarted. Retry after reconnecting."
			return
		}
		f.Busy = true
		a.rpcInline("config/value/write", map[string]any{"keyPath": "mcp_servers." + configKey(name), "value": value, "mergeStrategy": "replace"}, func(json.RawMessage) {
			if operation != f.Generation {
				return
			}
			if !current() {
				f.Busy = false
				f.Error = "Codex restarted before the save was confirmed. Reload to check."
				return
			}
			a.rpcInline("config/mcpServer/reload", map[string]any{}, func(json.RawMessage) {
				if operation != f.Generation {
					return
				}
				f.Busy = false
				if !current() {
					f.Error = "Codex restarted. Reload the server list to check the saved configuration."
					return
				}
				f.Open = false
				if a.settingsView != nil && a.settingsView.Page == "MCP servers" {
					a.loadSettingsPage("MCP servers")
				}
			}, func(err error) {
				if operation != f.Generation {
					return
				}
				f.Busy = false
				f.Error = "Configuration saved, but servers could not reload: " + err.Error()
			})
		}, fail)
	}
	a.rpcInline("config/read", map[string]any{"includeLayers": false, "cwd": a.prefs.WorkingDirectory}, func(raw json.RawMessage) {
		if operation != f.Generation {
			return
		}
		if !current() {
			f.Busy = false
			f.Error = "Codex restarted. Retry after reconnecting."
			return
		}
		var result struct {
			Config struct {
				Servers map[string]json.RawMessage `json:"mcp_servers"`
			} `json:"config"`
		}
		if err := json.Unmarshal(raw, &result); err != nil {
			fail(err)
			return
		}
		if _, exists := result.Config.Servers[name]; exists {
			f.Busy = false
			a.confirm("Replace MCP server?", "Replace the existing configuration for "+name+"?", write)
		} else {
			write()
		}
	}, fail)
}
