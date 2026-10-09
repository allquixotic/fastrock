//go:build fltk_headless

package desktop

import (
	"image"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/desktop/style"
)

func TestV52GutterLogicalLines(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		value := "a\tb\n\n" + strings.Repeat("long ", 20) + "\n"
		ed := &TextEditor{Buffer: []rune(value), Flags: EditBox | EditReadOnly | EditSoftWrap | EditNoHorizontalScroll, GutterWidth: int(30 * scale)}
		starts := map[int]int{}
		ed.PaintGutter = func(out *command.Buffer, b rect.Rect, start int, face font.Face) {
			starts[start]++
			if b.X < 0 || b.W != int(30*scale) {
				t.Fatal("gutter geometry mismatch", b)
			}
		}
		h := NewHeadlessHarness(0, image.Pt(int(220*scale), int(500*scale)), func(w *Window) {
			w.RowScaled(int(400 * scale)).Dynamic(1)
			ed.Edit(w)
		})
		h.Master().SetStyle(style.FromTheme(style.DarkTheme, scale))
		h.Frame(false)
		for _, at := range []int{0, 4, 5, len(ed.Buffer)} {
			if starts[at] == 0 {
				t.Fatal("missing logical line number", at, starts)
			}
		}
		if len(starts) != 4 {
			t.Fatal("wrapped continuations received extra line numbers", starts)
		}
		if ed.gutter.X+ed.gutter.W > ed.wrapRight || ed.gutter.W == 0 {
			t.Fatal("text area did not reserve the gutter")
		}
	}
}
