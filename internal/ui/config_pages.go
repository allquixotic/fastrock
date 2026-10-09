package ui

import (
	"encoding/json"
	"strconv"

	"github.com/aarzilli/nucular"
)

func configurationPage(page string) bool {
	return page == "Codex configuration" || page == "Memories" || page == "Common"
}

func commonConfigKey(key string) bool {
	switch key {
	case "model", "model_provider", "model_reasoning_effort", "model_context_window", "approval_policy", "sandbox_mode", "web_search", "model_reasoning_summary", "model_verbosity", "notify":
		return true
	}
	return false
}

func memoryValue(field *configField) bool {
	if draft := text(field.Editor); draft != field.Baseline {
		if value, err := strconv.ParseBool(draft); err == nil {
			return value
		}
	}
	if value, ok := field.Value.(bool); ok {
		return value
	}
	// These two settings default on in Codex even when omitted from TOML.
	return field.Key == "memories.use_memories" || field.Key == "memories.generate_memories"
}

func (a *App) drawMemorySettings(w *nucular.Window, s *settingsView) {
	if s.Busy {
		muted(w, "Loading memory settings…", a.p)
		return
	}
	memoryEnabled := false
	for _, row := range []struct{ key, title, description string }{
		{"features.memories", "Memories", "Let Codex remember context from your conversations."},
		{"memories.use_memories", "Use memories", "Bring remembered context into new conversations."},
		{"memories.generate_memories", "Generate memories", "Create memories after conversations become idle."},
	} {
		var field *configField
		for i := range s.Fields {
			if s.Fields[i].Key == row.key {
				field = &s.Fields[i]
				break
			}
		}
		if field == nil {
			muted(w, row.title+": unavailable in this Codex configuration", a.p)
			continue
		}
		on := memoryValue(field)
		origin, locked := configOrigin(s.ConfigData, row.key)
		if row.key == "features.memories" {
			if required, exists := a.catalog.Requirements["memories"]; exists {
				on, locked, origin = required, true, "organization policy"
			}
			memoryEnabled = on
		}
		w.Row(28).Dynamic(1)
		if locked {
			w.Label(row.title+" · "+strconv.FormatBool(on)+" (locked by "+origin+")", "LC")
		} else if w.CheckboxText(row.title, &on) {
			setText(field.Editor, strconv.FormatBool(on))
			a.configWrite(row.key, on)
		}
		muted(w, row.description, a.p)
		a.configFeedback(w, s, row.key)
	}
	if !memoryEnabled {
		muted(w, "Use and generate settings take effect when Memories is turned on.", a.p)
	}
	w.Row(30).Dynamic(2)
	if s.MemoryResetting {
		w.Label("Resetting memories…", "LC")
	} else if w.ButtonText("Reset all memories…") {
		a.confirm("Reset all memories?", "Delete Codex memory files and thread summaries? Your conversations are kept. This cannot be undone.", func() {
			if s.MemoryResetting {
				return
			}
			s.MemoryResetting = true
			key := "page:Memories:reset"
			setSettingsFeedback(s, key, settingFeedback{Pending: true, Message: "Resetting memories…"})
			a.rpcInline("memory/reset", nil, func(json.RawMessage) {
				s.MemoryResetting = false
				setSettingsFeedback(s, key, settingFeedback{Message: "All memories were reset"})
			}, func(err error) {
				s.MemoryResetting = false
				setSettingsFeedback(s, key, settingFeedback{Failed: true, Message: err.Error()})
			})
		})
	}
	if w.ButtonText("Learn more") {
		a.openURL("https://developers.openai.com/codex/memories")
	}
}

func (a *App) drawCommonSettings(w *nucular.Window, s *settingsView) {
	if s.ConfigContext == nil {
		s.ConfigContext = textEditor(a.prefs.WorkingDirectory, false)
	}
	a.drawConfiguration(w, s)
}
