package ui

import (
	"encoding/json"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
)

func (a *App) settingsServerMessage() (string, bool) {
	if a.serverStarting || a.connecting {
		return "Codex is starting…", false
	}
	if a.client == nil || a.serverPaused {
		message := "Codex is not running."
		if a.serverError != "" {
			message += " " + a.serverError
		}
		return message, true
	}
	return "", false
}

func (a *App) drawSettingsServer(w *desktop.Window) {
	if message, failed := a.settingsServerMessage(); message != "" {
		if failed {
			a.drawSettingsError(w, message)
		} else {
			muted(w, message, a.p)
		}
		if failed {
			w.Row(28).Static(160, 160)
			if w.ButtonText("Edit config.toml") {
				a.settingsPage("Raw configuration")
			}
			if a.client == nil {
				if w.ButtonText("Reconnect Codex") {
					a.reconnect()
				}
			} else if w.ButtonText("Restart Codex") {
				a.confirmRestartServer()
			}
		}
	}
	if a.restartNote != "" {
		muted(w, a.restartNote, a.p)
	}
	if a.client != nil && !a.serverStarting && !a.restartPending {
		w.Row(28).Static(200)
		if w.ButtonText("Restart Codex app-server…") {
			a.confirmRestartServer()
		}
	}
}

func (a *App) confirmRestartServer() {
	a.confirm("Restart Codex?", "Stop running turns in all Fastrock windows and reload Codex configuration? Unsent drafts stay open.", a.restartServer)
}

func (a *App) restartServer() {
	if a.restartPending || a.client == nil {
		return
	}
	a.restartPending = true
	a.rpcResult("fastrock/restart", map[string]any{}, func(json.RawMessage) {
		a.restartPending = false
	}, func(err error) {
		a.restartPending = false
		a.serverStarting = false
		a.serverError = err.Error()
	})
}

func (a *App) observeProvider(provider string) {
	provider = strings.TrimSpace(provider)
	if provider == "" {
		provider = "openai"
	}
	if a.startedProvider == "" {
		a.startedProvider = provider
		return
	}
	if provider != a.startedProvider {
		a.restartNote = "Model provider changed from " + a.startedProvider + " to " + provider + ". Restart Codex to apply it."
	} else {
		a.restartNote = ""
	}
}

func (a *App) refreshConfiguredProvider() {
	if a.client == nil || a.serverPaused {
		return
	}
	client, generation := a.client, a.serverGeneration
	a.rpcResult("config/read", map[string]any{"includeLayers": false, "cwd": a.prefs.WorkingDirectory}, func(raw json.RawMessage) {
		if a.client != client || generation != a.serverGeneration {
			return
		}
		var result struct{ Config map[string]any }
		if json.Unmarshal(raw, &result) == nil && result.Config != nil {
			a.observeProvider(str(result.Config, "model_provider"))
			a.catalog.Config = result.Config
		}
	}, nil)
}

func (a *App) resetSettingsConnection() {
	if s := a.settingsView; s != nil {
		a.resetLocalProviders(s)
		s.LoadGeneration++
		s.Busy = false
		for key, feedback := range s.ActionFeedback {
			if feedback.Pending {
				if s.ActionRequest != nil {
					s.ActionRequest[key]++
				}
				s.ActionFeedback[key] = settingFeedback{Failed: true, Message: "The Codex connection changed. Reload to check the saved value."}
			}
		}
		s.MCPLive = nil
		if s.Bedrock != nil && s.Bedrock.Busy {
			s.Bedrock.Generation++
			s.Bedrock.Busy = false
			s.Bedrock.Feedback = settingFeedback{Failed: true, Message: "The Codex connection changed. Reload to check the saved provider."}
		}
		if s.MCP != nil && s.MCP.Busy {
			s.MCP.Generation++
			s.MCP.Busy = false
			s.MCP.Error = "The Codex connection changed. Reload and retry."
		}
		s.LoginGeneration++
		if s.LoginBusy {
			s.LoginError = "The Codex connection changed. Start sign-in again."
		}
		s.LoginBusy = false
		s.LoginID, s.LoginCode, s.LoginURL = "", "", ""
	}
}
