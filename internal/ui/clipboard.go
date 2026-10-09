package ui

import (
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/platform"
)

func (a *App) pasteComposer(v *chatView) {
	original, at := text(v.Editor), position(v.Editor)
	a.work(func() {
		data, err := platform.ClipboardPNG()
		path, value := "", ""
		if err == nil && len(data) > 0 {
			path, err = storeClipboardImage(a.store.Dir, data)
		} else if err == nil {
			value, err = desktop.ReadClipboardTextResult()
		}
		a.post(func() {
			if err != nil {
				a.report(err)
				return
			}
			owned := false
			for _, view := range a.chats {
				if view == v {
					owned = true
					break
				}
			}
			if !owned {
				return
			}
			if path != "" {
				v.Attachments = append(v.Attachments, path)
			} else {
				if text(v.Editor) != original {
					a.toast = "The draft changed while reading the clipboard; paste again to choose the insertion point"
					return
				}
				at.apply(v.Editor)
				v.Editor.Paste(value)
			}
		})
	})
}
