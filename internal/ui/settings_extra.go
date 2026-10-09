package ui

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/settings"
	"golang.org/x/mobile/event/key"
)

type settingsItem struct {
	Name, ID, Description                                 string
	Group, Subtitle, Status, Detail, Error                string
	Enabled                                               bool
	ReadOnly, CanInstall, CanRemove, CanLogin, NeedsTrust bool
	Raw                                                   map[string]any
}

func (a *App) inspectRPC(title, method string, params any) {
	s := a.settingsView
	key := "page:" + s.Page + ":details:" + title
	if s.ActionFeedback[key].Pending {
		return
	}
	setSettingsFeedback(s, key, settingFeedback{Pending: true, Message: "Loading details…"})
	a.rpcInline(method, params, func(raw json.RawMessage) {
		setSettingsFeedback(s, key, settingFeedback{})
		a.work(func() {
			var data any
			_ = json.Unmarshal(raw, &data)
			scrub(data)
			b, _ := json.MarshalIndent(data, "", "  ")
			a.post(func() { a.openText(title, string(b)) })
		})
	}, func(err error) { setSettingsFeedback(s, key, settingFeedback{Failed: true, Message: err.Error()}) })
}
func (a *App) settingsRequest(method string, params any) {
	a.settingsRequestAt("page:"+a.settingsView.Page+":"+method, method, params)
}
func pluginParams(item settingsItem) map[string]any {
	p := map[string]any{"pluginName": str(item.Raw, "name")}
	if path := str(item.Raw, "_marketplacePath"); path != "" {
		p["marketplacePath"] = path
	} else {
		p["remoteMarketplaceName"] = str(item.Raw, "_marketplaceName")
	}
	return p
}
func (a *App) uninstallPlugin(id string) {
	a.confirm("Uninstall plugin?", "Remove "+id+" and its skills, MCP servers and hooks?", func() {
		a.settingsRequestAt(settingsRowKey("Plugins", id), "plugin/uninstall", map[string]any{"pluginId": id})
	})
}
func (a *App) login(params map[string]any) {
	s := a.settingsView
	method, _ := params["type"].(string)
	if s == nil || s.LoginBusy || a.client == nil || a.serverPaused {
		return
	}
	if !a.catalog.PolicyLoaded || a.policyLoading {
		s.LoginError = "Sign-in policy has not loaded. Reload it before signing in."
		return
	}
	if !a.catalog.Policy.LoginAllowed(method, str(a.catalog.Config, "forced_login_method")) {
		s.LoginError = "This sign-in method is not allowed by organization policy."
		return
	}
	s.LoginGeneration++
	generation := s.LoginGeneration
	s.LoginBusy, s.LoginError = true, ""
	a.rpcInline("account/login/start", params, func(raw json.RawMessage) {
		if s.LoginGeneration != generation {
			return
		}
		r := codex.Decode(raw)
		s.LoginID = str(r, "loginId")
		if url := str(r, "authUrl"); url != "" {
			s.LoginURL = url
			a.openURL(url)
		}
		if url := str(r, "verificationUrl"); url != "" {
			s.LoginURL = url
			a.openURL(url)
		}
		if code := str(r, "userCode"); code != "" {
			a.copyText(code)
			s.LoginCode = code
			a.toast = "Enter sign-in code: " + code + " (copied)"
		}
		if s.LoginID == "" {
			s.LoginBusy = false
			a.refreshAccount()
			a.refreshUsage()
			a.refreshConfiguredProvider()
		}
	}, func(err error) {
		if s.LoginGeneration == generation {
			s.LoginBusy, s.LoginError = false, err.Error()
		}
	})
}
func (a *App) drawExtraSettings(w *nucular.Window, s *settingsView) {
	w.Row(29).Static(100, 100, 130)
	if w.ButtonText("Refresh") {
		a.loadSettingsPage(s.Page)
	}
	if w.ButtonText("View data") {
		a.openText(s.Page, text(s.Output))
	}
	if w.ButtonText("Open configuration") {
		a.work(func() { home := codexHome(); a.post(func() { a.openFile(filepath.Join(home, "config.toml")) }) })
	}
	switch s.Page {
	case "Account":
		a.drawAccount(w, s)
		return
	case "AWS Bedrock":
		a.bedrockSettings(w, s)
		return
	case "MCP servers":
		a.drawMCPSettings(w, s)
	case "Plugins":
		w.Row(30).Dynamic(2)
		if button(w, "Installed plugins", !s.PluginCatalog, a.p) {
			s.PluginCatalog = false
			a.loadSettingsPage(s.Page)
		}
		if button(w, "Browse marketplace", s.PluginCatalog, a.p) {
			s.PluginCatalog = true
			a.loadSettingsPage(s.Page)
		}
		w.Row(28).Dynamic(1)
		s.PluginSearch.Placeholder = "Search plugins"
		s.PluginSearch.Edit(w)
		a.field(w, "Plugin name / installed ID", s.Name, false)
		a.field(w, "Marketplace path (optional)", s.Value, false)
		w.Row(30).Static(100, 110)
		if w.ButtonText("Install") {
			params := map[string]any{"pluginName": text(s.Name)}
			if text(s.Value) != "" {
				params["marketplacePath"] = text(s.Value)
			}
			a.settingsRequest("plugin/install", params)
		}
		if w.ButtonText("Uninstall") {
			a.uninstallPlugin(text(s.Name))
		}
	case "Import":
		a.field(w, "Migration source", s.Name, false)
		w.Row(30).Static(150, 150, 100)
		if w.ButtonText("Select all") {
			for _, item := range s.Items {
				s.Selected[item.ID] = true
			}
		}
		if w.ButtonText("Detect sources") {
			a.loadSettingsPage(s.Page)
		}
		if w.ButtonText("Import selected") {
			var selected []any
			for _, item := range s.Items {
				if s.Selected[item.ID] {
					selected = append(selected, item.Raw)
				}
			}
			a.settingsRequest("externalAgentConfig/import", map[string]any{"migrationItems": selected, "migrationSource": text(s.Name), "source": "fastrock"})
		}
	case "Feedback":
		a.field(w, "Feedback", s.Value, true)
		w.Row(28).Dynamic(1)
		w.CheckboxText("Include Codex logs", &s.IncludeLogs)
		w.Row(30).Static(140)
		if w.ButtonText("Submit feedback") {
			a.settingsRequest("feedback/upload", map[string]any{"classification": "bug", "reason": text(s.Value), "includeLogs": s.IncludeLogs})
		}
	case "Sandbox":
		w.Row(30).Static(190, 190)
		if w.ButtonText("Set up elevated sandbox") {
			a.settingsRequest("windowsSandbox/setupStart", map[string]any{"mode": "elevated", "cwd": a.prefs.WorkingDirectory})
		}
		if w.ButtonText("Set up standard sandbox") {
			a.settingsRequest("windowsSandbox/setupStart", map[string]any{"mode": "unelevated", "cwd": a.prefs.WorkingDirectory})
		}
	case "Diagnostics":
		w.Row(30).Static(140, 140)
		if w.ButtonText("Read configuration") {
			a.inspectRPC("Configuration layers", "config/read", map[string]any{"includeLayers": true, "cwd": a.prefs.WorkingDirectory})
		}
		if w.ButtonText("Open logs") {
			a.openPath(filepath.Join(codexHome(), "log"), false)
		}
	}
	if extensionPage(s.Page) {
		a.drawExtensionSettings(w, s)
		return
	}
	if s.LoadError != "" {
		a.drawSettingsError(w, s.LoadError)
	}
	for i := range s.Items {
		item := &s.Items[i]
		w.Row(30).Dynamic(2)
		w.Label(item.Name, "LC")
		if s.Page == "Import" {
			on := s.Selected[item.ID]
			if w.CheckboxText("Import", &on) {
				s.Selected[item.ID] = on
			}
		}
		if item.Description != "" {
			muted(w, cut(item.Description, 240), a.p)
		}
	}
	if len(s.Items) == 0 {
		w.Row(max(120, w.LayoutAvailableHeight()-10)).Dynamic(1)
		codeEditor(w, s.Output)
	}
}
func settingsItems(page string, data any) []settingsItem {
	if items, ok := extensionItems(page, data); ok {
		return items
	}
	var result []settingsItem
	var visit func(any)
	visit = func(v any) {
		switch x := v.(type) {
		case []any:
			for _, y := range x {
				visit(y)
			}
		case map[string]any:
			name, _ := x["name"].(string)
			if page == "Import" {
				name, _ = x["description"].(string)
				if name != "" {
					kind, _ := x["itemType"].(string)
					cwd, _ := x["cwd"].(string)
					name = kind + ": " + name + " " + cwd
				}
			}
			if name == "" {
				name, _ = x["displayName"].(string)
			}
			id, _ := x["id"].(string)
			if page == "Hooks" {
				id, _ = x["key"].(string)
				if id != "" {
					name = str(x, "eventName") + " · " + str(x, "sourcePath")
				}
			}
			if page == "Skills" {
				id, _ = x["path"].(string)
			}
			if id == "" {
				id = name
			}
			if name != "" && id != "" {
				enabled, _ := x["enabled"].(bool)
				desc, _ := x["description"].(string)
				result = append(result, settingsItem{Name: name, ID: id, Description: desc, Enabled: enabled, Raw: x})
				return
			}
			for _, y := range x {
				switch y.(type) {
				case []any, map[string]any:
					visit(y)
				}
			}
		}
	}
	visit(data)
	sort.Slice(result, func(i, j int) bool { return result[i].Name < result[j].Name })
	return result
}
func (a *App) loadSettingsData(page, method string, params any) {
	c := a.client
	if c == nil {
		return
	}
	s := a.settingsView
	if configurationPage(page) && (s.ConfigWriting || len(s.ConfigQueue) > 0) {
		return
	}
	s.LoadGeneration++
	generation := s.LoadGeneration
	server := a.serverGeneration
	cwd := a.prefs.WorkingDirectory
	s.Busy = true
	s.LoadError = ""
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		data, err := settingsPayload(ctx, c, page, method, params, cwd)
		scrub(data)
		items := settingsItems(page, data)
		var fields []configField
		var layers []configLayer
		if configurationPage(page) {
			if d, ok := data.(map[string]any); ok {
				layers = configLayers(d)
				if conf, ok := d["config"].(map[string]any); ok {
					fields = schemaConfigFields(conf)
				}
			}
		}
		b, _ := json.MarshalIndent(data, "", "  ")
		a.post(func() {
			if s.Page != page || s.LoadGeneration != generation || c != a.client || server != a.serverGeneration {
				return
			}
			s.Busy = false
			if err != nil {
				s.LoadError = err.Error()
			} else {
				s.LoadError = ""
			}
			s.Items = items
			if d, ok := data.(map[string]any); ok && configurationPage(page) {
				s.ConfigData = d
				s.Layers = layers
			}
			if configurationPage(page) {
				if d, ok := data.(map[string]any); ok {
					if conf, ok := d["config"].(map[string]any); ok {
						_ = conf
						s.Fields = mergeConfigFields(s.Fields, fields)
					}
					if layers, ok := d["layers"].([]any); ok {
						for _, layer := range layers {
							l, _ := layer.(map[string]any)
							source, _ := l["name"].(map[string]any)
							if source == nil {
								source, _ = l["source"].(map[string]any)
							}
							if str(source, "type") == "user" {
								s.ConfigVersion = str(l, "version")
							}
						}
					}
				}
			}
			setText(s.Output, string(b))
		})
	}, func() { s.Busy = false; s.LoadError = "The background work queue is full. Retry shortly." })
}
func (a *App) keyboardSettings(w *nucular.Window) {
	if a.recordShortcut != "" {
		muted(w, "Press a shortcut for "+a.recordShortcut+" (Esc cancels)", a.p)
	}
	for _, action := range shellActions() {
		w.Row(30).Ratio(.4, .28, .1, .1, .12)
		w.Label(action.Title, "LC")
		binding := shortcutLabel(action.Code, action.Mods)
		if custom, ok := a.prefs.Keymap[action.ID]; ok {
			binding = shortcutLabel(key.Code(custom.Code), key.Modifiers(custom.Mods))
			if custom.Code == 0 {
				binding = "Unbound"
			}
		}
		w.Label(binding, "LC")
		if w.ButtonText("Record") {
			a.recordShortcut = action.ID
		}
		if w.ButtonText("Reset") {
			delete(a.prefs.Keymap, action.ID)
			a.savePrefs()
		}
		if w.ButtonText("Unbind") {
			if a.prefs.Keymap == nil {
				a.prefs.Keymap = map[string]settings.KeyBinding{}
			}
			a.prefs.Keymap[action.ID] = settings.KeyBinding{}
			a.savePrefs()
		}
	}
	w.Row(30).Static(140)
	if w.ButtonText("Reset all shortcuts") {
		a.confirm("Reset all shortcuts?", "Restore all keyboard shortcuts to their defaults?", func() { a.prefs.Keymap = nil; a.savePrefs() })
	}
}
func codexHome() string {
	if h := os.Getenv("CODEX_HOME"); h != "" {
		return h
	}
	h, _ := os.UserHomeDir()
	return filepath.Join(h, ".codex")
}
