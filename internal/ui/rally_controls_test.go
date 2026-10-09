//go:build nucular_headless

package ui

import (
	"fmt"
	"image"
	"slices"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"golang.org/x/mobile/event/mouse"
)

func groupedControlView() *rallyView {
	v := newRallyView(rally.FindPage("userstories"))
	v.Items = []rally.Object{
		{"_ref": "/story/1", "Name": "one", "DragAndDropRank": "A", "Owner": map[string]any{"_ref": "/user/1", "_refObjectName": "Alex"}},
		{"_ref": "/story/2", "Name": "two", "DragAndDropRank": "B", "Owner": map[string]any{"_ref": "/user/2", "_refObjectName": "Alex"}},
		{"_ref": "/story/3", "Name": "three", "DragAndDropRank": "C", "Owner": map[string]any{"_ref": "/user/1", "_refObjectName": "Alex"}},
		{"_ref": "/story/4", "Name": "four", "DragAndDropRank": "D"},
	}
	return v
}

func TestV46GroupingChangesRebuildOrdering(t *testing.T) {
	v := groupedControlView()
	a := &App{}
	before := a.rallySignature(v)
	v.filtered()
	revision := v.filterRevision
	v.applySavedView(settings.SavedView{Group: "Owner", Sort: "Rank"})
	items := v.filtered()
	if v.filterRevision == revision || a.rallySignature(v) == before {
		t.Fatal("group change reused ordering or paging identity")
	}
	// Distinct references sharing a display name must form separate groups.
	if items[0].Ref("Owner") != "/user/1" || items[1].Ref("Owner") != "/user/1" || items[2].Ref("Owner") != "/user/2" {
		t.Fatal("reference groups did not remain contiguous", items)
	}
}

func TestV46GroupHeadersCountLoadedItems(t *testing.T) {
	a := &App{p: colors(false)}
	v := groupedControlView()
	v.Group = "Owner"
	v.Items = append(v.Items, rally.Object{"_ref": "/story/5", "Name": "five", "DragAndDropRank": "E"})
	v.PageSize = 2
	v.Page = 3 // Group continuation starts on this page.
	items := v.filtered()
	h := nucular.NewHeadlessHarness(0, image.Pt(900, 600), func(w *nucular.Window) { a.table(w, v, items) })
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	h.Frame(true)
	var labels []string
	for _, c := range h.Commands() {
		if c.Kind == command.TextCmd {
			labels = append(labels, c.Text.String)
		}
	}
	if !strings.Contains(strings.Join(labels, "\n"), "Unassigned (2 loaded)") {
		t.Fatal("missing scoped group header on a continuation page", labels)
	}
}

func TestV46RefreshRebuildsFilteredObjects(t *testing.T) {
	a, v, server := boardMutationFixture(t)
	v.filtered()
	server.mu.Lock()
	for _, o := range server.items {
		o["Name"] = "fresh value"
		o["Owner"] = nil
	}
	server.mu.Unlock()
	a.refreshRallyItems(v)
	drain(t, a, func() bool { return !v.Loading })
	items := v.filtered()
	if len(items) != 3 || items[0].String("Name") != "fresh value" || v.groupCounts[""] != 3 {
		t.Fatal("refresh published new records but retained old filtered objects or counts", items, v.groupCounts)
	}
}

func TestV46PageModesAndGroupingLabels(t *testing.T) {
	for _, tc := range []struct {
		page  string
		modes []string
	}{
		{"teamboard", []string{"list", "board", "charts"}},
		{"iterationstatus", []string{"list", "board", "charts"}},
		{"portfoliokanban", []string{"board", "charts"}},
		{"timeboxes", []string{"list", "charts"}},
		{"teamplan", []string{"list", "planning", "charts"}},
		{"timeline", []string{"list", "timeline", "charts"}},
		{"tasks", nil}, {"myrally", nil},
	} {
		v := newRallyView(rally.FindPage(tc.page))
		v.Mode = "list"
		var modes []string
		for _, c := range rallyModeChoices(v) {
			modes = append(modes, c.Key)
		}
		if !slices.Equal(modes, tc.modes) {
			t.Fatalf("%s: %v", tc.page, modes)
		}
	}
	v := newRallyView(rally.FindPage("teamboard"))
	v.Group = "c_TeamRegion"
	v.Fields = []rally.Field{{Name: "Project", DisplayName: "Team"}, {Name: "c_TeamRegion", DisplayName: "Sales Region"}}
	keys, labels := rallyGroupChoices(v)
	if slices.Contains(keys, "ScheduleState") || !slices.Contains(keys, "Project") || labels[index(keys, v.Group)] != "Sales Region" || labels[index(keys, "Project")] != "Team" {
		t.Fatal("group choices lost identities or display names", keys, labels)
	}
	v.Group = "ScheduleState"
	keys, labels = rallyGroupChoices(v)
	if labels[index(keys, v.Group)] != "Schedule state" {
		t.Fatal("saved group lost", keys, labels)
	}
}

func populatedFilterView() *rallyView {
	v := newRallyView(rally.FindPage("teamboard"))
	setText(v.Search, "part of title")
	v.Timebox, v.OwnerFilter, v.StateFilter = "/iteration/1", "/user/1", "Accepted"
	v.OnlyBlocked, v.OnlyReady, v.QueryApplied = true, true, "(PlanEstimate > 1)"
	setText(v.Query, v.QueryApplied)
	return v
}

func TestV46ActiveFilterRemoval(t *testing.T) {
	a := &App{scopeChoices: map[string]*pickerChoices{
		"OwnerFilter": makePickerChoices([]rally.Object{{"_ref": "/user/1", "Name": "Alex"}}),
	}}
	original := a.activeRallyFilters(populatedFilterView())
	if len(original) != 7 || original[2].Label != "Owner is Alex" {
		t.Fatal(original)
	}
	for _, remove := range original {
		v := populatedFilterView()
		if !v.removeFilter(remove.Key) {
			t.Fatal(remove.Key)
		}
		want := slices.DeleteFunc(slices.Clone(original), func(f rallyActiveFilter) bool { return f.Key == remove.Key })
		if actual := a.activeRallyFilters(v); !slices.Equal(actual, want) {
			t.Fatal(remove.Key, actual, want)
		}
		if remove.Key == "query" && text(v.Query) != "" {
			t.Fatal("query editor retained removed filter")
		}
	}
	if populatedFilterView().removeFilter("unknown") || rallyFilterCaption(false, 1) != "Show 1 Filter" || rallyFilterCaption(true, 7) != "Hide 7 Filters" {
		t.Fatal("invalid filter or count label")
	}
	long := populatedFilterView()
	long.QueryApplied = strings.Repeat("field ", 100000)
	filters := a.activeRallyFilters(long)
	if len(filters[6].Label) > 210 || len(long.QueryApplied) != 600000 {
		t.Fatal("chip preview was unbounded or changed the applied query")
	}
}

func TestV46RallyControlsFitDisplayScales(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		for _, width := range []int{320, 640, 1000} {
			t.Run(fmt.Sprintf("%g/%d", scale, width), func(t *testing.T) {
				a := &App{p: colors(false)}
				v := populatedFilterView()
				v.Spec = rally.FindPage("teamplan")
				v.Mode = "list"
				var click bool
				var pos image.Point
				h := nucular.NewHeadlessHarness(0, image.Pt(int(float64(width)*scale), int(600*scale)), func(w *nucular.Window) {
					m := &w.Input().Mouse
					m.Pos = pos
					m.Buttons[mouse.ButtonLeft].Clicked, m.Buttons[mouse.ButtonLeft].Down = click, false
					m.Buttons[mouse.ButtonLeft].ClickedPos = pos
					a.drawRallyModes(w, v)
					a.drawRallyGrouping(w, v)
					a.drawRallyFilterChips(w, v, a.activeRallyFilters(v))
				})
				style := makeStyle(a.p, 13)
				style.Scale(scale)
				h.Master().SetStyle(style)
				h.Frame(false)
				var plan, owner image.Point
				blue := false
				for _, c := range h.Commands() {
					if c.Kind == command.RectFilledCmd && c.RectFilled.Color == hex(0x3272d9) {
						blue = true
					}
					if c.Kind != command.TextCmd {
						continue
					}
					if c.Rect.X < 0 || c.Rect.X+c.Rect.W > int(float64(width)*scale) {
						t.Fatal("text extends past viewport", c.Text.String, c.Rect)
					}
					if c.Text.String == "Planning" {
						plan = image.Pt(c.Rect.X+2, c.Rect.Y+2)
					}
					if strings.HasPrefix(c.Text.String, "Owner is ") {
						owner = image.Pt(c.Rect.X+2, c.Rect.Y+2)
					}
				}
				if !blue || plan == (image.Point{}) || owner == (image.Point{}) {
					t.Fatal("missing active mode, return mode, or filter", blue, plan, owner)
				}
				pos, click = plan, true
				h.Frame(false)
				if v.Mode != "planning" {
					t.Fatal("planning was not recoverable")
				}
				// Mode-specific controls can change the following rows' positions.
				// Locate the current chip before simulating the next user click.
				click = false
				h.Frame(false)
				owner = image.Point{}
				for _, c := range h.Commands() {
					if c.Kind == command.TextCmd && strings.HasPrefix(c.Text.String, "Owner is ") {
						owner = image.Pt(c.Rect.X+2, c.Rect.Y+2)
					}
				}
				if owner == (image.Point{}) {
					t.Fatal("owner chip disappeared after switching mode")
				}
				click = true
				pos = owner
				h.Frame(false)
				if v.OwnerFilter != "" || v.StateFilter == "" || text(v.Search) == "" {
					t.Fatal("chip click removed wrong filters")
				}
			})
		}
	}
}
