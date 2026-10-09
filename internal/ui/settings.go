package ui

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/buildinfo"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

type settingsView struct {
	ReloadTimer                 *time.Timer
	MCP                         *mcpForm
	Bedrock                     *bedrockForm
	ConfigData                  map[string]any
	ConfigLayer                 int
	Layers                      []configLayer
	ConfigContext               *desktop.TextEditor
	PluginSearch                *desktop.TextEditor
	ConfigFolders               []string
	PluginCatalog               bool
	Locals                      []*localProviderView
	LocalLoadGeneration         uint64
	Fields                      []configField
	Search, Raw                 *desktop.TextEditor
	RawPath, ConfigVersion      string
	RawHash                     [32]byte
	afterRawSave                func()
	Name, Value, Secret, Region *desktop.TextEditor
	Items                       []settingsItem
	Selected                    map[string]bool
	LoginID, LoginCode          string
	LoginURL, LoginError        string
	LoginBusy                   bool
	LoginGeneration             uint64
	LoadGeneration              int
	LoadError                   string
	MCPLive                     map[string]mcpLiveStatus
	IncludeLogs                 bool
	ConfigQueue                 []configEditRequest
	ConfigWriting               bool
	ConfigFeedback              map[string]string
	ConfigFailed                map[string]bool
	ActionFeedback              map[string]settingFeedback
	ActionRequest               map[string]uint64

	Page                                                                string
	Endpoint, Token, Workspace, Project, ConfigKey, ConfigValue, Output *desktop.TextEditor
	Busy                                                                bool
	MemoryResetting                                                     bool
	RawFeedback                                                         settingFeedback
}

func newSettingsView(p settings.Preferences) *settingsView {
	secret := textEditor("", false)
	secret.PasswordChar = '●'
	return &settingsView{Search: textEditor("", false), PluginSearch: textEditor("", false), Raw: textEditor("", true), Name: textEditor("", false), Value: textEditor("", true), Secret: secret, Region: textEditor("us-east-1", false), Selected: map[string]bool{}, Page: "Common", Endpoint: textEditor(p.RallyEndpoint, false), Token: &desktop.TextEditor{Flags: desktop.EditSimple, PasswordChar: '●'}, Workspace: textEditor(p.RallyWorkspace, false), Project: textEditor(p.RallyProject, false), ConfigKey: textEditor("", false), ConfigValue: textEditor("", false), Output: textEditor("", true)}
}
func (a *App) drawSettings(w *desktop.Window) {
	s := a.settingsView
	if s == nil {
		s = newSettingsView(a.prefs)
		a.settingsView = s
		a.loadSettingsPage(s.Page)
	}
	title(w, "Settings", a.p)
	a.drawSettingsServer(w)
	w.Row(max(200, w.LayoutAvailableHeight()-10)).Static(170, max(300, w.LayoutAvailableWidth()-180))
	if nav := w.GroupBegin("settings-nav", desktop.WindowNoHScrollbar); nav != nil {
		a.drawSettingsNavigation(nav, s)
		nav.GroupEnd()
	}
	if body := w.GroupBegin("settings-content", desktop.WindowNoHScrollbar); body != nil {
		title(body, s.Page, a.p)
		muted(body, settingsSubtitle(s.Page), a.p)
		a.drawPageFeedback(body, s)
		switch s.Page {
		case "Appearance":
			body.Row(30).Static(100, 100, 100)
			if button(body, "System", a.prefs.Theme == "system", a.p) {
				a.prefs.Theme = "system"
				a.theme()
			}
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
			sizes := []string{}
			for size := 10; size <= 24; size++ {
				sizes = append(sizes, strconv.Itoa(size))
			}
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
			muted(body, "Ctrl/Cmd+Enter always sends.", a.p)
			body.Row(30).Dynamic(1)
			if body.CheckboxText("Show conversations sidebar", &a.prefs.Sidebar) {
				a.savePrefs()
			}
			body.Row(30).Dynamic(1)
			if body.CheckboxText("Show conversation information", &a.prefs.Info) {
				a.savePrefs()
			}
			title(body, "When sending during a running turn", a.p)
			body.Row(30).Dynamic(2)
			if button(body, "Queue", a.prefs.BusyInput != "steer", a.p) {
				a.prefs.BusyInput = "queue"
				a.savePrefs()
			}
			if button(body, "Steer", a.prefs.BusyInput == "steer", a.p) {
				a.prefs.BusyInput = "steer"
				a.savePrefs()
			}
			body.Row(30).Dynamic(1)
			if body.CheckboxText("Agent messages between conversations", &a.prefs.AgentMessages) {
				a.savePrefs()
			}
			muted(body, "Applies to conversations started from now on. Deliveries still need your approval.", a.p)
		case "Rally":
			a.field(body, "Rally endpoint", s.Endpoint, false)
			a.field(body, "API token (stored in OS credential store)", s.Token, false)
			muted(body, "Leave token empty to keep the existing saved token.", a.p)
			body.Row(32).Static(170, 170)
			if !s.Busy && primary(body, "Save and connect", a.p) {
				s.Busy = true
				setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Pending: true, Message: "Saving connection…"})
				endpoint := strings.TrimRight(strings.TrimSpace(text(s.Endpoint)), "/")
				token := text(s.Token)
				if _, err := rally.New(endpoint, "validation", nil); err != nil {
					s.Busy = false
					setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Failed: true, Message: err.Error()})
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
							setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Failed: true, Message: e.Error()})
							return
						}
						a.prefs.RallyEndpoint = endpoint
						setText(s.Token, "")
						a.rallyClient = nil
						a.savePrefs()
						a.connectRally()
						setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Message: "Connection saved"})
					})
				}, func() {
					s.Busy = false
					setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Failed: true, Message: errWorkQueueFull.Error()})
				})
			}
			if !s.Busy && body.ButtonText("Forget saved token") {
				s.Busy = true
				setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Pending: true, Message: "Removing saved token…"})
				endpoint := a.prefs.RallyEndpoint
				a.work(func() {
					e := a.store.SetToken(endpoint, "")
					a.post(func() {
						s.Busy = false
						if e != nil {
							setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Failed: true, Message: e.Error()})
							return
						}
						setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Message: "Saved token removed"})
						a.rallyClient = nil
						a.rallyErr = "Connect your Rally endpoint and API token in Settings."
					})
				}, func() {
					s.Busy = false
					setSettingsFeedback(s, "page:Rally:connection", settingFeedback{Failed: true, Message: errWorkQueueFull.Error()})
				})
			}
			if a.rallyErr != "" {
				a.drawSettingsError(body, a.rallyErr)
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
		case "Local providers":
			a.drawLocalProviders(body, s)
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
				a.refreshCatalog()
			}
		case "Codex configuration":
			a.drawConfiguration(body, s)
		case "Common":
			a.drawCommonSettings(body, s)
		case "Memories":
			a.drawMemorySettings(body, s)
		case "Raw configuration":
			a.drawRawConfig(body, s)

		case "Keyboard":
			a.keyboardSettings(body)

		case "About":
			body.Row(30).Static(190)
			if body.ButtonText("Check for updates…") {
				a.showUpdates()
			}
			muted(body, "Fastrock "+buildinfo.Version+" · Go + FLTK", a.p)
			body.Row(70).Dynamic(1)
			body.LabelWrap("Independent desktop client for Codex and Rally. Not an official Broadcom or OpenAI application. No Fastrock account, telemetry, embedded browser, or bundled Codex runtime.")
			muted(body, a.status, a.p)
			body.Row(30).Static(190)
			if body.ButtonText("Source and licenses") {
				a.openURL("https://github.com/allquixotic/fastrock")
			}
		default:
			a.drawExtraSettings(body, s)
		}
		body.GroupEnd()
	}
}
func (a *App) loadSettingsPage(page string) {
	if configurationPage(page) {
		a.prepareConfigFolders()
	}
	if (configurationPage(page) || page == "Features") && !a.catalog.PolicyLoaded && !a.policyLoading {
		a.refreshSignInPolicy()
	}
	if page == "Account" {
		a.refreshAccount()
		a.refreshUsage()
		a.refreshSignInPolicy()
		return
	}
	if page == "Local providers" {
		a.loadLocalProvider()
		return
	}
	if page == "Raw configuration" {
		if a.settingsView.RawPath != "" {
			return
		}
		a.loadRawConfig()
		return
	}
	methods := map[string]string{"Codex configuration": "config/read", "Account": "account/read", "MCP servers": "mcpServerStatus/list", "Skills": "skills/list", "Plugins": "plugin/installed", "Hooks": "hooks/list", "Features": "experimentalFeature/list", "Memories": "config/read", "Common": "config/read", "Import": "externalAgentConfig/detect", "Sandbox": "windowsSandbox/readiness", "Diagnostics": "config/read", "AWS Bedrock": "account/bedrock/discover"}
	method := methods[page]
	if page == "Plugins" && a.settingsView.PluginCatalog {
		method = "plugin/list"
	}
	if method == "" {
		return
	}
	params := map[string]any{}
	if method == "plugin/list" || method == "plugin/installed" {
		params["cwds"] = []string{a.prefs.WorkingDirectory}
	}
	if method == "config/read" {
		params["includeLayers"] = true
		params["cwd"] = a.prefs.WorkingDirectory
		if c := a.settingsView.ConfigContext; c != nil && text(c) != "" {
			params["cwd"] = text(c)
		}
	}
	if method == "skills/list" {
		a.skillCache.invalidate()
	}
	if method == "skills/list" || method == "hooks/list" || method == "externalAgentConfig/detect" {
		params["cwds"] = []string{a.prefs.WorkingDirectory}
	}
	if method == "externalAgentConfig/detect" {
		params["includeHome"] = true
		params["migrationSource"] = text(a.settingsView.Name)
	}
	a.loadSettingsData(page, method, params)
}

func scrub(v any) {
	switch x := v.(type) {
	case map[string]any:
		for k, v := range x {
			l := strings.ReplaceAll(strings.ToLower(k), "_", "")
			if strings.Contains(l, "token") || strings.Contains(l, "secret") || strings.Contains(l, "password") || strings.Contains(l, "apikey") || strings.Contains(l, "authorization") {
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
func (a *App) scopePicker(w *desktop.Window, label string, items []rally.Object, selected *string, changed func()) {
	title(w, label, a.p)
	names, refs, old := a.picker(label, items).options("All", *selected)
	w.Row(30).Dynamic(1)
	next := w.ComboSimple(names, old, 28)
	if old != next {
		*selected = refs[next]
		a.savePrefs()
		changed()
		a.reloadRally()
	}
}
func (a *App) reloadRally() {
	for id, v := range a.rallyViews {
		if a.state != nil && a.state.Active == id {
			a.refreshRally(v)
		} else {
			if v.cancel != nil {
				v.cancel()
			}
			v.Generation++
			v.Loading = false
			v.Evicted = true
		}
	}
}

var _ = fmt.Sprintf
