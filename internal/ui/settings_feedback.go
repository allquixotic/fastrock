package ui

import (
	"encoding/json"
	"sort"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
)

type settingFeedback struct {
	Pending, Failed bool
	Message         string
}

func settingsRowKey(page, id string) string { return "row:" + page + ":" + id }

func (a *App) settingsRequestAt(key, method string, params any) {
	s := a.settingsView
	if s.ActionFeedback == nil {
		s.ActionFeedback = map[string]settingFeedback{}
	}
	if s.ActionFeedback[key].Pending {
		return
	}
	if a.client == nil || a.serverPaused {
		s.ActionFeedback[key] = settingFeedback{Failed: true, Message: "Codex is not running. Reconnect, then retry."}
		return
	}
	live := nextSettingAction(s, key)
	page, client, generation := s.Page, a.client, a.serverGeneration
	notice := "Saved"
	s.ActionFeedback[key] = settingFeedback{Pending: true, Message: "Saving…"}
	complete := func() {
		if !live() {
			return
		}
		if client != a.client || generation != a.serverGeneration {
			s.ActionFeedback[key] = settingFeedback{Failed: true, Message: "Codex restarted before the change was confirmed. Reload to check its value."}
			return
		}
		s.ActionFeedback[key] = settingFeedback{Message: notice}
		if a.settingsView == s && s.Page == page {
			a.loadSettingsPage(page)
		}
	}
	a.rpcInline(method, params, func(raw json.RawMessage) {
		if !live() {
			return
		}
		if method == "skills/config/write" {
			var result struct {
				Effective *bool `json:"effectiveEnabled"`
			}
			if json.Unmarshal(raw, &result) == nil && result.Effective != nil {
				if desired, ok := object(params)["enabled"].(bool); ok && desired != *result.Effective {
					notice = "Saved, but a higher-precedence setting keeps this skill " + map[bool]string{true: "on.", false: "off."}[*result.Effective]
				}
			}
		}
		if page == "MCP servers" && (method == "config/value/write" || method == "config/batchWrite") {
			if client != a.client || generation != a.serverGeneration {
				complete()
				return
			}
			a.rpcInline("config/mcpServer/reload", map[string]any{}, func(json.RawMessage) { complete() }, func(err error) {
				if live() {
					s.ActionFeedback[key] = settingFeedback{Failed: true, Message: "Configuration saved, but servers could not reload: " + err.Error()}
				}
			})
			return
		}
		complete()
	}, func(err error) {
		if live() {
			s.ActionFeedback[key] = settingFeedback{Failed: true, Message: err.Error()}
		}
	})
}

func (a *App) drawSettingFeedback(w *desktop.Window, feedback settingFeedback) {
	if feedback.Message == "" {
		return
	}
	if feedback.Failed {
		a.drawSettingsError(w, feedback.Message)
	} else {
		muted(w, feedback.Message, a.p)
	}
}
func (a *App) drawPageFeedback(w *desktop.Window, s *settingsView) {
	keys := []string{}
	for key := range s.ActionFeedback {
		if strings.HasPrefix(key, "page:"+s.Page+":") {
			keys = append(keys, key)
		}
	}
	sort.Strings(keys)
	for _, key := range keys {
		a.drawSettingFeedback(w, s.ActionFeedback[key])
	}
}
func (a *App) configFeedback(w *desktop.Window, s *settingsView, key string) {
	a.drawSettingFeedback(w, settingFeedback{Message: s.ConfigFeedback[key], Failed: s.ConfigFailed[key]})
}

func setSettingsFeedback(s *settingsView, key string, feedback settingFeedback) {
	if s.ActionFeedback == nil {
		s.ActionFeedback = map[string]settingFeedback{}
	}
	s.ActionFeedback[key] = feedback
}
func (a *App) settingsFormCall(action, method string, params any, done func(json.RawMessage)) {
	s := a.settingsView
	key := "page:" + s.Page + ":" + action
	if s.ActionFeedback[key].Pending {
		return
	}
	if a.client == nil || a.serverPaused {
		setSettingsFeedback(s, key, settingFeedback{Failed: true, Message: "Codex is not running. Reconnect, then retry."})
		return
	}
	live := nextSettingAction(s, key)
	client, generation := a.client, a.serverGeneration
	setSettingsFeedback(s, key, settingFeedback{Pending: true, Message: "Saving…"})
	a.rpcInline(method, params, func(raw json.RawMessage) {
		if !live() {
			return
		}
		if client != a.client || generation != a.serverGeneration {
			setSettingsFeedback(s, key, settingFeedback{Failed: true, Message: "Codex restarted before the change was confirmed. Reload to check."})
			return
		}
		setSettingsFeedback(s, key, settingFeedback{Message: "Saved"})
		if done != nil {
			done(raw)
		}
	}, func(err error) {
		if live() {
			setSettingsFeedback(s, key, settingFeedback{Failed: true, Message: err.Error()})
		}
	})
}

func nextSettingAction(s *settingsView, key string) func() bool {
	if s.ActionRequest == nil {
		s.ActionRequest = map[string]uint64{}
	}
	s.ActionRequest[key]++
	request := s.ActionRequest[key]
	return func() bool { return s.ActionRequest[key] == request }
}
