package ui

import (
	"image"
	"image/color"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"
	"golang.org/x/mobile/event/mouse"
)

func inset(r rect.Rect, x, y int) rect.Rect {
	return rect.Rect{X: r.X + x, Y: r.Y + y, W: max(0, r.W-2*x), H: max(0, r.H-2*y)}
}
func labelAt(out *command.Buffer, r rect.Rect, s string, f font.Face, c color.RGBA) {
	r.Y += (r.H - nucular.FontHeight(f)) / 2
	r.H = nucular.FontHeight(f) + 2
	out.DrawText(r, ellipsize(s, f, r.W), f, c)
}
func ellipsize(s string, f font.Face, width int) string {
	if nucular.FontWidth(f, s) <= width {
		return s
	}
	suffix := "…"
	width -= nucular.FontWidth(f, suffix)
	for len(s) > 0 {
		_, n := utf8.DecodeLastRuneInString(s)
		s = s[:len(s)-n]
		if nucular.FontWidth(f, s) <= width {
			return s + suffix
		}
	}
	return suffix
}
func closeGlyph(out *command.Buffer, r rect.Rect, c color.RGBA) {
	x, y := r.X+r.W/2, r.Y+r.H/2
	out.StrokeLine(image.Pt(x-3, y-3), image.Pt(x+3, y+3), 1, c)
	out.StrokeLine(image.Pt(x+3, y-3), image.Pt(x-3, y+3), 1, c)
}
func iconButton(w *nucular.Window, icon string, active bool, p palette) bool {
	b, o := w.Custom(w.CustomState())
	if o == nil {
		return false
	}
	in := w.Input()
	if active {
		o.FillRect(inset(b, 2, 3), 4, p.Selected)
	} else if in.Mouse.HoveringRect(b) {
		o.FillRect(inset(b, 2, 3), 4, p.Hover)
	}
	r := inset(b, 7, 7)
	c := p.Muted
	if active {
		c = p.Text
	}
	switch icon {
	case "sidebar":
		o.FillRect(rect.Rect{X: r.X, Y: r.Y, W: r.W, H: r.H}, 2, p.Border)
		o.FillRect(inset(r, 1, 1), 1, p.Window)
		o.StrokeLine(image.Pt(r.X+5, r.Y), image.Pt(r.X+5, r.Y+r.H), 1, c)
	case "close":
		closeGlyph(o, b, c)
	case "plus":
		x, y := b.X+b.W/2, b.Y+b.H/2
		o.StrokeLine(image.Pt(x-4, y), image.Pt(x+4, y), 1, c)
		o.StrokeLine(image.Pt(x, y-4), image.Pt(x, y+4), 1, c)
	default:
		labelAt(o, inset(b, 5, 0), icon, w.Master().Style().Font, c)
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
func flatRow(w *nucular.Window, title, detail string, selected bool, dot color.RGBA, p palette) bool {
	b, o := w.Custom(w.CustomState())
	if o == nil {
		return false
	}
	in := w.Input()
	if selected {
		o.FillRect(inset(b, 2, 1), 4, p.Selected)
	} else if in.Mouse.HoveringRect(b) {
		o.FillRect(inset(b, 2, 1), 4, p.Hover)
	}
	face := w.Master().Style().Font
	x := b.X + 12
	if dot.A != 0 {
		o.FillCircle(rect.Rect{X: x, Y: b.Y + b.H/2 - 3, W: 6, H: 6}, dot)
		x += 13
	}
	fg := p.Muted
	if selected {
		fg = p.Text
	}
	labelAt(o, rect.Rect{X: x, Y: b.Y, W: b.W - (x - b.X) - 10, H: b.H}, title, face, fg)
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}

type tabRects struct{ Body, Title, Close, Dot rect.Rect }

func tabLayout(b rect.Rect) tabRects {
	return tabRects{Body: b, Title: rect.Rect{X: b.X + 23, Y: b.Y, W: max(0, b.W-49), H: b.H}, Close: rect.Rect{X: b.X + b.W - 24, Y: b.Y + (b.H-20)/2, W: 20, H: 20}, Dot: rect.Rect{X: b.X + 10, Y: b.Y + b.H/2 - 3, W: 6, H: 6}}
}
func documentTab(w *nucular.Window, title string, active bool, dot color.RGBA, p palette) (activate, close bool, b rect.Rect) {
	b, o := w.Custom(w.CustomState())
	if o == nil {
		return false, false, b
	}
	r := tabLayout(b)
	in := w.Input()
	if active {
		o.FillRect(b, 4, p.Surface)
	} else if in.Mouse.HoveringRect(b) {
		o.FillRect(b, 4, p.Hover)
	}
	if dot.A != 0 {
		o.FillCircle(r.Dot, dot)
	}
	fg := p.Muted
	if active {
		fg = p.Text
	}
	labelAt(o, r.Title, title, w.Master().Style().Font, fg)
	if in.Mouse.HoveringRect(r.Close) {
		o.FillRect(r.Close, 4, p.Hover)
	}
	closeGlyph(o, r.Close, p.Muted)
	close = in.Mouse.Clicked(mouse.ButtonLeft, r.Close) || in.Mouse.Clicked(mouse.ButtonMiddle, b)
	activate = !close && in.Mouse.Clicked(mouse.ButtonLeft, b)
	return
}

func sectionTab(w *nucular.Window, title string, active bool, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	if in.Mouse.HoveringRect(b) {
		out.FillRect(b, 3, p.Hover)
	}
	fg := p.Muted
	if active {
		fg = p.Text
	}
	labelAt(out, inset(b, 12, 0), title, w.Master().Style().Font, fg)
	out.FillRect(rect.Rect{X: b.X, Y: b.Y + b.H - 1, W: b.W, H: 1}, 0, p.Border)
	if active {
		out.FillRect(rect.Rect{X: b.X + 8, Y: b.Y + b.H - 2, W: b.W - 16, H: 2}, 0, p.Accent)
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}

func formatButton(w *nucular.Window, label, tip string, active bool, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	if active {
		out.FillRect(inset(b, 1, 2), 3, p.Selected)
	} else if in.Mouse.HoveringRect(b) {
		out.FillRect(inset(b, 1, 2), 3, p.Hover)
	}
	f := w.Master().Style().Font
	if label == "undo" || label == "redo" {
		x, y := b.X+b.W/2, b.Y+b.H/2
		sign := 1
		if label == "redo" {
			sign = -1
		}
		out.StrokeLine(image.Pt(x-5*sign, y), image.Pt(x+5*sign, y), 1, p.Muted)
		out.StrokeLine(image.Pt(x-5*sign, y), image.Pt(x-sign, y-4), 1, p.Muted)
		out.StrokeLine(image.Pt(x-5*sign, y), image.Pt(x-sign, y+4), 1, p.Muted)
	} else {
		r := b
		r.X += (b.W - nucular.FontWidth(f, label)) / 2
		labelAt(out, r, label, f, p.Muted)
	}
	if in.Mouse.HoveringRect(b) {
		w.Tooltip(tip)
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
