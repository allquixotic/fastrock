package ui

import (
	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"
	"image/color"
	"path/filepath"
	"strconv"
	"strings"
)

type diffFile struct {
	Path, Text string
	Collapsed  bool
	Editor     *nucular.TextEditor
}

func splitDiff(source string) []diffFile {
	files := []diffFile{}
	for part := range strings.SplitSeq(source, "diff --git ") {
		if part == "" {
			continue
		}
		header, _, _ := strings.Cut(part, "\n")
		_, path, _ := strings.Cut(header, " b/")
		for line := range strings.SplitSeq(part, "\n") {
			if strings.HasPrefix(line, "+++ ") && line != "+++ /dev/null" {
				path = strings.TrimPrefix(line, "+++ ")
				if p, e := strconv.Unquote(path); e == nil {
					path = p
				}
				path = strings.TrimPrefix(path, "b/")
				break
			}
		}
		value := "diff --git " + part
		files = append(files, diffFile{Path: path, Text: value})
	}
	return files
}
func (a *App) openDiff(title, source, cwd string) {
	a.openText(title, source)
	if v := a.files[a.state.Active]; v != nil {
		v.BasePath = cwd
		a.prepareDiff(v, source)
	}
}
func (a *App) prepareDiff(v *fileView, source string) {
	a.work(func() {
		files := splitDiff(cut(source, maxFileBytes))
		for i := range files {
			f := &files[i]
			f.Editor = textEditor(f.Text, true)
			f.Editor.Flags |= nucular.EditReadOnly
			f.Editor.PaintText = func(out *command.Buffer, b rect.Rect, run []rune, start int, face font.Face, fg color.RGBA, selected bool) {
				if !selected {
					line := min(start, len(f.Editor.Buffer)-1)
					for line > 0 && f.Editor.Buffer[line-1] != '\n' {
						line--
					}
					if line >= 0 && line < len(f.Editor.Buffer) {
						switch f.Editor.Buffer[line] {
						case '+':
							fg = a.p.Success
						case '-':
							fg = a.p.Danger
						case '@':
							fg = a.p.Accent
						}
					}
				}
				out.DrawText(b, string(run), face, fg)
			}
		}
		a.post(func() { v.Diff = files })
	})
}
func (a *App) drawDiff(w *nucular.Window, v *fileView) {
	w.Row(28).Dynamic(3)
	if w.ButtonText("Expand all") {
		for i := range v.Diff {
			v.Diff[i].Collapsed = false
		}
	}
	if w.ButtonText("Collapse all") {
		for i := range v.Diff {
			v.Diff[i].Collapsed = true
		}
	}
	if w.ButtonText("View as text") {
		v.Diff = nil
		return
	}
	for i := range v.Diff {
		f := &v.Diff[i]
		w.Row(28).Ratio(.85, .15)
		if w.ButtonText(f.Path) {
			f.Collapsed = !f.Collapsed
		}
		if w.ButtonText("Open file") {
			path := f.Path
			if !filepath.IsAbs(path) {
				path = filepath.Join(v.BasePath, path)
			}
			a.openFile(path)
		}
		if !f.Collapsed {
			w.Row(min(500, 60+strings.Count(f.Text, "\n")*20)).Dynamic(1)
			f.Editor.Edit(w)
		}
	}
}
