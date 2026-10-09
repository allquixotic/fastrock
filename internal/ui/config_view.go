package ui

import (
	"encoding/json"
	"sort"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
)

func (a *App) prepareConfigFolders() {
	s := a.settingsView
	if s == nil {
		return
	}
	seen := map[string]bool{}
	s.ConfigFolders = nil
	add := func(path string) {
		if path != "" && !seen[path] {
			seen[path] = true
			s.ConfigFolders = append(s.ConfigFolders, path)
		}
	}
	add(text(s.ConfigContext))
	add(a.prefs.WorkingDirectory)
	for _, path := range a.prefs.RecentFolders {
		add(path)
	}
	if a.state != nil {
		for _, tab := range a.state.Tabs {
			if c := a.state.Chats[tab.Target]; c != nil {
				add(c.Cwd)
			}
		}
	}
}
func (a *App) configError(s *settingsView, key string, err error) {
	if s.ConfigFeedback == nil {
		s.ConfigFeedback = map[string]string{}
	}
	if s.ConfigFailed == nil {
		s.ConfigFailed = map[string]bool{}
	}
	s.ConfigFeedback[key], s.ConfigFailed[key] = err.Error(), true
}
func configOriginLabel(origin string) string {
	labels := map[string]string{"default": "Default", "user": "User configuration", "project": "Project configuration", "profile": "Selected profile", "sessionFlags": "Command-line settings", "mdm": "Device management", "system": "System configuration", "cloudRequirements": "Organization requirements", "cloudManagedConfig": "Organization configuration"}
	if label := labels[origin]; label != "" {
		return label
	}
	return origin
}
func (a *App) drawConfiguration(w *desktop.Window, s *settingsView) {
	common := s.Page == "Common"
	if s.ConfigContext == nil {
		s.ConfigContext = textEditor(a.prefs.WorkingDirectory, false)
	}
	if s.ConfigFolders == nil {
		a.prepareConfigFolders()
	}
	if len(s.ConfigFolders) > 0 {
		selected := index(s.ConfigFolders, text(s.ConfigContext))
		if selected < 0 {
			selected = 0
		}
		w.Row(28).Dynamic(1)
		if choice := w.ComboSimple(s.ConfigFolders, selected, 28); choice != selected {
			setText(s.ConfigContext, s.ConfigFolders[choice])
			a.loadSettingsPage(s.Page)
		}
	}
	w.Row(28).Ratio(.8, .2)
	s.ConfigContext.Edit(w)
	if w.ButtonText("Use context") {
		a.loadSettingsPage(s.Page)
	}
	if !common {
		labels := []string{"Effective configuration"}
		for _, layer := range s.Layers {
			labels = append(labels, layer.Label)
		}
		w.Row(28).Dynamic(1)
		s.ConfigLayer = w.ComboSimple(labels, min(s.ConfigLayer, len(labels)-1), 28)
	}
	fields := s.Fields
	if !common && s.ConfigLayer > 0 {
		layer := s.Layers[s.ConfigLayer-1]
		fields = layer.Fields
		if layer.Disabled != "" {
			muted(w, "Disabled: "+layer.Disabled, a.p)
		}
		if layer.Path != "" {
			w.Row(28).Static(150)
			if w.ButtonText("Open layer file") {
				a.openFile(layer.Path)
			}
		}
	}
	w.Row(30).Ratio(.7, .15, .15)
	s.Search.Placeholder = "Search settings"
	s.Search.Edit(w)
	if w.ButtonText("Reload") {
		a.loadSettingsPage(s.Page)
	}
	if w.ButtonText("Raw TOML") {
		a.settingsPage("Raw configuration")
	}
	if s.LoadError != "" {
		a.drawSettingsError(w, s.LoadError)
	}
	if common && (!a.catalog.PolicyLoaded || a.policyLoading) {
		muted(w, "Loading organization policy before editing Common settings…", a.p)
		if a.policyError != "" {
			a.drawSettingsError(w, a.policyError)
		}
		w.Row(28).Static(150)
		if w.ButtonText("Reload policy") {
			a.refreshSignInPolicy()
		}
		return
	}
	needle := strings.ToLower(strings.TrimSpace(text(s.Search)))
	group := ""
	matched := 0
	for i := range fields {
		field := &fields[i]
		if common && !commonConfigKey(field.Key) {
			continue
		}
		search := field.Search
		if search == "" {
			search = strings.ToLower(field.Key)
		}
		if !strings.Contains(search, needle) {
			continue
		}
		matched++
		if next := configGroup(field.Key); next != group {
			group = next
			title(w, group, a.p)
		}
		height := 210
		if field.Kind == "toml" {
			height = 330
		}
		w.Row(height).Dynamic(1)
		bounds := w.WidgetBounds()
		if bounds.Y+bounds.H < w.Bounds.Y-100 || bounds.Y > w.Bounds.Y+w.Bounds.H+100 {
			w.Spacing(1)
			continue
		}
		if row := w.GroupBegin("config:"+field.Key, desktop.WindowNoScrollbar); row != nil {
			a.drawConfigField(row, s, field, !common && s.ConfigLayer > 0)
			row.GroupEnd()
		}
	}
	if matched == 0 {
		muted(w, "No settings match your search.", a.p)
	}
	if common {
		return
	}
	title(w, "Add or edit a setting", a.p)
	a.field(w, "Setting path", s.ConfigKey, false)
	a.field(w, "JSON value", s.ConfigValue, false)
	key := text(s.ConfigKey)
	a.configFeedback(w, s, key)
	w.Row(30).Static(130)
	if w.ButtonText("Apply setting") {
		var value any
		if err := json.Unmarshal([]byte(text(s.ConfigValue)), &value); err != nil {
			a.configError(s, key, err)
		} else {
			a.configWrite(key, value)
		}
	}
}
func (a *App) drawConfigField(w *desktop.Window, s *settingsView, f *configField, layerReadOnly bool) {
	origin, locked := configFieldOrigin(s.ConfigData, f)
	allowed, constrained := a.catalog.Policy.AllowedValues(f.Key)
	if constrained && (len(allowed) == 0 || f.Key == "model_provider") {
		origin, locked = "organization policy", true
	}
	if strings.HasPrefix(f.Key, "features.") {
		if _, managed := a.catalog.Requirements[strings.TrimPrefix(f.Key, "features.")]; managed {
			origin, locked = "organization policy", true
		}
	}
	if policyControlledKey(f.Key) && (!a.catalog.PolicyLoaded || a.policyLoading) {
		origin, locked = "policy still loading", true
	}
	w.Row(26).Dynamic(1)
	w.Label(f.Key, "LC")
	source := "Source: " + configOriginLabel(origin)
	switch {
	case f.Protected:
		locked = true
		source += " · Contains protected values; edit config.toml directly."
	case layerReadOnly:
		locked = true
		source += " · Layer inspection is read-only."
	case locked:
		source += " · Locked by " + configOriginLabel(origin) + "."
	}
	muted(w, source, a.p)
	if f.Spec != nil && f.Spec.Description != "" {
		w.Row(48).Dynamic(1)
		w.LabelWrap(cut(f.Spec.Description, 280))
	}
	if locked {
		w.Row(32).Dynamic(1)
		w.LabelWrap(cut(text(f.Editor), 180))
		a.configFeedback(w, s, f.Key)
		return
	}
	if f.Kind == "toml" {
		w.Row(135).Dynamic(1)
		codeEditor(w, f.Editor)
		muted(w, "TOML snippet for "+configLeaf(f.Key)+"; leave blank to remove the override.", a.p)
		w.Row(28).Dynamic(2)
	} else {
		w.Row(30).Ratio(.72, .14, .14)
		switch {
		case f.Kind == "bool":
			on, _ := f.Value.(bool)
			if w.CheckboxText("Enabled", &on) {
				a.configWrite(f.Key, on)
			}
		case f.Spec != nil && len(f.Spec.Choices) > 0:
			choices, values := configChoices(f, allowed, constrained)
			current := 0
			for i, choice := range values {
				if choice == text(f.Editor) {
					current = i + 1
				}
			}
			if selected := w.ComboSimple(choices, current, 28); selected != current {
				if selected == 0 {
					a.configWrite(f.Key, nil)
				} else {
					setText(f.Editor, values[selected-1])
					a.configWrite(f.Key, values[selected-1])
				}
			}
		default:
			if f.Kind == "words" {
				f.Editor.Placeholder = "Command and quoted arguments"
			}
			f.Editor.Edit(w)
		}
	}
	if f.Kind == "bool" {
		w.Label("", "LC")
	} else if w.ButtonText("Apply") {
		if value, err := configFieldValue(f); err != nil {
			a.configError(s, f.Key, err)
		} else {
			a.configWrite(f.Key, value)
		}
	}
	if w.ButtonText("Reset") {
		setText(f.Editor, "")
		a.configWrite(f.Key, nil)
	}
	a.configFeedback(w, s, f.Key)
}

func configFieldOrigin(data map[string]any, f *configField) (string, bool) {
	origin, locked := configOrigin(data, f.Key)
	if locked || f.Kind != "toml" {
		return origin, locked
	}
	var keys []string
	for key := range object(data["origins"]) {
		if strings.HasPrefix(key, f.Key+".") {
			keys = append(keys, key)
		}
	}
	sort.Strings(keys)
	for _, key := range keys {
		if source, locked := configOrigin(data, key); locked {
			return source, true
		}
	}
	return origin, false
}
