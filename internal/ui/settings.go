package ui

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

type settingsView struct {
	Page                                                                string
	Endpoint, Token, Workspace, Project, ConfigKey, ConfigValue, Output *nucular.TextEditor
	Busy                                                                bool
}

func newSettingsView(p settings.Preferences) *settingsView {
	return &settingsView{Page: "Appearance", Endpoint: textEditor(p.RallyEndpoint, false), Token: &nucular.TextEditor{Flags: nucular.EditSimple, PasswordChar: '●'}, Workspace: textEditor(p.RallyWorkspace, false), Project: textEditor(p.RallyProject, false), ConfigKey: textEditor("", false), ConfigValue: textEditor("", false), Output: textEditor("", true)}
}
func (a *App) drawSettings(w *nucular.Window) {
	s := a.settingsView
	if s == nil {
		s = newSettingsView(a.prefs)
		a.settingsView = s
	}
	title(w, "Settings", a.p)
	if a.client == nil {
		w.Row(30).Static(200)
		if w.ButtonText("Reconnect Codex") {
			a.reconnect()
		}
	}
	w.Row(max(200, w.LayoutAvailableHeight()-10)).Static(170, max(300, w.LayoutAvailableWidth()-180))
	if nav := w.GroupBegin("settings-nav", nucular.WindowNoHScrollbar); nav != nil {
		for _, page := range []string{"Appearance", "Rally", "Models", "Codex configuration", "Account", "MCP servers", "Skills", "Keyboard", "About"} {
			nav.Row(30).Dynamic(1)
			if button(nav, page, s.Page == page, a.p) {
				s.Page = page
				setText(s.Output, "")
				a.loadSettingsPage(page)
			}
		}
		nav.GroupEnd()
	}
	if body := w.GroupBegin("settings-content", nucular.WindowNoHScrollbar); body != nil {
		title(body, s.Page, a.p)
		switch s.Page {
		case "Appearance":
			body.Row(30).Static(100, 100)
			if button(body, "Dark", a.prefs.Theme == "dark", a.p) {
				a.prefs.Theme = "dark"
				a.theme()
			}
			if button(body, "Light", a.prefs.Theme == "light", a.p) {
				a.prefs.Theme = "light"
				a.theme()
			}
			title(body, "Font size", a.p)
			body.Row(30).Static(180)
			sizes := []string{"11", "12", "13", "14", "15", "16", "18", "20"}
			i := body.ComboSimple(sizes, index(sizes, strconv.Itoa(a.prefs.FontSize)), 28)
			n, _ := strconv.Atoi(sizes[i])
			if n != a.prefs.FontSize {
				a.prefs.FontSize = n
				a.theme()
			}
			body.Row(30).Dynamic(1)
			if body.CheckboxText("Enter sends; Shift+Enter inserts a new line", &a.prefs.EnterSends) {
				a.savePrefs()
			}
		case "Rally":
			a.field(body, "Rally endpoint", s.Endpoint, false)
			a.field(body, "API token (stored in OS credential store)", s.Token, false)
			muted(body, "Leave token empty to keep the existing saved token.", a.p)
			body.Row(32).Static(170, 170)
			if !s.Busy && primary(body, "Save and connect", a.p) {
				s.Busy = true
				endpoint := strings.TrimRight(strings.TrimSpace(text(s.Endpoint)), "/")
				token := text(s.Token)
				if _, err := rally.New(endpoint, "validation", nil); err != nil {
					s.Busy = false
					a.report(err)
					return
				}
				a.work(func() {
					var e error
					if token != "" {
						e = a.store.SetToken(endpoint, token)
					}
					a.post(func() {
						s.Busy = false
						if e != nil {
							a.report(e)
							return
						}
						a.prefs.RallyEndpoint = endpoint
						setText(s.Token, "")
						a.rallyClient = nil
						a.savePrefs()
						a.connectRally()
					})
				})
			}
			if body.ButtonText("Forget saved token") {
				endpoint := a.prefs.RallyEndpoint
				a.work(func() {
					e := a.store.SetToken(endpoint, "")
					a.post(func() {
						a.report(e)
						a.rallyClient = nil
						a.rallyErr = "Connect your Rally endpoint and API token in Settings."
					})
				})
			}
			if a.rallyErr != "" {
				body.Row(48).Dynamic(1)
				body.LabelWrap(a.rallyErr)
			} else {
				muted(body, "Connected", a.p)
			}
			a.scopePicker(body, "Workspace", a.workspaces, &a.prefs.RallyWorkspace, func() { a.prefs.RallyProject = ""; a.loadScope() })
			a.scopePicker(body, "Project", a.projects, &a.prefs.RallyProject, a.loadScope)
			body.Row(28).Dynamic(1)
			if body.CheckboxText("Include parent projects", &a.prefs.ProjectParents) {
				a.savePrefs()
				a.reloadRally()
			}
			body.Row(28).Dynamic(1)
			if body.CheckboxText("Include child projects", &a.prefs.ProjectChildren) {
				a.savePrefs()
				a.reloadRally()
			}
		case "Models":
			muted(body, "The model catalog and provider come from your installed Codex CLI.", a.p)
			muted(body, "Supported baseline: Codex CLI 0.162.0 or newer.", a.p)
			found := false
			for _, m := range a.catalog.Models {
				body.Row(28).Dynamic(1)
				body.Label(m.Name+"  ·  "+m.Model, "LC")
				speeds := []string{}
				for _, tier := range a.catalog.Speeds(m) {
					speeds = append(speeds, tier.Name)
					if strings.Contains(m.Model, "gpt-6.1-sol") && tier.ID == "ultrafast" {
						found = true
					}
				}
				muted(body, "Speed: "+strings.Join(speeds, ", "), a.p)
			}
			if !found {
				body.Row(65).Dynamic(1)
				body.LabelWrap("gpt-6.1-sol Ultrafast is not advertised by this Codex installation/provider. Update Codex or check its Bedrock configuration. Other available models and speeds remain usable.")
			}
			body.Row(30).Static(180)
			if body.ButtonText("Reload catalog") {
				if a.client != nil {
					c := a.client
					cwd := a.prefs.WorkingDirectory
					a.work(func() { a.loadCatalog(c, cwd) })
				}
			}
		case "Codex configuration":
			muted(body, "Edits are sent to Codex's own configuration API and also apply to Codex CLI.", a.p)
			a.field(body, "Setting path (e.g. model or model_reasoning_effort)", s.ConfigKey, false)
			a.field(body, "JSON value (strings need quotes)", s.ConfigValue, false)
			body.Row(30).Static(160, 160)
			if body.ButtonText("Apply setting") {
				var value any
				if e := json.Unmarshal([]byte(text(s.ConfigValue)), &value); e != nil {
					a.report(e)
				} else {
					a.rpc("config/value/write", map[string]any{"keyPath": text(s.ConfigKey), "value": value, "mergeStrategy": "replace"}, func(_ json.RawMessage) {
						a.toast = "Saved Codex setting"
						a.loadSettingsPage(s.Page)
						if a.client != nil {
							c := a.client
							cwd := a.prefs.WorkingDirectory
							a.work(func() { a.loadCatalog(c, cwd) })
						}
					})
				}
			}
			if body.ButtonText("Refresh configuration") {
				a.loadSettingsPage(s.Page)
			}
			body.Row(max(120, body.LayoutAvailableHeight()-10)).Dynamic(1)
			s.Output.Edit(body)
		case "Keyboard":
			for _, line := range []string{"Ctrl/Cmd+T — New tab", "Ctrl/Cmd+W — Close tab", "Ctrl/Cmd+, — Settings", "Ctrl/Cmd+Shift+P — Command palette", "Ctrl/Cmd+Shift+T — Toggle dark / light", "Ctrl/Cmd+R — Refresh Rally", "Ctrl/Cmd+S — Save work item", "Ctrl+Tab — Next tab", "Ctrl+Shift+Tab — Previous tab", "Enter — Send or queue", "Shift+Enter — New line", "Escape — Close palette"} {
				muted(body, line, a.p)
			}
		case "About":
			muted(body, "Fastrock 1.0 · Go + nucular", a.p)
			body.Row(70).Dynamic(1)
			body.LabelWrap("Independent desktop client for Codex and Rally. Not an official Broadcom or OpenAI application. No Fastrock account, telemetry, embedded browser, or bundled Codex runtime.")
			muted(body, a.status, a.p)
			body.Row(30).Static(190)
			if body.ButtonText("Source and licenses") {
				a.openURL("https://github.com/allquixotic/fastrock")
			}
		default:
			body.Row(30).Static(110)
			if body.ButtonText("Refresh") {
				a.loadSettingsPage(s.Page)
			}
			body.Row(max(200, body.LayoutAvailableHeight()-10)).Dynamic(1)
			s.Output.Edit(body)
		}
		body.GroupEnd()
	}
}
func (a *App) loadSettingsPage(page string) {
	methods := map[string]string{"Codex configuration": "config/read", "Account": "account/read", "MCP servers": "mcpServerStatus/list", "Skills": "skills/list"}
	method := methods[page]
	if method == "" {
		return
	}
	params := map[string]any{}
	if method == "config/read" {
		params["includeLayers"] = false
	}
	if method == "skills/list" {
		params["cwds"] = []string{a.prefs.WorkingDirectory}
	}
	a.rpc(method, params, func(raw json.RawMessage) {
		var data any
		_ = json.Unmarshal(raw, &data)
		scrub(data)
		pretty, _ := json.MarshalIndent(data, "", "  ")
		if a.settingsView != nil {
			setText(a.settingsView.Output, string(pretty))
		}
	})
}
func scrub(v any) {
	switch x := v.(type) {
	case map[string]any:
		for k, v := range x {
			l := strings.ToLower(k)
			if strings.Contains(l, "token") || strings.Contains(l, "secret") || strings.Contains(l, "password") || strings.Contains(l, "api_key") {
				x[k] = "[redacted]"
			} else {
				scrub(v)
			}
		}
	case []any:
		for _, v := range x {
			scrub(v)
		}
	}
}
func (a *App) scopePicker(w *nucular.Window, label string, items []rally.Object, selected *string, changed func()) {
	title(w, label, a.p)
	names := []string{"All"}
	refs := []string{""}
	for _, o := range items {
		names = append(names, o.String("Name"))
		refs = append(refs, o.String("_ref"))
	}
	w.Row(30).Dynamic(1)
	old := index(refs, *selected)
	next := w.ComboSimple(names, old, 28)
	if old != next {
		*selected = refs[next]
		a.savePrefs()
		changed()
		a.reloadRally()
	}
}
func (a *App) reloadRally() {
	for _, v := range a.rallyViews {
		a.refreshRally(v)
	}
}

var _ = fmt.Sprintf
