//go:build fltk_headless

package ui

import (
	"image"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
)

func TestV29TypographyRolesRestoreBodyFont(t *testing.T) {
	p := colors(false)
	ed := textEditor("literal code", true)
	seen := map[string]int{}
	h := desktop.NewHeadlessHarness(0, image.Pt(700, 500), func(w *desktop.Window) {
		title(w, "Heading", p)
		muted(w, "Caption", p)
		w.Row(30).Dynamic(1)
		w.Label("Body", "LC")
		w.Row(80).Dynamic(1)
		codeEditor(w, ed)
		a := &App{p: p}
		a.markdown(w, "```sh\ncode block\n```")
		for _, cmd := range w.Commands().Commands {
			if cmd.Kind == command.TextCmd {
				seen[cmd.Text.String] = fontPointSize(cmd.Text.Face)
			}
		}
	})
	for _, size := range []int{13, 18} {
		clear(seen)
		h.Master().SetStyle(makeStyle(p, size))
		h.Frame(false)
		if seen["Heading"] != size+7 || seen["Caption"] != size-2 || seen["Body"] != size || seen["literal code"] != size-1 || seen["code block"] != size-1 {
			t.Fatal(size, seen)
		}
		if h.Master().Style().Font != typeFace(size, regularFont) {
			t.Fatal("font leaked into following controls")
		}
	}
}
