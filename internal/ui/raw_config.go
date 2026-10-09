package ui

import (
	"crypto/sha256"
	"github.com/allquixotic/fastrock/internal/desktop"
)

func (s *settingsView) rawDirty() bool {
	return s != nil && s.RawPath != "" && sha256.Sum256([]byte(text(s.Raw))) != s.RawHash
}

func (a *App) leaveRawConfig(next func()) {
	s := a.settingsView
	if !s.rawDirty() {
		next()
		return
	}
	a.window.PopupOpen("Unsaved config.toml", desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(520, 200), false, func(w *desktop.Window) {
		w.Row(70).Dynamic(1)
		w.LabelWrap("Save config.toml before continuing? This draft is kept only in this window because configuration can contain credentials.")
		w.Row(30).Dynamic(3)
		if w.ButtonText("Keep editing") {
			w.Close()
		}
		if w.ButtonText("Discard") {
			setText(s.Raw, "")
			s.RawPath = ""
			w.Close()
			a.post(next)
		}
		if !s.Busy && primary(w, "Save", a.p) {
			s.afterRawSave = next
			a.saveRawConfig()
			w.Close()
		}
	})
}
