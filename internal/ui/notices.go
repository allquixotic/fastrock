package ui

import (
	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/rect"
	"golang.org/x/mobile/event/mouse"
	"time"
)

type notice struct {
	Text, Kind  string
	Until       time.Time
	ActionLabel string
	Action      func()
}

func (a *App) collectNotice() {
	if a.toast != "" {
		life := 4 * time.Second
		if a.noticeKind == "error" {
			life = 12 * time.Second
		}
		a.notices = append(a.notices, notice{Text: cut(a.toast, 4096), Kind: a.noticeKind, Until: time.Now().Add(life)})
		if len(a.notices) > 4 {
			a.notices = append([]notice(nil), a.notices[len(a.notices)-4:]...)
		}
		a.toast, a.noticeKind = "", ""
		time.AfterFunc(life, func() {
			if a.window != nil {
				a.window.Changed()
			}
		})
	}
	n := 0
	for _, message := range a.notices {
		if time.Now().Before(message.Until) {
			a.notices[n] = message
			n++
		}
	}
	clear(a.notices[n:])
	a.notices = a.notices[:n]
}

func (a *App) actionNotice(message, label string, action func()) {
	a.notices = append(a.notices, notice{Text: message, Until: time.Now().Add(10 * time.Second), ActionLabel: label, Action: action})
	if len(a.notices) > 4 {
		a.notices = append([]notice(nil), a.notices[len(a.notices)-4:]...)
	}
	time.AfterFunc(10*time.Second, func() {
		if a.window != nil {
			a.window.Changed()
		}
	})
}

type noticeLayout struct {
	index          int
	bounds, button rect.Rect
	lines          []string
}

func (a *App) noticeLayouts(w *nucular.Window) []noticeLayout {
	width := min(680, max(160, w.Bounds.W-32))
	y := w.Bounds.Y + w.Bounds.H - 14
	layouts := make([]noticeLayout, 0, len(a.notices))
	for i := len(a.notices) - 1; i >= 0; i-- {
		n := a.notices[i]
		if time.Now().After(n.Until) {
			continue
		}
		lines := nucular.WrapText(w.Master().Style().Font, n.Text, width-32)
		if len(lines) > 4 {
			lines = append(lines[:3], "…")
		}
		height := len(lines)*(a.prefs.FontSize+6) + 16
		if n.ActionLabel != "" {
			height += 30
		}
		y -= height
		b := rect.Rect{X: w.Bounds.X + (w.Bounds.W-width)/2, Y: y, W: width, H: height}
		button := rect.Rect{X: b.X + 12, Y: b.Y + b.H - 30, W: min(150, b.W-24), H: 26}
		layouts = append(layouts, noticeLayout{i, b, button, lines})
		y -= 6
	}
	return layouts
}

// Handle overlays before document controls, so Undo cannot also click the card
// or toolbar underneath it. The focused modal still owns its pointer events.
func (a *App) handleNoticeInput(w *nucular.Window) {
	in := w.Input()
	var activate func()
	for _, layout := range a.noticeLayouts(w) {
		n := &a.notices[layout.index]
		if n.ActionLabel == "" || !in.Mouse.HoveringRect(layout.bounds) {
			continue
		}
		if n.Action != nil && in.Mouse.Clicked(mouse.ButtonLeft, layout.button) {
			activate = n.Action
			n.Action = nil
			n.ActionLabel = "Undo requested"
		}
		for _, button := range []mouse.Button{mouse.ButtonLeft, mouse.ButtonMiddle, mouse.ButtonRight} {
			in.Mouse.Buttons[button].Clicked = false
		}
		if v := a.currentRally(); v != nil && v.cardDragging && !in.Mouse.Down(mouse.ButtonLeft) {
			v.cardDragging = false
			v.dragCard = ""
		}
	}
	if activate != nil {
		activate()
	}
}

// Notices overlay the bottom edge without moving the document or stealing
// editor focus. Persistent actionable failures also remain at their source.
func (a *App) drawNotices(w *nucular.Window) {
	out := w.Commands()
	old := out.Clip
	out.PushScissor(w.Bounds)
	defer out.PushScissor(old)
	for _, layout := range a.noticeLayouts(w) {
		n := a.notices[layout.index]
		out.FillRect(layout.bounds, 8, a.p.Alt)
		fg := a.p.Text
		if n.Kind == "error" {
			fg = a.p.Danger
		}
		for j, line := range layout.lines {
			labelAt(out, rect.Rect{X: layout.bounds.X + 16, Y: layout.bounds.Y + 8 + j*(a.prefs.FontSize+6), W: layout.bounds.W - 32, H: a.prefs.FontSize + 6}, line, w.Master().Style().Font, fg)
		}
		if n.ActionLabel != "" {
			color := a.p.Accent
			if n.Action == nil {
				color = a.p.Muted
			}
			out.FillRect(layout.button, 4, a.p.Selected)
			labelAt(out, inset(layout.button, 8, 0), n.ActionLabel, w.Master().Style().Font, color)
		}
	}
}
