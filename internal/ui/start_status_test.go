//go:build nucular_headless

package ui

import (
	"context"
	"image"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestV53StartPageCenteredScrolling(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		for _, width := range []int{360, 1000} {
			a := &App{ctx: context.Background(), state: workspace.NewState(), p: colors(false), newFolder: textEditor("", false)}
			a.prefs.RecentFolders = []string{"/first-folder", "/second-folder", "/third-folder", "/last-folder"}
			scroll := false
			h := nucular.NewHeadlessHarness(0, image.Pt(int(float64(width)*scale), int(360*scale)), func(w *nucular.Window) {
				w.Input().Mouse.Pos = image.Pt(int(float64(width)*scale/2), int(180*scale))
				if scroll {
					w.Input().Mouse.ScrollDelta = -10000
				}
				a.drawNew(w)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			h.Frame(false)
			found := false
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == "Start a new thread" {
					found = true
					if c.Rect.W > int(600*scale) || c.Rect.X < 0 || c.Rect.X+c.Rect.W > int(float64(width)*scale) {
						t.Fatal("start column outside bounds", scale, width, c.Rect)
					}
					if width == 1000 && c.Rect.X < int(200*scale) {
						t.Fatal("start column not centered", c.Rect)
					}
				}
			}
			if !found {
				t.Fatal("reference start heading missing", scale, width)
			}
			scroll = true
			for range 3 {
				h.Frame(false)
			}
			found = false
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == "/last-folder" && c.Rect.Y >= 0 && c.Rect.Y < int(360*scale) {
					found = true
				}
			}
			if !found {
				t.Fatal("lower start actions cannot be scrolled into view", scale, width)
			}
		}
	}
}

func TestV53DetailStatusButtons(t *testing.T) {
	a := transferFixture()
	a.p = colors(false)
	a.prefs.FontSize = 13
	d := makeDetail(rally.Object{"Name": "Story", "Blocked": true, "Ready": true, "BlockedReason": "Waiting"}, "HierarchicalRequirement", false)
	var click image.Point
	h := nucular.NewHeadlessHarness(0, image.Pt(800, 1100), func(w *nucular.Window) {
		if click != (image.Point{}) {
			m := &w.Input().Mouse
			m.Pos = click
			m.Buttons[mouse.ButtonLeft].Down = false
			m.Buttons[mouse.ButtonLeft].Clicked = true
			m.Buttons[mouse.ButtonLeft].ClickedPos = click
		}
		a.detailFields(w, d)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	green, red, reason := false, false, false
	for _, c := range h.Commands() {
		if c.Kind == command.RectFilledCmd {
			green = green || c.RectFilled.Color == a.p.Success
			red = red || c.RectFilled.Color == a.p.Danger
		}
		if c.Kind == command.TextCmd {
			reason = reason || strings.Contains(c.Text.String, "Blocked reason")
			if c.Text.String == "⬟ Blocked" {
				click = image.Pt(c.Rect.X+5, c.Rect.Y+5)
			}
		}
	}
	if !green || !red || !reason || click == (image.Point{}) {
		t.Fatal("status presentation missing", green, red, reason, click)
	}
	h.Frame(false)
	if text(d.Editors["Blocked"]) != "false" || text(d.Editors["Ready"]) != "true" || text(d.Editors["BlockedReason"]) != "Waiting" {
		t.Fatal("status toggle changed another value or discarded its reason")
	}
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd && strings.Contains(c.Text.String, "Blocked reason") {
			t.Fatal("inactive blocked reason is still shown")
		}
	}
}
