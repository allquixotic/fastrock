package ui

import (
	"fmt"
	"strings"

	"github.com/aarzilli/nucular"
)

func reviewTarget(kind int, value string) (map[string]any, error) {
	value = strings.TrimSpace(value)
	target := map[string]any{"type": "uncommittedChanges"}
	if kind == 0 {
		return target, nil
	}
	if value == "" {
		return nil, fmt.Errorf("enter the review target")
	}
	switch kind {
	case 1:
		target = map[string]any{"type": "baseBranch", "branch": value}
	case 2:
		target = map[string]any{"type": "commit", "sha": value}
	case 3:
		target = map[string]any{"type": "custom", "instructions": value}
	default:
		return nil, fmt.Errorf("choose a supported review target")
	}
	return target, nil
}

func (a *App) reviewDialog(thread string) {
	kind := 0
	value := textEditor("", false)
	errText := ""
	a.window.PopupOpen("Review changes", nucular.WindowTitle|nucular.WindowClosable, a.modalBounds(570, 250), false, func(w *nucular.Window) {
		w.Row(30).Dynamic(1)
		kind = w.ComboSimple([]string{"Uncommitted changes", "Base branch", "Commit", "Custom instructions"}, kind, 28)
		if kind > 0 {
			label := []string{"", "Base branch", "Commit SHA", "Instructions"}[kind]
			a.field(w, label, value, false)
		}
		if errText != "" {
			muted(w, errText, a.p)
		}
		w.Row(30).Dynamic(2)
		if primary(w, "Start review", a.p) {
			target, err := reviewTarget(kind, text(value))
			if err != nil {
				errText = err.Error()
			} else {
				a.rpc("review/start", map[string]any{"threadId": thread, "target": target, "delivery": "inline"}, nil)
				w.Close()
			}
		}
		if w.ButtonText("Cancel") {
			w.Close()
		}
	})
}
