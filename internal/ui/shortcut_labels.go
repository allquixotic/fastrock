package ui

import (
	"fmt"
	"strings"

	"github.com/allquixotic/fastrock/internal/platform"
	"golang.org/x/mobile/event/key"
)

func shortcutLabel(code key.Code, mods key.Modifiers) string {
	if code == 0 {
		return "Unbound"
	}
	parts := []string{}
	for _, item := range []struct {
		mod  key.Modifiers
		name string
	}{{key.ModControl, "Ctrl"}, {key.ModAlt, "Alt"}, {key.ModShift, "Shift"}, {key.ModMeta, "Cmd"}} {
		if mods&item.mod != 0 {
			parts = append(parts, item.name)
		}
	}
	name := map[key.Code]string{key.CodeReturnEnter: "Enter", key.CodeEscape: "Esc", key.CodeTab: "Tab", key.CodeSpacebar: "Space", key.CodeComma: ",", key.CodeFullStop: ".", key.CodeSlash: "/", key.CodeLeftArrow: "Left", key.CodeRightArrow: "Right", key.CodeUpArrow: "Up", key.CodeDownArrow: "Down"}[code]
	switch {
	case code >= key.CodeA && code <= key.CodeZ:
		name = string(rune('A' + code - key.CodeA))
	case code >= key.Code1 && code <= key.Code9:
		name = string(rune('1' + code - key.Code1))
	case code == key.Code0:
		name = "0"
	case code >= key.CodeF1 && code <= key.CodeF12:
		name = fmt.Sprintf("F%d", code-key.CodeF1+1)
	}
	if name == "" {
		name = fmt.Sprintf("Key %d", code)
	}
	return strings.Join(append(parts, name), "+")
}

func validateShortcut(code key.Code, mods key.Modifiers) string {
	if mods == 0 && (code >= key.CodeA && code <= key.CodeZ || code >= key.Code1 && code <= key.CodeSlash) {
		return "Use Ctrl, Cmd or Alt with a typing key. A plain typing key would block text editing."
	}
	if mods == platform.PrimaryModifier() {
		switch code {
		case key.CodeA, key.CodeC, key.CodeV, key.CodeX, key.CodeZ, key.CodeY:
			return "This shortcut is reserved for text editing."
		}
	}
	return ""
}
