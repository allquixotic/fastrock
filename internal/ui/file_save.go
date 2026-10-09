package ui

import (
	"path/filepath"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/settings"
)

func suggestedTextName(title string) string {
	name := filepath.Base(strings.ReplaceAll(title, "\\", "/"))
	name = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || strings.ContainsRune(`<>:"/\|?*`, r) {
			return '_'
		}
		return r
	}, name)
	name = strings.Trim(name, " .")
	if name == "" {
		name = "document"
	}
	ext := strings.ToLower(filepath.Ext(name))
	if ext != ".md" && ext != ".markdown" && ext != ".txt" {
		ext = ".md"
	} else {
		name = strings.TrimSuffix(name, filepath.Ext(name))
	}
	base := strings.ToUpper(strings.SplitN(name, ".", 2)[0])
	if base == "CON" || base == "PRN" || base == "AUX" || base == "NUL" ||
		len(base) == 4 && (strings.HasPrefix(base, "COM") || strings.HasPrefix(base, "LPT")) && base[3] >= '1' && base[3] <= '9' {
		name = "_" + name
	}
	// Leave room for extensions within common 255-byte filename limits.
	for len(name) > 180 {
		_, size := utf8.DecodeLastRuneInString(name)
		name = name[:len(name)-size]
	}
	return name + ext
}

func (a *App) saveTextAs(title, value string) {
	name := suggestedTextName(title)
	a.work(func() {
		path, err := platform.ChooseSaveText(name)
		a.post(func() {
			if err != nil {
				a.report(err)
			} else if path != "" {
				a.writeTextFile(path, value)
			}
		})
	})
}

func (a *App) writeTextFile(path, value string) {
	a.writeWork(func() {
		err := settings.WriteFileAtomic(path, []byte(value), 0600)
		a.post(func() {
			if err != nil {
				a.report(err)
			} else {
				a.toast = "Saved"
			}
		})
	})
}
