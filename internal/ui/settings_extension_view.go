package ui

import (
	"encoding/json"
	"strings"

	"github.com/aarzilli/nucular"
)

func extensionPage(page string) bool {
	switch page {
	case "MCP servers", "Features", "Skills", "Hooks", "Plugins":
		return true
	}
	return false
}
func (a *App) drawExtensionSettings(w *nucular.Window, s *settingsView) {
	if s.LoadError != "" {
		a.drawSettingsError(w, s.LoadError)
	}
	if s.Busy {
		muted(w, "Loading…", a.p)
	}
	if s.Page == "Skills" {
		muted(w, "Enablement follows Codex configuration precedence. A higher-precedence setting may override your choice; the refreshed switch shows the effective value.", a.p)
	}
	if s.Page == "Features" && !a.catalog.PolicyLoaded {
		muted(w, "Loading organization policy before changing features…", a.p)
		if a.policyError != "" {
			a.drawSettingsError(w, a.policyError)
		}
	}
	group := ""
	shown := 0
	needle := strings.ToLower(strings.TrimSpace(text(s.PluginSearch)))
	for i := range s.Items {
		item := &s.Items[i]
		if s.Page == "Plugins" && needle != "" && !strings.Contains(strings.ToLower(item.Name+" "+item.Description+" "+item.Group), needle) {
			continue
		}
		shown++
		if item.Group != "" && item.Group != group {
			group = item.Group
			title(w, group, a.p)
		}
		w.Row(29).Dynamic(1)
		w.Label(item.Name, "LC")
		status := item.Status
		itemError := item.Error
		if s.Page == "MCP servers" && item.Enabled {
			if live, ok := s.MCPLive[item.ID]; ok {
				status, itemError = live.Status, live.Error
			}
		}
		locked := item.ReadOnly
		if s.Page == "Features" {
			if required, ok := a.catalog.Requirements[item.ID]; ok {
				locked = true
				status = strings.TrimSpace(status + " · Managed: " + map[bool]string{true: "on", false: "off"}[required])
			}
			locked = locked || !a.catalog.PolicyLoaded || a.policyLoading
		}
		if status != "" {
			muted(w, strings.TrimPrefix(status, " · "), a.p)
		}
		if item.Subtitle != "" {
			muted(w, item.Subtitle, a.p)
		}
		if item.Description != "" {
			w.Row(42).Dynamic(1)
			w.LabelWrap(item.Description)
		}
		if item.Detail != "" {
			muted(w, item.Detail, a.p)
		}
		if itemError != "" {
			a.drawSettingsError(w, itemError)
		}
		if item.Raw == nil {
			continue
		}
		key := settingsRowKey(s.Page, item.ID)
		feedback := s.ActionFeedback[key]
		if feedback.Pending {
			muted(w, feedback.Message, a.p)
		} else {
			w.Row(29).Dynamic(4)
			if !locked && (s.Page != "Plugins" || flag(item.Raw, "installed")) {
				on := item.Enabled
				if w.CheckboxText("Enabled", &on) {
					a.setExtensionEnabled(s.Page, *item, on)
				}
			} else {
				w.Label("", "LC")
			}
			switch s.Page {
			case "MCP servers":
				if item.CanRemove {
					if w.ButtonText("Remove…") {
						a.removeMCP(*item)
					}
				} else {
					w.Label("", "LC")
				}
				if item.CanLogin {
					if w.ButtonText("Sign in") {
						a.mcpLogin(*item)
					}
				} else {
					w.Label("", "LC")
				}
			case "Plugins":
				if item.CanRemove {
					if w.ButtonText("Uninstall…") {
						a.uninstallPlugin(item.ID)
					}
				} else if item.CanInstall {
					if w.ButtonText("Install") {
						a.settingsRequestAt(key, "plugin/install", pluginParams(*item))
					}
				} else {
					w.Label("", "LC")
				}
				w.Label("", "LC")
			case "Hooks":
				if item.NeedsTrust {
					if w.ButtonText("Trust…") {
						a.trustHook(*item)
					}
				} else {
					w.Label("", "LC")
				}
				if path := str(item.Raw, "sourcePath"); path != "" {
					if w.ButtonText("Open source") {
						a.openFile(path)
					}
				} else {
					w.Label("", "LC")
				}
			case "Skills":
				if w.ButtonText("Open skill") {
					a.openFile(item.ID)
				}
				w.Label("", "LC")
			default:
				w.Label("", "LC")
				w.Label("", "LC")
			}
			if w.ButtonText("Details") {
				if s.Page == "Plugins" {
					a.inspectRPC(item.Name, "plugin/read", pluginParams(*item))
				} else {
					b, _ := json.MarshalIndent(item.Raw, "", "  ")
					a.openText(item.Name, string(b))
				}
			}
		}
		if !feedback.Pending {
			a.drawSettingFeedback(w, feedback)
		}
		if s.Page == "Plugins" {
			a.configFeedback(w, s, "plugins."+configKey(item.ID)+".enabled")
		}
	}
	if shown == 0 && !s.Busy && s.LoadError == "" {
		muted(w, "No items found.", a.p)
	}
}
func (a *App) setExtensionEnabled(page string, item settingsItem, on bool) {
	if item.ReadOnly {
		return
	}
	key := settingsRowKey(page, item.ID)
	switch page {
	case "Features":
		if !a.catalog.PolicyLoaded || a.policyLoading {
			return
		}
		if _, locked := a.catalog.Requirements[item.ID]; locked {
			return
		}
		a.settingsRequestAt(key, "config/batchWrite", map[string]any{"edits": []any{map[string]any{"keyPath": "features." + configKey(item.ID), "value": on, "mergeStrategy": "replace"}}, "reloadUserConfig": true})
	case "Hooks":
		a.settingsRequestAt(key, "config/batchWrite", map[string]any{"edits": []any{map[string]any{"keyPath": "hooks.state", "value": map[string]any{item.ID: map[string]any{"enabled": on}}, "mergeStrategy": "upsert"}}, "reloadUserConfig": true})
	case "Skills":
		a.settingsRequestAt(key, "skills/config/write", map[string]any{"path": item.ID, "enabled": on})
	case "MCP servers":
		a.settingsRequestAt(key, "config/value/write", map[string]any{"keyPath": "mcp_servers." + configKey(item.ID) + ".enabled", "value": on, "mergeStrategy": "replace"})
	case "Plugins":
		if flag(item.Raw, "installed") {
			a.configWrite("plugins."+configKey(item.ID)+".enabled", on)
		}
	}
}
func (a *App) removeMCP(item settingsItem) {
	if !item.CanRemove {
		return
	}
	a.confirm("Remove MCP server?", "Remove the user configuration for "+item.Name+"?", func() {
		a.settingsRequestAt(settingsRowKey("MCP servers", item.ID), "config/value/write", map[string]any{"keyPath": "mcp_servers." + configKey(item.ID), "value": nil, "mergeStrategy": "replace"})
	})
}
func (a *App) trustHook(item settingsItem) {
	if !item.NeedsTrust {
		return
	}
	a.confirm("Trust hook?", "Allow the current contents of this hook to run?", func() {
		a.settingsRequestAt(settingsRowKey("Hooks", item.ID), "config/batchWrite", map[string]any{"edits": []any{map[string]any{"keyPath": "hooks.state", "value": map[string]any{item.ID: map[string]any{"trusted_hash": str(item.Raw, "currentHash")}}, "mergeStrategy": "upsert"}}, "reloadUserConfig": true})
	})
}
func (a *App) mcpLogin(item settingsItem) {
	if !item.CanLogin {
		return
	}
	s := a.settingsView
	key := settingsRowKey("MCP servers", item.ID)
	if s.ActionFeedback == nil {
		s.ActionFeedback = map[string]settingFeedback{}
	}
	if s.ActionFeedback[key].Pending {
		return
	}
	s.ActionFeedback[key] = settingFeedback{Pending: true, Message: "Opening sign-in…"}
	a.rpcInline("mcpServer/oauth/login", map[string]any{"name": item.ID}, func(raw json.RawMessage) {
		var result struct {
			AuthorizationURL string `json:"authorizationUrl"`
		}
		if err := json.Unmarshal(raw, &result); err != nil || result.AuthorizationURL == "" {
			s.ActionFeedback[key] = settingFeedback{Failed: true, Message: "Codex did not return a sign-in URL."}
			return
		}
		s.ActionFeedback[key] = settingFeedback{Message: "Complete sign-in in your browser."}
		a.openURL(result.AuthorizationURL)
	}, func(err error) { s.ActionFeedback[key] = settingFeedback{Failed: true, Message: err.Error()} })
}
