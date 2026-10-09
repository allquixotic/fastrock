package ui

import (
	"fmt"
	"runtime"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
)

type settingPair struct{ Key, Value *desktop.TextEditor }

func newSettingPair(key, value string) settingPair {
	return settingPair{textEditor(key, false), textEditor(value, false)}
}

func (a *App) drawStringList(w *desktop.Window, titleText string, values *[]*desktop.TextEditor) {
	title(w, titleText, a.p)
	remove := -1
	for i, editor := range *values {
		w.Row(28).Ratio(.87, .13)
		editor.Edit(w)
		if w.ButtonText("Remove") {
			remove = i
		}
	}
	if remove >= 0 {
		*values = append((*values)[:remove], (*values)[remove+1:]...)
	}
	w.Row(28).Static(150)
	if w.ButtonText("Add argument") {
		*values = append(*values, textEditor("", false))
	}
}
func (a *App) drawSettingPairs(w *desktop.Window, titleText string, values *[]settingPair) {
	title(w, titleText, a.p)
	remove := -1
	for i, pair := range *values {
		w.Row(28).Ratio(.3, .57, .13)
		pair.Key.Placeholder = "Name"
		pair.Value.Placeholder = "Value"
		pair.Key.Edit(w)
		pair.Value.Edit(w)
		if w.ButtonText("Remove") {
			remove = i
		}
	}
	if remove >= 0 {
		*values = append((*values)[:remove], (*values)[remove+1:]...)
	}
	w.Row(28).Static(150)
	if w.ButtonText("Add entry") {
		*values = append(*values, newSettingPair("", ""))
	}
}
func settingPairs(values []settingPair, kind string) (map[string]string, error) {
	result := make(map[string]string, len(values))
	for _, pair := range values {
		key, value := strings.TrimSpace(text(pair.Key)), text(pair.Value)
		if kind == "environment" && !environmentName(key) {
			return nil, fmt.Errorf("enter a valid environment variable name")
		}
		if kind == "headers" && (!headerName(key) || strings.ContainsAny(value, "\r\n\x00")) {
			return nil, fmt.Errorf("headers need valid names and single-line values")
		}
		if _, exists := result[key]; exists {
			return nil, fmt.Errorf("duplicate %s name: %s", kind, key)
		}
		result[key] = value
	}
	return result, nil
}
func environmentName(name string) bool {
	if name == "" {
		return false
	}
	for i, c := range name {
		if !(c == '_' || c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' || i > 0 && c >= '0' && c <= '9') {
			return false
		}
	}
	return true
}
func headerName(name string) bool {
	if name == "" {
		return false
	}
	for _, c := range name {
		if !(c >= 'A' && c <= 'Z' || c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || strings.ContainsRune("!#$%&'*+-.^_`|~", c)) {
			return false
		}
	}
	return true
}

// Display quoting follows the platform command-line convention; no shell is run.
func displayCommand(words []string) string {
	quoted := make([]string, len(words))
	for i, word := range words {
		if runtime.GOOS == "windows" {
			quoted[i] = quoteWindowsWord(word)
		} else {
			if word != "" && strings.IndexFunc(word, func(c rune) bool {
				return !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || strings.ContainsRune("_@%+=:,./-", c))
			}) < 0 {
				quoted[i] = word
			} else {
				quoted[i] = "'" + strings.ReplaceAll(word, "'", "'\"'\"'") + "'"
			}
		}
	}
	return strings.Join(quoted, " ")
}
func quoteWindowsWord(word string) string {
	if word != "" && !strings.ContainsAny(word, " \t\n\r\"") {
		return word
	}
	var out strings.Builder
	out.WriteByte('"')
	slashes := 0
	for _, c := range word {
		if c == '\\' {
			slashes++
			continue
		}
		if c == '"' {
			out.WriteString(strings.Repeat("\\", slashes*2+1))
		} else {
			out.WriteString(strings.Repeat("\\", slashes))
		}
		out.WriteRune(c)
		slashes = 0
	}
	out.WriteString(strings.Repeat("\\", slashes*2))
	out.WriteByte('"')
	return out.String()
}
