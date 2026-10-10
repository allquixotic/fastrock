//go:build fltk_headless

package desktop

import (
	"fmt"
	"image"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
)

func TestV81MenusStayInsideAndAlignLeft(t *testing.T) {
	for _, flags := range []WindowFlags{windowMenu, windowContextual, windowCombo, windowContextual | windowHDynamic} {
		for _, point := range []image.Point{{390, 10}, {390, 290}, {10, 290}} {
			t.Run(fmt.Sprint(flags, point), func(t *testing.T) {
				var popup *Window
				h := NewHeadlessHarness(0, image.Pt(400, 300), func(w *Window) {
					if popup == nil {
						popup = w.ctx.nonblockOpen(flags|WindowNoScrollbar, rect.Rect{X: point.X, Y: point.Y, W: 180, H: 300}, rect.Rect{X: point.X, Y: point.Y - 20, W: 10, H: 20}, func(m *Window) {
							m.Row(28).Dynamic(1)
							m.MenuItem(label.T("Short"))
							m.MenuItem(label.T("Much longer choice"))
							m.MenuItem(label.TA("Last choice", "RC"))
						})
					}
				})
				h.Frame(false)
				h.Frame(false)
				x := -1
				found := 0
				for _, c := range h.Commands() {
					if c.Kind != command.TextCmd {
						continue
					}
					if c.Rect.X < 0 || c.Rect.Y < 0 || c.Rect.X+c.Rect.W > 400 || c.Rect.Y+c.Rect.H > 300 {
						t.Fatal("menu text outside window", c.Rect)
					}
					if x >= 0 && c.Rect.X != x {
						t.Fatal("menu labels not left aligned", x, c.Rect.X)
					}
					x = c.Rect.X
					found++
				}
				if found != 3 {
					t.Fatal("clipped menu choices", found)
				}
			})
		}
	}
}

func TestV81DropdownCanGrowAndScrollAfterLoading(t *testing.T) {
	var popup *Window
	count := 1
	h := NewHeadlessHarness(0, image.Pt(400, 300), func(w *Window) {
		if popup == nil {
			popup = w.ctx.nonblockOpen(windowCombo, rect.Rect{X: 380, Y: 280, W: 220, H: 200}, rect.Rect{X: 380, Y: 250, W: 20, H: 30}, func(m *Window) {
				m.Row(28).Dynamic(1)
				for i := range count {
					m.MenuItem(label.T(fmt.Sprintf("Choice %d", i)))
				}
			})
		}
	})
	h.Frame(false)
	count = 50
	h.Frame(false)
	h.Frame(false)
	popup.Scrollbar.Y = 10000
	h.Frame(false)
	h.Frame(false)
	found := false
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd && c.Text.String == "Choice 49" {
			found = true
		}
	}
	if !found {
		t.Fatal("last dynamically loaded choice is inaccessible")
	}
	if popup.Bounds.X < 0 || popup.Bounds.Y < 0 || popup.Bounds.Max().X > 400 || popup.Bounds.Max().Y > 300 {
		t.Fatal("dropdown outgrew viewport", popup.Bounds)
	}
}
