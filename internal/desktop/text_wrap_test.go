package desktop

import (
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"image/color"
	"testing"
)

func TestWrapPreservesWordsAndNewlines(t *testing.T) {
	f := font.DefaultFont(13, 1)
	lines := WrapText(f, "alpha beta\ngamma", FontWidth(f, "alpha beta")+2)
	if len(lines) != 2 || lines[0] != "alpha beta" || lines[1] != "gamma" {
		t.Fatal(lines)
	}
}
func TestWrapRendersSingleAndLastLine(t *testing.T) {
	f := font.DefaultFont(13, 1)
	b := command.Buffer{Clip: rect.Rect{W: 1000, H: 1000}}
	w := textWidget{Text: color.RGBA{255, 255, 255, 255}}
	height := FontHeight(f)
	widgetTextWrap(&b, rect.Rect{W: 300, H: height * 2}, []rune("first\nlast"), &w, f)
	count := 0
	for _, c := range b.Commands {
		if c.Kind == command.TextCmd {
			count++
		}
	}
	if count != 2 {
		t.Fatalf("rendered %d lines, want 2", count)
	}
}
