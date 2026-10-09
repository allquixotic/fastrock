package ui

import (
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"golang.org/x/mobile/event/mouse"
)

// Only the current table page retains measured link text. Reuse it on idle
// frames, and reflow when the value, column width, font or display scale changes.
type tableLinkLayout struct {
	value string
	width int
	face  font.Face
	lines []string
}

func (l *tableLinkLayout) prepare(value string, width int, face font.Face) {
	if l.value == value && l.width == width && l.face == face {
		return
	}
	l.value, l.width, l.face = value, width, face
	l.lines = desktop.WrapText(face, value, width)
}

func tableLink(w *desktop.Window, l *tableLinkLayout, padding int, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	lineHeight := l.face.Metrics().Height.Ceil()
	y := b.Y + (b.H-len(l.lines)*lineHeight)/2
	hover := in.Mouse.HoveringRect(b)
	out.Cursor(b, font.PointerCursor)
	for _, line := range l.lines {
		r := rect.Rect{X: b.X + padding, Y: y, W: max(1, b.W-2*padding), H: lineHeight}
		out.DrawText(r, line, l.face, p.Accent)
		if hover && line != "" {
			out.FillRect(rect.Rect{X: r.X, Y: y + lineHeight - 1, W: min(r.W, l.face.MeasureString(line)), H: 1}, 0, p.Accent)
		}
		y += lineHeight
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
