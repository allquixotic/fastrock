package ui

import (
	"image"
	"image/color"
	"math"
	"unicode"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"golang.org/x/mobile/event/mouse"
)

func inset(r rect.Rect, x, y int) rect.Rect {
	return rect.Rect{X: r.X + x, Y: r.Y + y, W: max(0, r.W-2*x), H: max(0, r.H-2*y)}
}

func tooltipButton(w *desktop.Window, caption, tip string) bool {
	b := w.WidgetBounds()
	clicked := w.ButtonText(caption)
	if w.Input().Mouse.HoveringRect(b) {
		w.Tooltip(tip)
	}
	return clicked
}

func detailStatusButton(w *desktop.Window, caption string, active bool, tone color.RGBA, p palette) bool {
	if active {
		p.Accent = tone
		return primary(w, caption, p)
	}
	return w.ButtonText(caption)
}
func labelAt(out *command.Buffer, r rect.Rect, s string, f font.Face, c color.RGBA) {
	r.Y += (r.H - desktop.FontHeight(f)) / 2
	r.H = desktop.FontHeight(f) + 2
	out.DrawText(r, ellipsize(s, f, r.W), f, c)
}
func ellipsize(s string, f font.Face, width int) string {
	if desktop.FontWidth(f, s) <= width {
		return s
	}
	suffix := "…"
	width -= desktop.FontWidth(f, suffix)
	lo, hi := 0, len(s)
	for lo < hi {
		mid := (lo + hi + 1) / 2
		for mid < len(s) && !utf8.RuneStart(s[mid]) {
			mid++
		}
		if mid > hi {
			mid = hi
		}
		if desktop.FontWidth(f, s[:mid]) <= width {
			lo = mid
		} else {
			hi = mid - 1
			for hi > 0 && !utf8.RuneStart(s[hi]) {
				hi--
			}
		}
	}
	// Never leave a combining mark or joiner dangling at the truncation point.
	for lo > 0 && lo < len(s) {
		r, _ := utf8.DecodeRuneInString(s[lo:])
		prev, n := utf8.DecodeLastRuneInString(s[:lo])
		if !unicode.Is(unicode.Mn, r) && r != '\u200d' && prev != '\u200d' {
			break
		}
		lo -= n
	}
	if lo > 0 {
		return s[:lo] + suffix
	}
	return suffix
}
func closeGlyph(out *command.Buffer, r rect.Rect, c color.RGBA) {
	x, y := r.X+r.W/2, r.Y+r.H/2
	out.StrokeLine(image.Pt(x-3, y-3), image.Pt(x+3, y+3), 1, c)
	out.StrokeLine(image.Pt(x+3, y-3), image.Pt(x-3, y+3), 1, c)
}
func iconButton(w *desktop.Window, icon string, active bool, p palette) bool {
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
	r := rect.Rect{X: b.X + (b.W-18)/2, Y: b.Y + (b.H-16)/2, W: 18, H: 16}
	c := p.Muted
	if active {
		c = p.Text
	}
	switch icon {
	case "sidebar", "info":
		o.FillRect(rect.Rect{X: r.X, Y: r.Y, W: r.W, H: r.H}, 2, p.Border)
		o.FillRect(inset(r, 1, 1), 1, p.Window)
		x := r.X + 5
		if icon == "info" {
			x = r.X + r.W - 5
		}
		o.StrokeLine(image.Pt(x, r.Y), image.Pt(x, r.Y+r.H), 1, c)
	case "close":
		closeGlyph(o, b, c)
	case "plus":
		x, y := b.X+b.W/2, b.Y+b.H/2
		o.StrokeLine(image.Pt(x-4, y), image.Pt(x+4, y), 1, c)
		o.StrokeLine(image.Pt(x, y-4), image.Pt(x, y+4), 1, c)
	case "tab-left", "tab-right":
		x, y, direction := b.X+b.W/2, b.Y+b.H/2, 1
		if icon == "tab-left" {
			direction = -1
		}
		o.StrokeLine(image.Pt(x-2*direction, y-4), image.Pt(x+2*direction, y), 1, c)
		o.StrokeLine(image.Pt(x+2*direction, y), image.Pt(x-2*direction, y+4), 1, c)
	default:
		labelAt(o, inset(b, 5, 0), icon, w.Master().Style().Font, c)
	}
	if in.Mouse.HoveringRect(b) {
		tips := map[string]string{"sidebar": "Show or hide conversations", "info": "Show or hide conversation information", "plus": "New tab or conversation", "close": "Hide status bar"}
		if tip := tips[icon]; tip != "" {
			w.Tooltip(tip)
		}
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
func flatRow(w *desktop.Window, title, detail string, selected bool, dot color.RGBA, p palette) bool {
	return flatStatusRow(w, title, detail, selected, statusDot{Color: dot}, 12, p)
}
func drawStatusDot(out *command.Buffer, bounds rect.Rect, dot statusDot) {
	if dot.Color.A == 0 {
		return
	}
	if dot.Hollow {
		x, y := float64(bounds.X)+float64(bounds.W)/2, float64(bounds.Y)+float64(bounds.H)/2
		radius := float64(min(bounds.W, bounds.H)-1) / 2
		point := func(i int) image.Point {
			angle := float64(i) * 2 * math.Pi / 16
			return image.Pt(int(math.Round(x+radius*math.Cos(angle))), int(math.Round(y+radius*math.Sin(angle))))
		}
		for i := range 16 {
			out.StrokeLine(point(i), point(i+1), 1, dot.Color)
		}
	} else {
		out.FillCircle(bounds, dot.Color)
	}
}
func flatStatusRow(w *desktop.Window, title, detail string, selected bool, dot statusDot, indent int, p palette) bool {
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
	scale := w.Master().Style().Scaling
	x := b.X + int(float64(indent)*scale)
	if dot.Color.A != 0 {
		size := int(8 * scale)
		drawStatusDot(o, rect.Rect{X: x, Y: b.Y + (b.H-size)/2, W: size, H: size}, dot)
		x += int(15 * scale)
	}
	fg := p.Muted
	if selected {
		fg = p.Text
	}
	detailWidth := 0
	if detail != "" {
		detailWidth = min(b.W/3, desktop.FontWidth(face, detail)+12)
		labelAt(o, rect.Rect{X: b.X + b.W - detailWidth - 8, Y: b.Y, W: detailWidth, H: b.H}, detail, face, p.Faint)
	}
	labelAt(o, rect.Rect{X: x, Y: b.Y, W: b.W - (x - b.X) - 10 - detailWidth, H: b.H}, title, face, fg)
	if in.Mouse.HoveringRect(b) {
		w.Tooltip(title)
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}

type tabRects struct{ Body, Title, Close, Dot rect.Rect }

func tabLayout(b rect.Rect) tabRects {
	return tabRects{Body: b, Title: rect.Rect{X: b.X + 23, Y: b.Y, W: max(0, b.W-49), H: b.H}, Close: rect.Rect{X: b.X + b.W - 24, Y: b.Y + (b.H-20)/2, W: 20, H: 20}, Dot: rect.Rect{X: b.X + 9, Y: b.Y + b.H/2 - 4, W: 8, H: 8}}
}
func documentTab(w *desktop.Window, title string, active, dragged bool, dot statusDot, p palette) (activate, close bool, b rect.Rect) {
	b, o := w.Custom(w.CustomState())
	if o == nil {
		return false, false, b
	}
	r := tabLayout(b)
	in := w.Input()
	if dragged {
		o.FillRect(b, 4, p.Accent)
		o.FillRect(inset(b, 1, 1), 3, p.Surface)
	} else if active {
		o.FillRect(b, 4, p.Surface)
	} else if in.Mouse.HoveringRect(b) {
		o.FillRect(b, 4, p.Hover)
	}
	drawStatusDot(o, r.Dot, dot)
	fg := p.Muted
	if active || dragged {
		fg = p.Text
	}
	labelAt(o, r.Title, title, w.Master().Style().Font, fg)
	if in.Mouse.HoveringRect(r.Close) {
		w.Tooltip("Close tab")
		o.FillRect(r.Close, 4, p.Hover)
	}
	closeGlyph(o, r.Close, p.Muted)
	close = in.Mouse.Clicked(mouse.ButtonLeft, r.Close) || in.Mouse.Clicked(mouse.ButtonMiddle, b)
	activate = !close && in.Mouse.Clicked(mouse.ButtonLeft, b)
	return
}

func formatButton(w *desktop.Window, label, tip string, active bool, p palette) bool {
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
		r.X += (b.W - desktop.FontWidth(f, label)) / 2
		labelAt(out, r, label, f, p.Muted)
	}
	if in.Mouse.HoveringRect(b) {
		w.Tooltip(tip)
	}
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}

func folderRow(w *desktop.Window, title string, expanded bool, p palette) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	in := w.Input()
	if in.Mouse.HoveringRect(b) {
		out.FillRect(inset(b, 2, 1), 4, p.Hover)
		action := "Expand "
		if expanded {
			action = "Collapse "
		}
		w.Tooltip(action + title)
	}
	x, y := b.X+12, b.Y+b.H/2
	if expanded {
		out.StrokeLine(image.Pt(x-3, y-2), image.Pt(x, y+1), 1, p.Muted)
		out.StrokeLine(image.Pt(x, y+1), image.Pt(x+3, y-2), 1, p.Muted)
	} else {
		out.StrokeLine(image.Pt(x-1, y-3), image.Pt(x+2, y), 1, p.Muted)
		out.StrokeLine(image.Pt(x+2, y), image.Pt(x-1, y+3), 1, p.Muted)
	}
	labelAt(out, rect.Rect{X: b.X + 26, Y: b.Y, W: b.W - 32, H: b.H}, title, w.Master().Style().Font, p.Muted)
	return in.Mouse.Clicked(mouse.ButtonLeft, b)
}
