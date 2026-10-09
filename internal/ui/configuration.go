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

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/pelletier/go-toml/v2"
)

type configField struct {
	Spec      *configSpec
	Search    string
	Key, Kind string
	Value     any
	Editor    *desktop.TextEditor
	Baseline  string
	Protected bool
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
		result = append(result, configLayer{Label: label, Path: path, Disabled: str(l, "disabledReason"), Fields: typedConfigFields(conf, configSpecs())})
	}
	return result
}

func configOrigin(data map[string]any, key string) (string, bool) {
	origins, _ := data["origins"].(map[string]any)
	for {
		if origin, ok := origins[key].(map[string]any); ok {
			n, _ := origin["name"].(map[string]any)
			kind := str(n, "type")
			locked := kind == "project" || kind == "profile" || kind == "sessionFlags" || strings.Contains(strings.ToLower(kind), "managed") || kind == "mdm" || kind == "cloudRequirements"
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
		fields = append(fields, configField{Key: path, Kind: kind, Value: value, Editor: textEditor(display, false), Baseline: display})
	}
	visit("", v)
	sort.Slice(fields, func(i, j int) bool { return fields[i].Key < fields[j].Key })
	return fields
}
func (a *App) configWrite(key string, value any) {
	s := a.settingsView
	if s.ConfigFeedback == nil {
		s.ConfigFeedback = map[string]string{}
	}
	if s.ConfigFailed == nil {
		s.ConfigFailed = map[string]bool{}
	}
	s.ConfigFailed[key] = false
	if policyControlledKey(key) && (!a.catalog.PolicyLoaded || a.policyLoading) {
		s.ConfigFeedback[key] = "Organization policy is still loading. Retry after it finishes."
		s.ConfigFailed[key] = true
		return
	}
	if !a.catalog.Policy.Allows(key, value) {
		s.ConfigFeedback[key] = "This value is not permitted by organization policy."
		s.ConfigFailed[key] = true
		return
	}
	request := configEditRequest{Key: key, Value: value}
	for _, f := range s.Fields {
		if f.Key == key {
			request.Text = text(f.Editor)
		}
	}
	s.ConfigQueue = append(s.ConfigQueue, request)
	s.ConfigFeedback[key] = "Waiting to save…"
	a.nextConfigWrite(s)
}
func (a *App) loadRawConfig() {
	s := a.settingsView
	s.Busy = true
	s.RawFeedback = settingFeedback{Pending: true, Message: "Loading configuration…"}
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
			if err != nil {
				s.RawFeedback = settingFeedback{Failed: true, Message: err.Error()}
				return
			}
			s.RawFeedback = settingFeedback{}
			s.RawPath = path
			s.RawHash = hash
			setText(s.Raw, string(data))
		})
	}, func() {
		s.Busy = false
		s.RawFeedback = settingFeedback{Failed: true, Message: errWorkQueueFull.Error()}
	})
}
func (a *App) saveRawConfig() {
	s := a.settingsView
	if s.Busy {
		return
	}
	value, path, expected := text(s.Raw), s.RawPath, s.RawHash
	if path == "" {
		return
	}
	s.Busy = true
	s.RawFeedback = settingFeedback{Pending: true, Message: "Saving…"}
	a.writeWork(func() {
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
			mode := os.FileMode(0600)
			if info, e := os.Stat(path); e == nil {
				mode = info.Mode().Perm()
			} else if !os.IsNotExist(e) {
				err = e
			}
			if err == nil {
				err = settings.WriteFileAtomic(path, []byte(value), mode)
			}
		}
		a.post(func() {
			s.Busy = false
			if err != nil {
				s.afterRawSave = nil
				s.RawFeedback = settingFeedback{Failed: true, Message: err.Error()}
				return
			}
			s.RawHash = sha256.Sum256([]byte(value))
			if next := s.afterRawSave; next != nil {
				s.afterRawSave = nil
				if text(s.Raw) == value {
					next()
				} else {
					s.RawFeedback = settingFeedback{Message: "Earlier changes saved; newer edits remain open."}
					return
				}
			}
			s.RawFeedback = settingFeedback{Message: "Configuration saved."}
			a.rpcInline("config/batchWrite", map[string]any{"edits": []any{}, "reloadUserConfig": true}, func(json.RawMessage) {
				a.refreshConfiguredProvider()
			}, func(err error) {
				s.RawFeedback = settingFeedback{Failed: true, Message: "Configuration saved, but Codex could not reload it: " + err.Error()}
				a.restartNote = "config.toml was saved but Codex could not reload it. Review the configuration, then restart Codex."
			})
		})
	}, func() {
		s.Busy = false
		s.afterRawSave = nil
		s.RawFeedback = settingFeedback{Failed: true, Message: errWorkQueueFull.Error()}
	})
}
func (a *App) drawRawConfig(w *desktop.Window, s *settingsView) {
	w.Row(30).Static(100, 100)
	if !s.Busy && w.ButtonText("Save") {
		a.saveRawConfig()
	}
	if w.ButtonText("Reload") {
		a.leaveRawConfig(a.loadRawConfig)
	}
	muted(w, s.RawPath, a.p)
	a.drawSettingFeedback(w, s.RawFeedback)
	w.Row(max(160, w.LayoutAvailableHeight()-5)).Dynamic(1)
	codeEditor(w, s.Raw)
}
