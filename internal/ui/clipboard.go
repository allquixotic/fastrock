package ui

import (
	"fmt"
	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/platform"
	"os"
	"path/filepath"
	"time"
)

func (a *App) pasteComposer(v *chatView) {
	a.work(func() {
		data, err := platform.ClipboardPNG()
		path, value := "", ""
		if err == nil && len(data) > 0 {
			path = filepath.Join(a.store.Dir, fmt.Sprintf("clipboard-%d.png", time.Now().UnixNano()))
			err = os.WriteFile(path, data, 0600)
		} else if err == nil {
			value = nucular.ReadClipboardText()
		}
		a.post(func() {
			if err != nil {
				a.report(err)
				return
			}
			if path != "" {
				v.Attachments = append(v.Attachments, path)
			} else {
				v.Editor.Paste(value)
			}
		})
	})
}
