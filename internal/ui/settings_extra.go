package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/aarzilli/nucular/label"
	"image"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/settings"
)

type settingsItem struct {
	Name, ID, Description string
	Enabled               bool
	Raw                   map[string]any
}

func (a *App) inspectRPC(title, method string, params any) {
	a.rpc(method, params, func(raw json.RawMessage) {
		a.work(func() {
			var data any
			_ = json.Unmarshal(raw, &data)
			scrub(data)
			b, _ := json.MarshalIndent(data, "", "  ")
			a.post(func() { a.openText(title, string(b)) })
		})
	})
}
func (a *App) settingsRequest(method string, params any) {
	page := a.settingsView.Page
	a.rpc(method, params, func(raw json.RawMessage) {
		a.toast = "Saved"
		if a.settingsView.Page == page {
			a.loadSettingsPage(page)
		}
	})
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
	a.confirm("Uninstall plugin?", "Remove "+id+" and its skills, MCP servers and hooks?", func() { a.settingsRequest("plugin/uninstall", map[string]any{"pluginId": id}) })
}
func (a *App) login(params map[string]any) {
	a.rpc("account/login/start", params, func(raw json.RawMessage) {
		r := codex.Decode(raw)
		a.settingsView.LoginID = str(r, "loginId")
		if url := str(r, "authUrl"); url != "" {
			a.openURL(url)
		}
		if url := str(r, "verificationUrl"); url != "" {
			a.openURL(url)
		}
		if code := str(r, "userCode"); code != "" {
			a.toast = "Enter sign-in code: " + code
			a.copyText(code)
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
		w.Row(30).Static(150, 140, 100)
		if w.ButtonText("Sign in with ChatGPT") {
			a.login(map[string]any{"type": "chatgpt"})
		}
		if w.ButtonText("Device code sign-in") {
			a.login(map[string]any{"type": "chatgptDeviceCode"})
		}
		if w.ButtonText("Cancel login") {
			a.settingsRequest("account/login/cancel", map[string]any{"loginId": s.LoginID})
		}
		s.Secret.PasswordChar = '●'
		a.field(w, "API key", s.Secret, false)
		w.Row(30).Static(150, 100, 150)
		if w.ButtonText("Sign in with key") {
			a.login(map[string]any{"type": "apiKey", "apiKey": text(s.Secret)})
			setText(s.Secret, "")
		}
		if w.ButtonText("Sign out") {
			a.settingsRequest("account/logout", map[string]any{})
		}
		if w.ButtonText("Usage and limits") {
			a.inspectRPC("Usage limits", "account/rateLimits/read", map[string]any{})
		}
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
		s.Search.Placeholder = "Search plugins"
		s.Search.Edit(w)
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
			a.settingsRequest("plugin/uninstall", map[string]any{"pluginId": text(s.Name)})
		}
	case "Memories":
		w.Row(30).Static(180, 130)
		if w.ButtonText("Reset saved memories…") {
			a.confirm("Reset memories?", "Clear the memories managed by Codex?", func() { a.settingsRequest("memory/reset", map[string]any{}) })
		}
		if w.ButtonText("Learn more") {
			a.openURL("https://developers.openai.com/codex")
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
		a.field(w, "Configuration key", s.Name, false)
		w.Row(30).Static(140, 140)
		if w.ButtonText("Read configuration") {
			a.inspectRPC("Configuration layers", "config/read", map[string]any{"includeLayers": true, "cwd": a.prefs.WorkingDirectory})
		}
		if w.ButtonText("Open logs") {
			a.openPath(filepath.Join(codexHome(), "log"), false)
		}
	}
	for i := range s.Items {
		item := &s.Items[i]
		if s.Page == "Plugins" && !strings.Contains(strings.ToLower(item.Name+" "+item.Description), strings.ToLower(text(s.Search))) {
			continue
		}
		w.Row(30).Ratio(.7, .15, .15)
		w.Label(item.Name, "LC")
		if menu := w.ContextualOpen(0, image.Pt(240, 115), w.LastWidgetBounds, nil); menu != nil {
			if path := str(item.Raw, "sourcePath"); path != "" && menu.MenuItem(label.T("Open source")) {
				a.openFile(path)
			}
			if s.Page == "Skills" && menu.MenuItem(label.T("Open skill")) {
				a.openFile(item.ID)
			}
			if s.Page == "MCP servers" && menu.MenuItem(label.T("Remove server…")) {
				name := item.Name
				a.confirm("Remove MCP server?", name, func() {
					a.settingsRequest("config/value/write", map[string]any{"keyPath": "mcp_servers." + configKey(name), "value": nil, "mergeStrategy": "replace"})
				})
			}
			if s.Page == "Hooks" && str(item.Raw, "currentHash") != "" && menu.MenuItem(label.T("Trust current hook…")) {
				id, hash := item.ID, str(item.Raw, "currentHash")
				a.confirm("Trust hook?", "Allow the current contents of this hook to run?", func() {
					a.settingsRequest("config/batchWrite", map[string]any{"edits": []any{map[string]any{"keyPath": "hooks.state", "value": map[string]any{id: map[string]any{"trusted_hash": hash}}, "mergeStrategy": "upsert"}}, "reloadUserConfig": true})
				})
			}
		}
		switch s.Page {
		case "Features":
			on := item.Enabled
			if w.CheckboxText("Enabled", &on) {
				a.settingsRequest("experimentalFeature/enablement/set", map[string]any{"enablement": map[string]bool{item.ID: on}})
			}
		case "Hooks":
			if managed, _ := item.Raw["isManaged"].(bool); !managed {
				on := item.Enabled
				if w.CheckboxText("Enabled", &on) {
					a.settingsRequest("config/batchWrite", map[string]any{"edits": []any{map[string]any{"keyPath": "hooks.state", "value": map[string]any{item.ID: map[string]any{"enabled": on}}, "mergeStrategy": "upsert"}}, "reloadUserConfig": true})
				}
			} else {
				w.Label("Managed", "LC")
			}
		case "Skills":
			on := item.Enabled
			if w.CheckboxText("Enabled", &on) {
				a.settingsRequest("skills/config/write", map[string]any{"path": item.ID, "enabled": on})
			}
		case "MCP servers":
			enabled := true
			if value, ok := item.Raw["enabled"].(bool); ok {
				enabled = value
			}
			if w.CheckboxText("Enabled", &enabled) {
				a.settingsRequest("config/value/write", map[string]any{"keyPath": "mcp_servers." + configKey(item.Name) + ".enabled", "value": enabled, "mergeStrategy": "replace"})
			}
			if w.ButtonText("Sign in") {
				a.rpc("mcpServer/oauth/login", map[string]any{"name": item.Name}, func(raw json.RawMessage) { a.openURL(str(codex.Decode(raw), "authorizationUrl")) })
			}
		case "Plugins":
			installed, _ := item.Raw["installed"].(bool)
			if installed {
				on := item.Enabled
				if w.CheckboxText("Enabled", &on) {
					a.configWrite("plugins."+configKey(item.ID)+".enabled", on)
				}
			} else {
				w.Label("Available", "LC")
			}
			if installed && w.ButtonText("Uninstall…") {
				a.uninstallPlugin(item.ID)
			}
			if !installed && w.ButtonText("Install") {
				a.settingsRequest("plugin/install", pluginParams(*item))
			}
		case "Import":
			on := s.Selected[item.ID]
			if w.CheckboxText("Import", &on) {
				s.Selected[item.ID] = on
			}
		}
		if w.ButtonText("Details") {
			if s.Page == "Plugins" {
				a.inspectRPC(item.Name, "plugin/read", pluginParams(*item))
			} else {
				b, _ := json.MarshalIndent(item.Raw, "", "  ")
				a.openText(item.Name, string(b))
			}
		}
		if item.Description != "" {
			muted(w, cut(item.Description, 240), a.p)
		}
	}
	if len(s.Items) == 0 {
		w.Row(max(120, w.LayoutAvailableHeight()-10)).Dynamic(1)
		s.Output.Edit(w)
	}
}
func settingsItems(page string, data any) []settingsItem {
	var result []settingsItem
	if page == "Plugins" {
		root, _ := data.(map[string]any)
		markets, _ := root["marketplaces"].([]any)
		for _, value := range markets {
			market, _ := value.(map[string]any)
			plugins, _ := market["plugins"].([]any)
			for _, value := range plugins {
				p, _ := value.(map[string]any)
				p["_marketplacePath"], p["_marketplaceName"] = market["path"], market["name"]
				on, _ := p["enabled"].(bool)
				face, _ := p["interface"].(map[string]any)
				name := fallback(str(face, "displayName"), str(p, "name"))
				result = append(result, settingsItem{Name: name, ID: str(p, "id"), Description: str(face, "shortDescription"), Enabled: on, Raw: p})
			}
		}
		sort.Slice(result, func(i, j int) bool { return result[i].Name < result[j].Name })
		return result
	}
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
	s.LoadGeneration++
	generation := s.LoadGeneration
	s.Busy = true
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var data any
		err := c.Call(ctx, method, params, &data)
		scrub(data)
		items := settingsItems(page, data)
		var fields []configField
		var layers []configLayer
		if page == "Codex configuration" {
			if d, ok := data.(map[string]any); ok {
				layers = configLayers(d)
				if conf, ok := d["config"].(map[string]any); ok {
					fields = schemaConfigFields(conf)
				}
			}
		}
		b, _ := json.MarshalIndent(data, "", "  ")
		a.post(func() {
			if s.Page != page || s.LoadGeneration != generation {
				return
			}
			s.Busy = false
			a.report(err)
			s.Items = items
			if d, ok := data.(map[string]any); ok && page == "Codex configuration" {
				s.ConfigData = d
				s.Layers = layers
			}
			if page == "Codex configuration" {
				if d, ok := data.(map[string]any); ok {
					if conf, ok := d["config"].(map[string]any); ok {
						_ = conf
						s.Fields = fields
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
	})
}
func (a *App) keyboardSettings(w *nucular.Window) {
	if a.recordShortcut != "" {
		muted(w, "Press a shortcut for "+a.recordShortcut+" (Esc cancels)", a.p)
	}
	for _, action := range shellActions() {
		w.Row(30).Ratio(.4, .28, .1, .1, .12)
		w.Label(action.Title, "LC")
		binding := fmt.Sprintf("%v + %v", action.Mods, action.Code)
		if custom, ok := a.prefs.Keymap[action.ID]; ok {
			binding = fmt.Sprintf("%v + %v", custom.Mods, custom.Code)
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
		a.prefs.Keymap = nil
		a.savePrefs()
	}
}
func codexHome() string {
	if h := os.Getenv("CODEX_HOME"); h != "" {
		return h
	}
	h, _ := os.UserHomeDir()
	return filepath.Join(h, ".codex")
}
