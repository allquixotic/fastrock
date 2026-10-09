package ui

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/pelletier/go-toml/v2"
)

type configField struct {
	Spec      *configSpec
	Search    string
	Key, Kind string
	Value     any
	Editor    *nucular.TextEditor
}
type configLayer struct {
	Label, Path, Disabled string
	Fields                []configField
}

func configLayers(data map[string]any) []configLayer {
	var result []configLayer
	layers, _ := data["layers"].([]any)
	for _, value := range layers {
		l, _ := value.(map[string]any)
		n, _ := l["name"].(map[string]any)
		path := str(n, "file")
		if path == "" && str(n, "dotCodexFolder") != "" {
			path = filepath.Join(str(n, "dotCodexFolder"), "config.toml")
		}
		label := str(n, "type")
		if profile := str(n, "profile"); profile != "" {
			label += " · " + profile
		}
		if path != "" {
			label += " · " + path
		}
		conf, _ := l["config"].(map[string]any)
		result = append(result, configLayer{Label: label, Path: path, Disabled: str(l, "disabledReason"), Fields: configFields(conf)})
	}
	return result
}

func configOrigin(data map[string]any, key string) (string, bool) {
	origins, _ := data["origins"].(map[string]any)
	for {
		if origin, ok := origins[key].(map[string]any); ok {
			n, _ := origin["name"].(map[string]any)
			kind := str(n, "type")
			locked := kind == "project" || kind == "sessionFlags" || strings.Contains(strings.ToLower(kind), "managed") || kind == "mdm"
			return kind, locked
		}
		i := strings.LastIndexByte(key, '.')
		if i < 0 {
			break
		}
		key = key[:i]
	}
	return "default", false
}

// Key paths use TOML syntax; quoted segments keep literal dots in server names.
func configKey(s string) string {
	if s != "" && strings.IndexFunc(s, func(r rune) bool {
		return !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '_' || r == '-')
	}) == -1 {
		return s
	}
	return strconv.Quote(s)
}
func configFields(v map[string]any) []configField {
	fields := []configField{}
	var visit func(string, any)
	visit = func(path string, value any) {
		if m, ok := value.(map[string]any); ok {
			for k, v := range m {
				key := configKey(k)
				if path != "" {
					key = path + "." + configKey(k)
				}
				visit(key, v)
			}
			return
		}
		if value == "[redacted]" {
			return
		}
		kind := "json"
		display := ""
		switch x := value.(type) {
		case string:
			kind = "string"
			display = x
		case bool:
			kind = "bool"
			display = strconv.FormatBool(x)
		default:
			b, _ := json.Marshal(value)
			display = string(b)
		}
		fields = append(fields, configField{Key: path, Kind: kind, Value: value, Editor: textEditor(display, false)})
	}
	visit("", v)
	sort.Slice(fields, func(i, j int) bool { return fields[i].Key < fields[j].Key })
	return fields
}
func (a *App) configWrite(key string, value any) {
	s := a.settingsView
	params := map[string]any{"edits": []map[string]any{{"keyPath": key, "value": value, "mergeStrategy": "replace"}}, "reloadUserConfig": true}
	if s.ConfigVersion != "" {
		params["expectedVersion"] = s.ConfigVersion
	}
	a.rpc("config/batchWrite", params, func(raw json.RawMessage) {
		a.toast = "Saved " + key
		var response struct{ OverriddenMetadata *struct{ Message string } }
		_ = json.Unmarshal(raw, &response)
		if response.OverriddenMetadata != nil {
			a.toast = response.OverriddenMetadata.Message
		}
		a.loadSettingsPage(s.Page)
	})
}
func (a *App) drawConfiguration(w *nucular.Window, s *settingsView) {
	if s.ConfigContext == nil {
		s.ConfigContext = textEditor(a.prefs.WorkingDirectory, false)
	}
	w.Row(28).Ratio(.8, .2)
	s.ConfigContext.Edit(w)
	if w.ButtonText("Use context") {
		a.loadSettingsPage(s.Page)
	}
	labels := []string{"Effective configuration"}
	for _, l := range s.Layers {
		labels = append(labels, l.Label)
	}
	w.Row(28).Dynamic(1)
	s.ConfigLayer = w.ComboSimple(labels, min(s.ConfigLayer, len(labels)-1), 28)
	fields := s.Fields
	if s.ConfigLayer > 0 {
		l := s.Layers[s.ConfigLayer-1]
		fields = l.Fields
		if l.Disabled != "" {
			muted(w, "Disabled: "+l.Disabled, a.p)
		}
		if l.Path != "" {
			w.Row(28).Static(150)
			if w.ButtonText("Open layer file") {
				a.openFile(l.Path)
			}
		}
	}
	w.Row(30).Ratio(.7, .15, .15)
	s.Search.Edit(w)
	if w.ButtonText("Reload") {
		a.loadSettingsPage(s.Page)
	}
	if w.ButtonText("Raw TOML") {
		a.settingsPage("Raw configuration")
	}
	needle := strings.ToLower(text(s.Search))
	offset := 150
	for i := range fields {
		f := &fields[i]
		search := f.Search
		if search == "" {
			search = strings.ToLower(f.Key)
		}
		if !strings.Contains(search, needle) {
			continue
		}
		// Skip offscreen controls while retaining their scroll extent.
		const rowHeight = 95
		visible := offset+rowHeight >= w.Scrollbar.Y-200 && offset <= w.Scrollbar.Y+w.Bounds.H+200
		offset += rowHeight
		if !visible {
			w.Row(rowHeight).Dynamic(1)
			w.Spacing(1)
			continue
		}
		origin, locked := configOrigin(s.ConfigData, f.Key)
		w.Row(25).Ratio(.85, .15)
		w.Label(f.Key, "LC")
		if w.ButtonText("Info") {
			a.configFieldHelp(f, origin)
		}
		if s.ConfigLayer > 0 {
			locked = true
		}
		muted(w, "Source: "+origin, a.p)
		if locked {
			w.Row(28).Dynamic(1)
			w.Label(text(f.Editor), "LC")
			continue
		}
		w.Row(30).Ratio(.72, .14, .14)
		if f.Kind == "bool" {
			on, _ := f.Value.(bool)
			if w.CheckboxText("Enabled", &on) {
				a.configWrite(f.Key, on)
			}
		} else if f.Spec != nil && len(f.Spec.Choices) > 0 {
			choices := f.Spec.Options
			current := 0
			for i, choice := range f.Spec.Choices {
				if choice == text(f.Editor) {
					current = i + 1
				}
			}
			selected := w.ComboSimple(choices, current, 28)
			if selected != current {
				if selected == 0 {
					a.configWrite(f.Key, nil)
				} else {
					setText(f.Editor, choices[selected])
				}
			}
		} else {
			f.Editor.Edit(w)
		}
		if f.Kind == "bool" {
			w.Label("", "LC")
		} else if w.ButtonText("Apply") {
			value, err := configFieldValue(f)
			if err != nil {
				a.report(err)
				continue
			}
			a.configWrite(f.Key, value)
		}
		if w.ButtonText("Reset") {
			a.configWrite(f.Key, nil)
		}
	}
	title(w, "Add or edit a setting", a.p)
	a.field(w, "Setting path", s.ConfigKey, false)
	a.field(w, "JSON value", s.ConfigValue, false)
	w.Row(30).Static(130)
	if w.ButtonText("Apply setting") {
		var value any
		if err := json.Unmarshal([]byte(text(s.ConfigValue)), &value); err != nil {
			a.report(err)
		} else {
			a.configWrite(text(s.ConfigKey), value)
		}
	}
}
func (a *App) loadRawConfig() {
	s := a.settingsView
	s.Busy = true
	a.work(func() {
		path := filepath.Join(codexHome(), "config.toml")
		if resolved, err := filepath.EvalSymlinks(path); err == nil {
			path = resolved
		}
		data, err := os.ReadFile(path)
		if os.IsNotExist(err) {
			err = nil
		}
		hash := sha256.Sum256(data)
		a.post(func() {
			s.Busy = false
			a.report(err)
			s.RawPath = path
			s.RawHash = hash
			setText(s.Raw, string(data))
		})
	})
}
func (a *App) saveRawConfig() {
	s := a.settingsView
	value, path, expected := text(s.Raw), s.RawPath, s.RawHash
	if path == "" {
		return
	}
	s.Busy = true
	a.work(func() {
		var parsed map[string]any
		err := toml.Unmarshal([]byte(value), &parsed)
		if err == nil {
			current, e := os.ReadFile(path)
			if e != nil && !os.IsNotExist(e) {
				err = e
			} else if sha256.Sum256(current) != expected {
				err = fmt.Errorf("config.toml changed in another program; reload before saving")
			}
		}
		if err == nil {
			f, e := os.CreateTemp(filepath.Dir(path), ".fastrock-config-*")
			err = e
			if err == nil {
				tmp := f.Name()
				defer os.Remove(tmp)
				_ = f.Chmod(0600)
				_, err = f.WriteString(value)
				if err == nil {
					err = f.Sync()
				}
				closeErr := f.Close()
				if err == nil {
					err = closeErr
				}
				if err == nil {
					err = os.Rename(tmp, path)
				}
			}
		}
		a.post(func() {
			s.Busy = false
			if err != nil {
				a.report(err)
				return
			}
			s.RawHash = sha256.Sum256([]byte(value))
			a.toast = "Configuration saved. Restart Codex to apply provider changes."
			a.rpc("config/batchWrite", map[string]any{"edits": []any{}, "reloadUserConfig": true}, nil)
		})
	})
}
func (a *App) drawRawConfig(w *nucular.Window, s *settingsView) {
	w.Row(30).Static(100, 100)
	if !s.Busy && w.ButtonText("Save") {
		a.saveRawConfig()
	}
	if w.ButtonText("Reload") {
		a.loadRawConfig()
	}
	muted(w, s.RawPath, a.p)
	w.Row(max(160, w.LayoutAvailableHeight()-5)).Dynamic(1)
	s.Raw.Edit(w)
}
