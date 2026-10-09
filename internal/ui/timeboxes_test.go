//go:build fltk_headless

package ui

import (
	"fmt"
	"image"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"golang.org/x/mobile/event/mouse"
)

func timeboxRow(kind, id, name, project string, start, end time.Time) rally.Object {
	a, b := "StartDate", "EndDate"
	if kind == "release" {
		a, b = "ReleaseStartDate", "ReleaseDate"
	}
	return rally.Object{"_ref": "/" + kind + "/" + id, "Name": name, "Project": map[string]any{"_ref": project}, a: start.Format(time.RFC3339), b: end.Format(time.RFC3339)}
}

func TestV69TimeboxChoices(t *testing.T) {
	now := time.Date(2026, 10, 9, 12, 0, 0, 0, time.UTC)
	rows := []rally.Object{
		timeboxRow("iteration", "sibling", "Sprint 1", "/project/2", now.Add(-time.Hour), now.Add(time.Hour)),
		timeboxRow("iteration", "selected", "Sprint 1", "/project/1", now.Add(-time.Hour), now.Add(time.Hour)),
		timeboxRow("iteration", "future", "Sprint 2", "/project/1", now.Add(time.Hour), now.Add(3*time.Hour)),
		timeboxRow("iteration", "old", "Sprint 0", "/project/1", now.Add(-3*time.Hour), now.Add(-2*time.Hour)),
		{"_ref": "/iteration/bad", "Name": "No dates", "StartDate": "invalid"},
	}
	p := makeTimeboxChoices(rows, "/project/1", now)
	if len(p.choices) != 4 || p.choices[0].Name != "Sprint 2" || p.choices[1].Ref != "/iteration/selected" || p.choices[3].Name != "No dates" {
		t.Fatal("wrong date order, deduplication or project default", p.choices)
	}
	if !strings.Contains(p.choices[1].Label, "2026-10-09") || !strings.Contains(p.choices[1].Label, "Current") || strings.Contains(p.choices[0].Label, "Current") || !p.nextChange.Equal(now.Add(time.Hour)) {
		t.Fatal("wrong dates/current boundary", p.choices, p.nextChange)
	}
	for _, ref := range []string{"/iteration/selected", "/iteration/sibling"} {
		labels, i, _ := p.options(ref, "", false, false)
		if !strings.HasPrefix(labels[i], "Sprint 1") {
			t.Fatal("legacy sibling ref lost", labels, i)
		}
	}
	if n := testing.AllocsPerRun(100, func() { p.options("/iteration/selected", "Sprint 1", false, false) }); n != 0 {
		t.Fatal("known selection rebuilt choices", n)
	}
	labels, i, _ := p.options("/iteration/missing", "Missing", true, false)
	again, j, _ := p.options("/iteration/missing", "Missing", true, false)
	if labels[i] != "Selected: Missing" || i != j || &labels[0] != &again[0] {
		t.Fatal("missing selection not retained/cached")
	}
	for i := 0; i < 100; i++ {
		p.options(fmt.Sprint(i), "", false, false)
	}
	if len(p.missing) > 16 {
		t.Fatal("missing cache unbounded")
	}
	after := makeTimeboxChoices(rows, "/project/1", now.Add(time.Hour))
	if !strings.Contains(after.choices[0].Label, "Current") || strings.Contains(after.choices[1].Label, "Current") {
		t.Fatal("boundary did not update marker")
	}
	r := makeTimeboxChoices([]rally.Object{timeboxRow("release", "q4", "Q4", "/project/1", now.Add(-time.Hour), now.Add(time.Hour))}, "/project/1", now)
	if !strings.Contains(r.choices[0].Label, "Current") || !strings.Contains(r.choices[0].Label, "2026-10-09") {
		t.Fatal("release dates not read", r.choices)
	}
}

func TestV69TimeboxQueryAndDefaults(t *testing.T) {
	a := presetApp(t)
	a.prefs.RallyProject = "/project/1"
	now := time.Now()
	a.iterations = []rally.Object{
		timeboxRow("iteration", "sibling", `Sprint "1"`, "/project/2", now.Add(-time.Hour), now.Add(time.Hour)),
		timeboxRow("iteration", "main", `Sprint "1"`, "/project/1", now.Add(-time.Hour), now.Add(time.Hour)),
	}
	a.releases = []rally.Object{timeboxRow("release", "main", "Q4", "/project/1", now.Add(-time.Hour), now.Add(time.Hour))}
	v := newRallyView(rally.FindPage("teamboard"))
	v.Timebox, v.ReleaseTimebox = "/iteration/sibling", "/release/main"
	a.prepareTimeboxNames(v)
	q := a.rallyQuery(v)
	if !strings.Contains(q.Expression, rally.Eq("Iteration.Name", `Sprint "1"`)) || !strings.Contains(q.Expression, rally.Eq("Release.Name", "Q4")) || strings.Contains(q.Expression, "sibling") {
		t.Fatal(q)
	}
	a.newArtifact(v)
	if v.Detail.Original.Ref("Iteration") != "/iteration/main" || v.Detail.Original.Ref("Release") != "/release/main" {
		t.Fatal("new item used sibling team's timebox", v.Detail.Original)
	}
	v.Timebox, v.TimeboxName = "/iteration/unknown", ""
	if !strings.Contains(a.rallyQuery(v).Expression, rally.Eq("Iteration", "/iteration/unknown")) {
		t.Fatal("unknown legacy selection broadened")
	}
	v.Timebox = "/release/unknown"
	v.ReleaseTimebox, v.ReleaseName = "", ""
	v.migrateTimeboxes()
	if v.Timebox != "" || v.ReleaseTimebox != "/release/unknown" || !strings.Contains(a.rallyQuery(v).Expression, rally.Eq("Release", "/release/unknown")) {
		t.Fatal("legacy release became iteration")
	}
	v.Timebox, v.TimeboxName, v.ReleaseTimebox, v.ReleaseName = "/release/old", "Old", "/release/new", "New"
	v.migrateTimeboxes()
	if v.Timebox != "" || v.ReleaseName != "New" || strings.Contains(a.rallyQuery(v).Expression, "Iteration") {
		t.Fatal("explicit release did not supersede legacy slot")
	}
	if timeboxDefaultRef(a.iterations, "/project/unknown", "/iteration/sibling", `Sprint "1"`) != "" {
		t.Fatal("foreign project default assigned")
	}
}

func TestV69TimeboxPersistence(t *testing.T) {
	a, v := keyboardBoard(t, 3)
	v.Timebox, v.TimeboxName, v.ReleaseTimebox, v.ReleaseName = "/iteration/1", "Sprint 1", "/release/2", "Q4"
	saved := v.savedView("Both")
	copy := newRallyView(v.Spec)
	copy.applySavedView(saved)
	if !savedViewEqual(saved, copy.savedView("Both")) {
		t.Fatal("saved view lost independent choices")
	}
	tab := *a.state.Current()
	first := a.checkpointDocument(tab)
	v.ReleaseName = "Q5"
	second := a.checkpointDocument(tab)
	if first.Rally == second.Rally || first.Rally.ReleaseName != "Q4" {
		t.Fatal("release change did not invalidate immutable checkpoint")
	}
	b := transferFixture()
	if err := b.installTransfer(transferJSON(t, second)); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[tab.ID]
	if restored.TimeboxName != "Sprint 1" || restored.ReleaseName != "Q5" || restored.ReleaseTimebox != "/release/2" {
		t.Fatal("transfer lost selections")
	}
	if !restored.removeFilter("release") || restored.ReleaseTimebox != "" || restored.ReleaseName != "" || restored.Timebox == "" {
		t.Fatal("release removal changed iteration")
	}
	restored.clearRallyFilters()
	if restored.TimeboxName != "" || restored.ReleaseName != "" || restored.Timebox != "" {
		t.Fatal("clear retained hidden predicate")
	}
	copy.applySavedView(settings.SavedView{})
	if copy.TimeboxName != "" || copy.ReleaseName != "" || copy.ReleaseTimebox != "" {
		t.Fatal("Standard View retained timeboxes")
	}
	old := newRallyView(copy.Spec).savedView("Old")
	old.Timebox = "/release/2"
	copy.applySavedView(old)
	copy.ReleaseName = "Q4"
	if !savedViewEqual(old, copy.savedView("Old")) {
		t.Fatal("legacy hydration marked view dirty")
	}
	changed := copy.savedView("Old")
	changed.ReleaseName = "Other"
	if savedViewEqual(changed, copy.savedView("Old")) {
		t.Fatal("explicit changed name ignored")
	}
	current := newRallyView(rally.FindPage("iterationstatus"))
	current.Timebox = "/iteration/now"
	current.TimeboxName = "Now"
	current.ReleaseTimebox = "/release/2"
	current.ReleaseName = "Q4"
	cs := current.savedView("Current")
	if cs.Timebox != "" || cs.TimeboxName != "" || cs.ReleaseName != "Q4" || !cs.CurrentIteration {
		t.Fatal("relative iteration pinned or release lost", cs)
	}
}

func TestV69TimeboxRefreshOwnership(t *testing.T) {
	a := presetApp(t)
	a.iterations = []rally.Object{{"_ref": "/iteration/1", "Name": "Original"}}
	old := a.getTimeboxChoices("Iteration")
	replacement := makeTimeboxChoices([]rally.Object{{"_ref": "/iteration/2", "Name": "Replacement"}}, "", time.Now())
	a.timeboxChoices["Iteration"] = replacement
	drain(t, a, func() bool { return !old.updating })
	if a.timeboxChoices["Iteration"] != replacement {
		t.Fatal("stale metadata preparation published")
	}
	replacement.nextChange = time.Now().Add(-time.Second)
	a.getTimeboxChoices("Iteration")
	drain(t, a, func() bool { return a.timeboxChoices["Iteration"] != replacement })
	if a.timeboxChoices["Iteration"].updating {
		t.Fatal("clock refresh stuck")
	}
	empty := makeTimeboxChoices(nil, "", time.Now())
	a.timeboxChoices["Release"] = empty
	if a.getTimeboxChoices("Release") != empty || empty.updating {
		t.Fatal("empty choices repeatedly rebuilt")
	}
}

func TestV69TimeboxControls(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := presetApp(t)
			a.p = colors(false)
			now := time.Now()
			a.timeboxChoices = map[string]*timeboxChoices{"Iteration": makeTimeboxChoices([]rally.Object{timeboxRow("iteration", "1", "Sprint 1", "", now.Add(-time.Hour), now.Add(time.Hour))}, "", now), "Release": makeTimeboxChoices([]rally.Object{timeboxRow("release", "2", "Q4", "", now.Add(-time.Hour), now.Add(time.Hour))}, "", now)}
			v := newRallyView(rally.FindPage("teamboard"))
			v.TimeboxName = "Sprint 1"
			v.ReleaseName = "Q4"
			h := desktop.NewHeadlessHarness(0, image.Pt(int(760*scale), int(250*scale)), func(w *desktop.Window) { a.drawTimeboxSelectors(w, v) })
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			var labels []string
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd {
					labels = append(labels, c.Text.String)
				}
			}
			if !slices.Contains(labels, "Iteration") || !slices.Contains(labels, "FY Quarter / Release") || !strings.Contains(strings.Join(labels, "|"), "Sprint 1") || !strings.Contains(strings.Join(labels, "|"), "Q4") {
				t.Fatal("missing independent controls", labels)
			}
		})
	}
	for _, page := range []string{"teamboard", "iterationstatus", "teamplan", "capacityplanning", "teamstatus"} {
		if !rallyTimeboxesSupported(newRallyView(rally.FindPage(page))) {
			t.Fatal(page)
		}
	}
	for _, page := range []string{"users", "projects", "timeboxes", "backlog", "customviews", "portfolioitemstreegrid", "portfoliokanban"} {
		if rallyTimeboxesSupported(newRallyView(rally.FindPage(page))) {
			t.Fatal("unsupported timebox predicate", page)
		}
	}
}

func TestV69TimeboxControlSelection(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := presetApp(t)
			now := time.Now()
			a.timeboxChoices = map[string]*timeboxChoices{
				"Iteration": makeTimeboxChoices([]rally.Object{timeboxRow("iteration", "1", "Sprint 1", "", now.Add(-time.Hour), now.Add(time.Hour))}, "", now),
				"Release":   makeTimeboxChoices([]rally.Object{timeboxRow("release", "2", "Q4", "", now.Add(-time.Hour), now.Add(time.Hour))}, "", now),
			}
			v := newRallyView(rally.FindPage("iterationstatus"))
			var click image.Point
			var clicking bool
			h := desktop.NewHeadlessHarness(0, image.Pt(int(760*scale), int(500*scale)), func(w *desktop.Window) {
				m := &w.Master().Input().Mouse
				m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
				m.Buttons[mouse.ButtonLeft].Clicked = clicking
				a.drawTimeboxSelectors(w, v)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			press := func(prefix string) {
				t.Helper()
				clicking = false
				h.Frame(false)
				found := false
				for _, cmd := range h.Commands() {
					if cmd.Kind == command.TextCmd && strings.HasPrefix(cmd.Text.String, prefix) {
						click = image.Pt(cmd.Rect.X+cmd.Rect.W/2, cmd.Rect.Y+cmd.Rect.H/2)
						found = true
						break
					}
				}
				if !found {
					t.Fatal("choice not visible", prefix)
				}
				clicking = true
				h.Frame(false)
				clicking = false
				h.Frame(false)
			}
			press("Current iteration")
			press("Sprint 1 ·")
			if v.CurrentIteration || v.TimeboxName != "Sprint 1" || v.Timebox != "/iteration/1" {
				t.Fatal("iteration choice not applied")
			}
			press("All")
			press("Q4 ·")
			if v.ReleaseTimebox != "/release/2" || v.ReleaseName != "Q4" || v.TimeboxName != "Sprint 1" {
				t.Fatal("independent release choice not applied")
			}
		})
	}
}
