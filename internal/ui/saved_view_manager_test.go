//go:build fltk_headless

package ui

import (
	"context"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestV68SavedViewOperations(t *testing.T) {
	a, v := keyboardBoard(t, 3)
	v.StructuredFilters = []settings.RallyFilter{{Field: "Tags", Operator: "contains", Value: "keep"}}
	v.Display.Density = "Compact"
	original := v.savedView("Mine")
	if err := a.changeSavedView(nil, &original); err != nil {
		t.Fatal(err)
	}
	original.Columns[0] = "changed source"
	if a.prefs.Views[0].Columns[0] == original.Columns[0] {
		t.Fatal("new view aliases source")
	}
	original = *a.findSavedView(v.Spec.ID, "Mine")
	v.ViewName = "Mine"
	setText(v.Search, "unsaved live filter")
	renamed := original.Clone()
	renamed.Name, renamed.Mode = "Renamed", "list"
	if err := a.changeSavedView(&original, &renamed); err != nil {
		t.Fatal(err)
	}
	if v.ViewName != "Renamed" || text(v.Search) != "unsaved live filter" || v.Mode != "board" {
		t.Fatal("rename discarded live state or lost association")
	}
	if err := a.changeSavedView(&original, nil); err == nil {
		t.Fatal("stale deletion accepted")
	}
	before := a.prefs.Views
	copy := renamed.Clone()
	copy.Name = "Copy"
	if err := a.changeSavedView(nil, &copy); err != nil {
		t.Fatal(err)
	}
	copy.Filters[0].Value, copy.Display.Density = "changed", "Comfortable"
	if len(before) != 1 || before[0].Name != "Renamed" || a.prefs.Views[1].Filters[0].Value != "keep" || a.prefs.Views[1].Display.Density != "Compact" {
		t.Fatal("copy mutated published snapshots")
	}
	for _, name := range []string{"", "Standard View", "standard view", strings.Repeat("x", 121), "bad\nname", "Copy"} {
		bad := renamed.Clone()
		bad.Name = name
		if err := a.changeSavedView(nil, &bad); err == nil {
			t.Fatal("invalid/duplicate name accepted", name)
		}
	}
	bad := renamed.Clone()
	bad.Page = "customviews"
	if err := a.changeSavedView(nil, &bad); err == nil {
		t.Fatal("recursive manager view accepted")
	}
	current := *a.findSavedView(v.Spec.ID, "Renamed")
	newer := current.Clone()
	newer.Query = "(Blocked = true)"
	if err := a.changeSavedView(&current, &newer); err != nil {
		t.Fatal(err)
	}
	if err := a.changeSavedView(&current, nil); err == nil {
		t.Fatal("old dialog overwrote newer data")
	}
	if err := a.changeSavedView(&newer, nil); err != nil {
		t.Fatal(err)
	}
	if v.ViewName != "" || text(v.Search) != "unsaved live filter" || len(a.prefs.Views) != 1 {
		t.Fatal("deletion changed current filters or other view")
	}
}

func TestV68StandardViewReset(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	baseline := v.savedView("")
	v.applySavedView(settings.SavedView{Name: "Changed", Page: "teamboard", Query: "(Blocked = true)", Search: "text", Group: "Owner", Mode: "list", Timebox: "/iteration/1", Owner: "/user/2", State: "Accepted", Blocked: true, Ready: true, Sort: "Name", Descending: true, Columns: []string{"Name"}, CardFields: []string{"Owner"}, Filters: []settings.RallyFilter{{Field: "Tags", Operator: "contains", Value: "test"}}, Display: (&settings.BoardDisplay{Density: "Compact", ColorBy: "Owner", WIPLimit: 2, AgeDays: 9}).Copy()})
	setText(v.Query, "unfinished query")
	v.applySavedView(settings.SavedView{})
	if !savedViewEqual(v.savedView(""), baseline) || text(v.Query) != "" || v.Page != 1 {
		t.Fatal("Standard View did not reset the complete baseline", v.savedView(""))
	}
}

func TestV68SavedViewManagerProjection(t *testing.T) {
	a := presetApp(t)
	v := newRallyView(rally.FindPage("customviews"))
	for i := range 10000 {
		a.prefs.Views = append(a.prefs.Views, settings.SavedView{Name: fmt.Sprintf("View %05d", i), Page: "teamboard", Mode: "board"})
	}
	setText(v.Search, "View 09999")
	a.prepareSavedViewManager(v)
	setText(v.Search, "View 00001")
	a.prepareSavedViewManager(v)
	drain(t, a, func() bool { return !v.SavedViewManager.loading })
	if len(v.SavedViewManager.rows) != 1 || v.SavedViewManager.rows[0].Name != "View 00001" {
		t.Fatal("older search replaced current result")
	}
	ctx, cancel := context.WithCancel(a.ctx)
	cancel()
	if _, err := projectSavedViews(ctx, a.prefs.Views, ""); err == nil {
		t.Fatal("cancelled scan continued")
	}
	setText(v.Search, "")
	a.prepareSavedViewManager(v)
	drain(t, a, func() bool { return !v.SavedViewManager.loading })
	rows := v.SavedViewManager.rows
	h := desktop.NewHeadlessHarness(0, image.Pt(800, 650), func(w *desktop.Window) { a.drawSavedViewManager(w, v) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	visible := 0
	for _, cmd := range h.Commands() {
		if cmd.Kind == command.TextCmd && strings.HasPrefix(cmd.Text.String, "View 0") {
			visible++
		}
	}
	if visible == 0 || visible > 16 || &rows[0] != &v.SavedViewManager.rows[0] {
		t.Fatal("unbounded draw or idle re-scan", visible)
	}
	saved := *a.findSavedView("teamboard", "View 00000")
	if err := a.changeSavedView(&saved, nil); err != nil {
		t.Fatal(err)
	}
	a.prepareSavedViewManager(v)
	drain(t, a, func() bool { return !v.SavedViewManager.loading })
	if len(v.SavedViewManager.rows) != 9999 || v.SavedViewManager.rows[0].Name != "View 00001" {
		t.Fatal("manager retained deleted record")
	}
	a.prepareSavedViewManager(v)
	v.Closed = true
	if v.SavedViewManager.cancel != nil {
		v.SavedViewManager.cancel()
	}
}

func TestV68CustomViewsOfflineAndOpen(t *testing.T) {
	a := presetApp(t)
	var requests atomic.Int32
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		fmt.Fprint(w, `{"QueryResult":{"Results":[]}}`)
	}))
	defer s.Close()
	a.rallyClient, _ = rally.New(s.URL, "fixture", nil)
	a.openRally("customviews")
	v := a.rallyViews[a.state.Active]
	a.refreshRally(v)
	a.refreshRallyItems(v)
	a.requestRallyPage(v, 1, true)
	h := desktop.NewHeadlessHarness(0, image.Pt(800, 600), func(w *desktop.Window) { a.drawRallyContent(w, v) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	h.Frame(false)
	drain(t, a, func() bool { return !v.SavedViewManager.loading })
	if requests.Load() != 0 || v.Loading || v.Spec.ID != "customviews" {
		t.Fatal("manager queried Rally")
	}
	a.rallyClient = nil
	view := newRallyView(rally.FindPage("teamboard")).savedView("Focus")
	view.Query, view.Mode, view.Group, view.Search = "(Blocked = true)", "list", "Owner", "saved"
	if err := a.changeSavedView(nil, &view); err != nil {
		t.Fatal(err)
	}
	a.openSavedView(view)
	doc := a.rallyViews[a.state.Active]
	if doc.Spec.ID != "teamboard" || doc.QueryApplied != view.Query || text(doc.Search) != "saved" || doc.Mode != "list" {
		t.Fatal("opening saved view lost settings")
	}
	other := view.Clone()
	other.Name = "Other"
	if err := a.changeSavedView(nil, &other); err != nil {
		t.Fatal(err)
	}
	blocked := presetApp(t) // No window: an unanswered prompt cannot discard a draft.
	blocked.prefs.Views = []settings.SavedView{view.Clone(), other.Clone()}
	blockedID := blocked.state.Open(workspace.Rally, "Team Board", "", "teamboard")
	draft := newRallyView(rally.FindPage("teamboard"))
	draft.ViewName = "Focus"
	draft.Detail = makeDetail(rally.Object{"Name": "Original"}, "HierarchicalRequirement", false)
	setText(draft.Detail.Editors["Name"], "Unsent edit")
	blocked.rallyViews[blockedID] = draft
	blocked.openSavedView(other)
	if text(draft.Detail.Editors["Name"]) != "Unsent edit" || draft.ViewName != "Focus" {
		t.Fatal("opening view discarded dirty work item")
	}
	managerID := a.state.Open(workspace.Rally, "Custom Views", "", "customviews")
	a.rallyViews[managerID] = v
	setText(v.Search, "retained search")
	copy := transferFixture()
	if err := copy.installTransfer(transferJSON(t, a.tabSnapshot(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	if text(copy.rallyViews[managerID].Search) != "retained search" {
		t.Fatal("manager search lost on transfer")
	}
}

func TestV68SavedViewControls(t *testing.T) {
	a := presetApp(t)
	v := newRallyView(rally.FindPage("teamboard"))
	original := v.savedView("Mine")
	if err := a.changeSavedView(nil, &original); err != nil {
		t.Fatal(err)
	}
	v.ViewName = "Mine"
	var click image.Point
	h := desktop.NewHeadlessHarness(0, image.Pt(800, 600), func(w *desktop.Window) {
		if click != (image.Point{}) {
			m := &w.Input().Mouse
			m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
			m.Buttons[mouse.ButtonLeft].Clicked = true
		}
		a.drawSavedViewActions(w, v)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	h.Frame(false)
	for _, cmd := range h.Commands() {
		if cmd.Kind == command.TextCmd && cmd.Text.String == "View actions" {
			click = image.Pt(cmd.Rect.X+2, cmd.Rect.Y+2)
		}
	}
	if click == (image.Point{}) {
		t.Fatal("view menu missing")
	}
	h.Frame(false)
	click = image.Point{}
	h.Frame(false)
	seen := map[string]bool{}
	for _, cmd := range h.Commands() {
		if cmd.Kind == command.TextCmd {
			seen[cmd.Text.String] = true
		}
	}
	for _, s := range []string{"Add new view…", "Edit / rename current view…", "Copy current view…", "Delete current view…", "Manage views"} {
		if !seen[s] {
			t.Fatal("missing menu action", s, seen)
		}
	}
	// Mutating a value returned to the UI cannot change the saved record.
	lookup := a.findSavedView("teamboard", "Mine")
	lookup.Columns[0] = "changed"
	if reflect.DeepEqual(*lookup, a.prefs.Views[0]) {
		t.Fatal("lookup aliases saved view")
	}
}

func TestV68SavedViewDialogCopiesConfiguration(t *testing.T) {
	a := presetApp(t)
	original := newRallyView(rally.FindPage("teamboard")).savedView("Original")
	original.Query, original.Search = "(Blocked = true)", "needle"
	original.Filters = []settings.RallyFilter{{Field: "Tags", Operator: "contains", Value: "important"}}
	original.Display = (&settings.BoardDisplay{Density: "Compact", ColorBy: "Owner", WIPLimit: 7, AgeDays: 4}).Copy()
	if err := a.changeSavedView(nil, &original); err != nil {
		t.Fatal(err)
	}
	var click image.Point
	h := desktop.NewHeadlessHarness(0, image.Pt(850, 700), func(w *desktop.Window) {
		if click != (image.Point{}) {
			m := &w.Master().Input().Mouse
			m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
			m.Buttons[mouse.ButtonLeft].Clicked = true
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	press := func(label string) {
		t.Helper()
		deadline := time.Now().Add(2 * time.Second)
		for {
			h.Master().Lock()
			h.Frame(false)
			for _, cmd := range h.Commands() {
				if cmd.Kind == command.TextCmd && cmd.Text.String == label {
					click = image.Pt(cmd.Rect.X+cmd.Rect.W/2, cmd.Rect.Y+cmd.Rect.H/2)
					break
				}
			}
			h.Master().Unlock()
			if click != (image.Point{}) {
				break
			}
			if time.Now().After(deadline) {
				t.Fatal("dialog action absent", label)
			}
			time.Sleep(time.Millisecond)
		}
		h.Master().Lock()
		h.Frame(false)
		h.Master().Unlock()
		click = image.Point{}
	}
	copy := original.Clone()
	copy.Name = "  Copied  "
	a.savedViewDialog(nil, copy, nil, false)
	press("Save view")
	got := a.findSavedView("teamboard", "Copied")
	copy.Name = "Copied"
	if got == nil || !reflect.DeepEqual(*got, copy) {
		t.Fatal("copy dialog lost settings", got)
	}
	copy.Name = "Cancelled"
	a.savedViewDialog(nil, copy, nil, false)
	press("Cancel")
	if len(a.prefs.Views) != 2 {
		t.Fatal("cancel added a view")
	}
}
