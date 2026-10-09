//go:build nucular_headless

package ui

import (
	"image"
	"reflect"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestV43TabInsertionBoundaries(t *testing.T) {
	for _, tc := range []struct {
		name                                      string
		x, first, stride, count, from, to, marker int
	}{
		{"before first", -100, 20, 100, 4, 3, 0, 20},
		{"left half stays", 164, 20, 100, 4, 1, 1, 120},
		{"right half stays", 176, 20, 100, 4, 1, 1, 220},
		{"before last", 325, 20, 100, 4, 0, 2, 320},
		{"after last", 900, 20, 100, 4, 0, 3, 420},
		{"scrolled gap", 88, -150, 120, 4, 3, 2, 90},
		{"scaled", 590, 20, 200, 4, 0, 2, 620},
	} {
		t.Run(tc.name, func(t *testing.T) {
			to, marker := tabDropTarget(tc.x, tc.first, tc.stride, tc.count, tc.from)
			if to != tc.to || marker != tc.marker {
				t.Fatalf("got %d/%d, want %d/%d", to, marker, tc.to, tc.marker)
			}
		})
	}
}

func TestV43TabDragDrawAndDrop(t *testing.T) {
	a := transferFixture()
	a.p = colors(false)
	for _, title := range []string{"First", "Second", "Third", "Fourth"} {
		a.state.Open(workspace.New, title, "", "")
	}
	a.state.Active = a.state.Tabs[0].ID
	var pos, origin image.Point
	var down, clicked bool
	var commands []command.Command
	h := nucular.NewHeadlessHarness(0, image.Pt(900, 200), func(w *nucular.Window) {
		m := &w.Input().Mouse
		m.Pos = pos
		m.Buttons[mouse.ButtonLeft].Down = down
		m.Buttons[mouse.ButtonLeft].Clicked = clicked
		m.Buttons[mouse.ButtonLeft].ClickedPos = origin
		a.drawTabs(w)
		commands = append(commands[:0], w.Commands().Commands...)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	h.Frame(false)
	centers := map[string]image.Point{}
	for _, c := range commands {
		if c.Kind == command.TextCmd {
			centers[c.Text.String] = image.Pt(c.Rect.X+4, c.Rect.Y+c.Rect.H/2)
		}
	}
	if centers["First"].X == 0 || centers["Fourth"].X == 0 {
		t.Fatal("missing tab geometry", centers)
	}
	origin, pos = centers["First"], centers["First"]
	down, clicked = true, true
	h.Frame(false)
	if a.dragTab != a.state.Tabs[0].ID || a.dragTabMoving {
		t.Fatal("press did not arm drag")
	}
	// The right half of Fourth means insert after it, even without a release
	// directly over its label or close button.
	stride := centers["Second"].X - centers["First"].X
	pos = image.Pt(centers["Fourth"].X+stride*2/3, origin.Y)
	clicked = false
	h.Frame(false)
	var border, marker bool
	for _, c := range commands {
		if c.Kind == command.RectFilledCmd && c.RectFilled.Color == a.p.Accent {
			border = border || c.Rect.W > 20 && c.Rect.H > 20
			marker = marker || c.Rect.W == 2 && c.Rect.H > 10
		}
	}
	if !a.dragTabMoving || !border || !marker {
		t.Fatal("drag feedback missing", a.dragTabMoving, border, marker)
	}
	down, clicked = false, true
	h.Frame(false)
	var titles []string
	for _, tab := range a.state.Tabs {
		titles = append(titles, tab.Title)
	}
	if !reflect.DeepEqual(titles, []string{"Second", "Third", "Fourth", "First"}) || a.state.Current().Title != "First" || a.dragTab != "" || a.dragTabMoving {
		t.Fatal("incorrect drop or drag state", titles, a.dragTab, a.dragTabMoving)
	}
	// Releasing outside the strip cancels and leaves the previous order intact.
	origin, pos = centers["Second"], centers["Second"]
	down, clicked = true, true
	h.Frame(false)
	pos = image.Pt(origin.X+20, 150)
	clicked = false
	h.Frame(false)
	down, clicked = false, true
	h.Frame(false)
	for i, title := range titles {
		if a.state.Tabs[i].Title != title {
			t.Fatal("outside drop changed order")
		}
	}
	if a.dragTab != "" || a.dragTabMoving {
		t.Fatal("outside drop retained drag state")
	}
	// A close-target press must remain a close action, even if the pointer
	// subsequently moves sideways while held.
	pos = image.Pt(centers["First"].X-27+stride-14, origin.Y)
	origin = pos
	down, clicked = true, true
	h.Frame(false)
	if a.dragTab != "" {
		t.Fatal("close-target press armed a drag")
	}
}
