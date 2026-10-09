//go:build fltk_headless

package ui

import (
	"context"
	"image"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestRallyNavigationCollapseAndRestore(t *testing.T) {
	a := transferFixture()
	a.p, a.prefs = colors(false), settings.Defaults()
	v := newRallyView(rally.FindPage("teamboard"))
	var click image.Point
	h := desktop.NewHeadlessHarness(0, image.Pt(1000, 500), func(w *desktop.Window) {
		if click != (image.Point{}) {
			m := &w.Input().Mouse
			m.Pos = click
			m.Buttons[mouse.ButtonLeft].Clicked = true
			m.Buttons[mouse.ButtonLeft].ClickedPos = click
		}
		a.rallyNav(w, v)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	labels := func() map[string]image.Point {
		out := map[string]image.Point{}
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd {
				out[c.Text.String] = image.Pt(c.Rect.X+2, c.Rect.Y+2)
			}
		}
		return out
	}
	settle := func() {
		for range 3 {
			h.Frame(false)
		}
	}
	settle()
	shown := labels()
	if shown["Home"] == (image.Point{}) || shown["Iteration Status"] == (image.Point{}) {
		t.Fatal("missing navigation")
	}
	// Click the small minus beside the first row, preserving the second row.
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd && c.Text.String == "−" && c.Rect.Y < shown["Home"].Y {
			click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
			break
		}
	}
	if click == (image.Point{}) {
		t.Fatal("missing row collapse button")
	}
	h.Frame(false)
	click = image.Point{}
	settle()
	hidden := labels()
	if !a.rallyRowHidden("sections") || a.rallyRowHidden("pages") || hidden["Home"] != (image.Point{}) || hidden["Iteration Status"] == (image.Point{}) || hidden["+ Sections"] == (image.Point{}) {
		t.Fatal("section row did not collapse independently")
	}
	a.setRallyRowHidden("pages", true)
	settle()
	if labels()["Iteration Status"] != (image.Point{}) || labels()["+ Pages"] == (image.Point{}) {
		t.Fatal("page row did not collapse independently")
	}
	click = labels()["+ Sections"]
	h.Frame(false)
	click = image.Point{}
	settle()
	if a.rallyRowHidden("sections") || !a.rallyRowHidden("pages") || labels()["Home"] == (image.Point{}) {
		t.Fatal("restoring sections changed pages")
	}
	a.setRallyRowHidden("sections", true)
	settle()
	click = labels()["+ All"]
	h.Frame(false)
	click = image.Point{}
	settle()
	if a.rallyRowHidden("sections") || a.rallyRowHidden("pages") || labels()["Home"] == (image.Point{}) {
		t.Fatal("restore all failed")
	}
	p, err := settings.Apply(settings.Defaults(), settings.Diff(settings.Defaults(), a.preferencesQueued))
	if err != nil || len(p.RallyHiddenRows) != 0 {
		t.Fatal("row preferences did not round trip", err)
	}
}

func TestRallyRowsReclaimHeightWithoutChangingState(t *testing.T) {
	a := transferFixture()
	a.p, a.prefs = colors(false), settings.Defaults()
	v := newRallyView(rally.FindPage("teamboard"))
	v.Group, v.Timebox, v.OwnerFilter = "Owner", "/iteration/1", "/user/1"
	setText(v.Search, "find me")
	var boardY int
	h := desktop.NewHeadlessHarness(0, image.Pt(1000, 900), func(w *desktop.Window) {
		a.drawRallyRowRestore(w, v)
		for _, row := range a.rallyRows(v) {
			a.rallyRow(w, v, row.ID, func(w *desktop.Window) { w.Row(28).Dynamic(1); w.Label(row.Label, "LC") })
		}
		boardY = w.LayoutNextRowY()
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	for range 3 {
		h.Frame(false)
	}
	expanded := boardY
	for _, row := range a.rallyRows(v) {
		a.setRallyRowHidden(row.ID, true)
	}
	for range 3 {
		h.Frame(false)
	}
	if expanded-boardY < 300 {
		t.Fatalf("collapse reclaimed only %d px", expanded-boardY)
	}
	if v.Group != "Owner" || v.Timebox != "/iteration/1" || v.OwnerFilter != "/user/1" || text(v.Search) != "find me" {
		t.Fatal("collapse changed board state")
	}
	p, err := settings.Apply(settings.Defaults(), settings.Diff(settings.Defaults(), a.preferencesQueued))
	if err != nil || len(p.RallyHiddenRows) != len(a.rallyRows(v)) {
		t.Fatal("hidden row choices did not persist", p.RallyHiddenRows, err)
	}
	// A legacy preference hides just its original two rows and can be restored individually.
	a.prefs.RallyHiddenRows = nil
	a.prefs.RallyNavHidden = true
	a.setRallyRowHidden("sections", false)
	if a.rallyRowHidden("sections") || !a.rallyRowHidden("pages") || a.rallyRowHidden("search") {
		t.Fatal("legacy preference migration changed other rows")
	}
}

func TestRallyRowMeasuresWrappedControls(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		a := &App{p: colors(false)}
		v := newRallyView(rally.FindPage("teamboard"))
		h := desktop.NewHeadlessHarness(0, image.Pt(int(300*scale), int(800*scale)), func(w *desktop.Window) {
			a.rallyRow(w, v, "density", func(w *desktop.Window) { a.drawBoardDisplayControls(w, v) })
			w.Row(20).Dynamic(1)
			w.Label("After controls", "LC")
		})
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		style.Font = typeFace(int(13*scale), regularFont)
		h.Master().SetStyle(style)
		for range 3 {
			h.Frame(false)
		}
		if a.rallyRowHeights["teamboard/density"] < int(50*scale) {
			t.Fatal("wrapped row height was lost", scale, a.rallyRowHeights)
		}
		seen := map[string]int{}
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd {
				seen[c.Text.String] = c.Rect.Y
			}
		}
		if seen["Page settings"] == 0 || seen["After controls"] <= seen["Page settings"] {
			t.Fatal("wrapped row clipped or overlapped following content", scale, seen)
		}
	}
}

func TestNewTabRecentsInitiallyCollapsed(t *testing.T) {
	a := &App{ctx: context.Background(), state: workspace.NewState(), p: colors(false), newFolder: textEditor("", false)}
	a.prefs.RecentFolders = []string{"/recent-project"}
	var click image.Point
	h := desktop.NewHeadlessHarness(0, image.Pt(900, 700), func(w *desktop.Window) {
		if click != (image.Point{}) {
			m := &w.Input().Mouse
			m.Pos = click
			m.Buttons[mouse.ButtonLeft].Clicked = true
			m.Buttons[mouse.ButtonLeft].ClickedPos = click
		}
		a.drawNew(w)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd {
			if c.Text.String == "/recent-project" {
				t.Fatal("recent folder should begin collapsed")
			}
			if c.Text.String == "Recent folders" {
				click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
			}
		}
	}
	if click == (image.Point{}) {
		t.Fatal("missing recent folders disclosure")
	}
	h.Frame(false)
	click = image.Point{}
	h.Frame(false)
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd && c.Text.String == "/recent-project" {
			return
		}
	}
	t.Fatal("recent folder cannot be expanded")
}
