package desktop

import (
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/desktop/style"
	"golang.org/x/image/font/gofont/goregular"
	"image"
	"image/color"
	"testing"
)

func TestSoftWrapPreservesBufferAndHitTesting(t *testing.T) {
	face, err := font.NewFace(goregular.TTF, 13)
	if err != nil {
		t.Fatal(err)
	}
	defer face.Face.Close()
	ed := TextEditor{Buffer: []rune("one two three four five 🚀"), Flags: EditSoftWrap, wrapRight: 75}
	out := command.Buffer{Clip: rect.Rect{W: 200, H: 1000}}
	pos := ed.editDrawText(&out, &style.Edit{}, image.Point{}, 0, ed.Buffer, 0, 20, face, color.RGBA{}, color.RGBA{A: 255}, false)
	if pos.Y == 0 || ed.Snapshot() != "one two three four five 🚀" {
		t.Fatal("wrap changed text or did not wrap")
	}
	if len(ed.drawchunks) < 2 {
		t.Fatal("missing display rows")
	}
	row := ed.drawchunks[1]
	if got := ed.locateCoord(image.Pt(row.X, row.Y+1), face, 20); got != row.start {
		t.Fatalf("hit-test %d, want %d", got, row.start)
	}
	if point := ed.indexToCoord(row.start, face, 20); point.Y < 20 {
		t.Fatal("caret remained on first row")
	}
}
