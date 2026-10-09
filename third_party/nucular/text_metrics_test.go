package nucular

import (
	"image"
	"image/color"
	"testing"

	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"
	"github.com/aarzilli/nucular/style"
)

func TestV29FormattedEditorUsesMeasuredCaretAndWrap(t *testing.T) {
	f := font.DefaultFont(13, 1)
	ed := &TextEditor{Buffer: []rune("abcd"), Flags: EditSoftWrap, wrapRight: 100}
	ed.MeasureText = func(text []rune, offset int, base font.Face) int { return len(text) * 50 }
	out := &command.Buffer{}
	out.Reset()
	ed.editDrawText(out, &style.Edit{}, image.Point{}, 0, ed.Buffer, 0, 30, f, color.RGBA{A: 255}, color.RGBA{R: 255, G: 255, B: 255, A: 255}, false)
	if len(ed.drawchunks) != 2 || ed.drawchunks[0].end != 2 || ed.drawchunks[1].start != 2 {
		t.Fatal("wrong formatted wrapping", ed.drawchunks)
	}
	if at := ed.indexToCoord(3, f, 30); at.X != 50 || at.Y != 45 {
		t.Fatal("caret used base font", at)
	}
	if index := ed.locateCoord(image.Pt(51, 40), f, 30); index != 3 {
		t.Fatal("hit test used base font", index)
	}
	calls := 0
	ed.Buffer = make([]rune, 10000)
	ed.drawchunks = []drawchunk{{Rect: rect.Rect{W: 500000, H: 30}, start: 0, end: len(ed.Buffer)}}
	ed.MeasureText = func(text []rune, offset int, base font.Face) int { calls++; return len(text) * 50 }
	ed.locateCoord(image.Pt(250001, 10), f, 30)
	if calls > 15 {
		t.Fatal("prefix measurement performed linear repeated scans", calls)
	}
}
