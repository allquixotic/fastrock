package ui

import (
	"encoding/json"
	"time"
)

type mcpLiveStatus struct{ Status, Error string }

func (a *App) settingsEvent(method string, data map[string]any) {
	switch method {
	case "mcpServer/startupStatus/updated":
		if s := a.settingsView; s != nil {
			if s.MCPLive == nil {
				s.MCPLive = map[string]mcpLiveStatus{}
			}
			if name := str(data, "name"); name != "" {
				state := str(data, "status")
				label := map[string]string{"starting": "Starting", "ready": "Connected", "failed": "Failed", "cancelled": "Cancelled"}[state]
				if label != "" {
					s.MCPLive[name] = mcpLiveStatus{label, str(data, "error")}
				}
			}
		}
	case "account/rateLimits/updated":
		if value, ok := data["rateLimits"]; ok {
			encoded, err := json.Marshal(value)
			var snapshot usageSnapshot
			if err == nil && json.Unmarshal(encoded, &snapshot) == nil {
				a.accountUsage.update(snapshot)
			}
		}
	case "account/login/completed":
		if s := a.settingsView; s != nil && s.LoginID != "" && s.LoginID == str(data, "loginId") {
			s.LoginBusy = false
			s.LoginID, s.LoginCode, s.LoginURL = "", "", ""
			if success, _ := data["success"].(bool); !success {
				s.LoginError = fallback(str(data, "error"), "Sign-in did not complete.")
			}
		}
		a.refreshConfiguredProvider()
	case "account/updated", "account/gatewayOAuth/changed":
		a.refreshConfiguredProvider()
	}
	a.settingsNotification(method)
}

func settingsSubtitle(page string) string {
	switch page {
	case "Appearance":
		return "Customize Fastrock windows, colors and text."
	case "Rally":
		return "Connect your workspace and choose the scope for work items."
	case "Account":
		return "Manage the account used by your installed Codex."
	case "AWS Bedrock", "Local providers":
		return "Configure the provider used by Codex conversations."
	case "Codex configuration", "Raw configuration":
		return "Review and edit the configuration used by your installed Codex."
	case "Keyboard":
		return "Customize shortcuts and resolve conflicting bindings."
	case "MCP servers":
		return "Manage tool servers and their connection status."
	case "Skills", "Plugins", "Hooks":
		return "Manage capabilities available to Codex."
	case "Features":
		return "Choose the experimental capabilities to enable."
	case "About":
		return "Version, updates and project information."
	default:
		return "Review and manage " + page + " for this Codex installation."
	}
}

func (a *App) settingsNotification(method string) {
	page := ""
	switch method {
	case "account/login/completed", "account/updated", "account/gatewayOAuth/changed":
		a.refreshAccount()
		page = "Account"
	case "account/rateLimits/updated":
		page = "Account"
	case "mcpServer/startupStatus/updated", "mcpServer/oauthLogin/completed":
		page = "MCP servers"
	case "skills/changed":
		a.skillCache.invalidate()
		page = "Skills"
	case "externalAgentConfig/import/progress", "externalAgentConfig/import/completed":
		page = "Import"
	case "windowsSandbox/setupCompleted":
		page = "Sandbox"
	}
	s := a.settingsView
	if s == nil || s.Page != page {
		return
	}
	if s.ReloadTimer != nil {
		s.ReloadTimer.Stop()
	}
	s.ReloadTimer = time.AfterFunc(200*time.Millisecond, func() {
		a.post(func() {
			if a.settingsView == s && s.Page == page {
				a.loadSettingsPage(page)
			}
		})
	})
}
