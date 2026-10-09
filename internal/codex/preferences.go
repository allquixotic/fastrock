package codex

import "github.com/allquixotic/fastrock/internal/settings"

type PreferencesState struct {
	Revision uint64               `json:"revision"`
	Data     settings.Preferences `json:"data"`
}

func (b *Broker) SetPreferences(p settings.Preferences, save func(settings.Preferences) error) {
	b.preferencesMu.Lock()
	defer b.preferencesMu.Unlock()
	b.preferences, b.savePreferences = p, save
}
