//go:build nucular_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
)

func TestV64FilterQueries(t *testing.T) {
	a := presetApp(t)
	a.rallyClient, _ = rally.New("https://rally.example", "test", nil)
	v := newRallyView(rally.FindPage("userstories"))
	for _, field := range rallyFilterFields[:4] {
		for _, operator := range rallyFilterOperators {
			t.Run(field.key+"/"+operator, func(t *testing.T) {
				value := "/slm/webservice/v2.0/" + strings.ToLower(field.kind) + "/42"
				wireOp, wireField := "=", field.key
				if operator == "is not" {
					wireOp = "!="
				}
				if field.key == "Tags" {
					wireOp = "contains"
					if operator == "is not" {
						wireOp = "!contains"
					}
				}
				if operator == "contains" {
					value = "Text \" OR (Blocked = true) \\ & Ω\nline"
					wireOp, wireField = "contains", field.key+".Name"
				}
				f := settings.RallyFilter{Field: field.key, Operator: operator, Value: value, Label: "Same display name"}
				got, err := a.rallyFilterClause(v, f)
				want := "(" + wireField + " " + wireOp + " " + rally.Quote(value) + ")"
				if err != nil || got != want {
					t.Fatalf("got %q, %v; want %q", got, err, want)
				}
			})
		}
	}
	v.QueryApplied = `(Blocked = true)`
	setText(v.Query, `(Name contains "not applied")`)
	v.StructuredFilters = []settings.RallyFilter{{Field: "Iteration", Operator: "contains", Value: "Sprint"}, {Field: "Release", Operator: "contains", Value: "FY26"}}
	q := a.rallyQuery(v)
	if !strings.Contains(q.Expression, `Iteration.Name contains "Sprint"`) || !strings.Contains(q.Expression, `Release.Name contains "FY26"`) || !strings.Contains(q.Expression, v.QueryApplied) || strings.Contains(q.Expression, "not applied") {
		t.Fatal("filters did not combine with the applied query", q)
	}
	before := a.rallySignature(v)
	setText(v.Query, "new draft")
	v.resetFilterDraft("Tags", "contains")
	setText(v.FilterDraft.Value, "unapplied tag")
	if a.rallySignature(v) != before {
		t.Fatal("draft typing changed the result-set identity")
	}
	v.StructuredFilters[0].Value = "Different"
	if a.rallySignature(v) == before {
		t.Fatal("applied filter reused old paging identity")
	}
}

func TestV64TypeFilterUsesPageKind(t *testing.T) {
	a := presetApp(t)
	v := newRallyView(rally.FindPage("userstories"))
	for _, tc := range []struct {
		op, value string
		match     bool
	}{
		{"is", "HierarchicalRequirement", true}, {"is not", "HierarchicalRequirement", false},
		{"is", "Defect", false}, {"is not", "Defect", true},
		{"contains", "user story", true}, {"contains", "hierarchical", true}, {"contains", "Defect", false},
	} {
		t.Run(tc.op+"/"+tc.value, func(t *testing.T) {
			got, err := a.rallyFilterClause(v, settings.RallyFilter{Field: "Type", Operator: tc.op, Value: tc.value})
			if err != nil || (got == "") != tc.match || !tc.match && got != "(ObjectID = 0)" {
				t.Fatal(got, err)
			}
		})
	}
}

func TestV64InvalidFiltersNeverBroadenScope(t *testing.T) {
	a := presetApp(t)
	a.rallyClient, _ = rally.New("https://rally.example", "test", nil)
	v := newRallyView(rally.FindPage("userstories"))
	for _, f := range []settings.RallyFilter{
		{Field: "Injected.Name", Operator: "contains", Value: "bad"},
		{Field: "Iteration", Operator: "OR", Value: "bad"},
		{Field: "Iteration", Operator: "contains", Value: " "},
		{Field: "Type", Operator: "is", Value: "unknown type"},
		{Field: "Iteration", Operator: "is", Value: "/slm/webservice/v2.0/project/42"},
		{Field: "Iteration", Operator: "is", Value: "https://foreign.example/slm/webservice/v2.0/iteration/42"},
	} {
		v.StructuredFilters = []settings.RallyFilter{f}
		if _, err := a.structuredFilterExpression(v); err == nil || !strings.Contains(a.rallyQuery(v).Expression, "ObjectID = 0") {
			t.Fatal("invalid filter broadened scope", f)
		}
	}
	v.StructuredFilters = make([]settings.RallyFilter, maxRallyFilters+1)
	if _, err := a.structuredFilterExpression(v); err == nil {
		t.Fatal("oversized restored filter set accepted")
	}
}

func TestV64FilterApplyQueriesAllMatches(t *testing.T) {
	queries := make(chan rally.Query, 4)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == rally.WSAPI+"hierarchicalrequirement" {
			values := r.URL.Query()
			queries <- rally.Query{Expression: values.Get("query"), Order: values.Get("order"), Workspace: values.Get("workspace"), Project: values.Get("project"), Start: 1}
			if values.Get("start") != "1" {
				t.Errorf("applied filter retained an old cursor: %s", values.Get("start"))
			}
			// Deliberately omit Tags/Release: the server already matched them.
			fmt.Fprintf(w, `{"QueryResult":{"Results":[{"ObjectID":9,"_ref":"http://%s/slm/webservice/v2.0/hierarchicalrequirement/9","Name":"matched"}],"TotalResultCount":1,"StartIndex":1}}`, r.Host)
			return
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	a.prefs.RallyWorkspace, a.prefs.RallyProject = "/workspace/1", "/project/2"
	v := newRallyView(rally.FindPage("userstories"))
	v.Page, v.Start, v.Total = 8, 176, 300
	v.resetFilterDraft("Tags", "contains")
	setText(v.FilterDraft.Value, `Ω & "special"`)
	if !a.addRallyFilter(v) {
		t.Fatal(v.FilterDraft.Error)
	}
	drain(t, a, func() bool { return !v.Loading })
	q := <-queries
	if q.Expression != `(Tags.Name contains "Ω & \"special\"")` || q.Workspace != "/workspace/1" || q.Project != "/project/2" || v.Page != 1 || v.Total != 1 || len(v.filtered()) != 1 {
		t.Fatal(q, v.Page, v.Total, v.Error)
	}
	if got := a.activeRallyFilters(v); len(got) != 1 || !strings.Contains(got[0].Label, "Tags contains") || !v.removeFilter(got[0].Key) || len(v.StructuredFilters) != 0 {
		t.Fatal("filter chip did not remove its clause", got)
	}
	if v.removeFilter("structured:-1") || v.removeFilter("structured:999") || v.removeFilter("structured:bad") {
		t.Fatal("invalid chip index was accepted")
	}
	v.StructuredFilters = []settings.RallyFilter{{Field: "Project", Operator: "is", Value: "wrong"}}
	a.refreshRallyItems(v)
	if v.Loading || v.Error == "" || len(queries) != 0 {
		t.Fatal("invalid restored filter sent a query", v.Error)
	}
}

func TestV64ReferencePickerIdentityAndLifecycle(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		kind := strings.TrimPrefix(r.URL.Path, rally.WSAPI)
		fmt.Fprintf(w, `{"QueryResult":{"Results":[{"ObjectID":1,"_ref":"http://%s/slm/webservice/v2.0/%s/1","Name":"Same name"},{"ObjectID":2,"_ref":"http://%s/slm/webservice/v2.0/%s/2","Name":"Same name"}],"TotalResultCount":2,"StartIndex":1}}`, r.Host, kind, r.Host, kind)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	for _, field := range rallyFilterFields[:4] {
		t.Run(field.key, func(t *testing.T) {
			v := newRallyView(rally.FindPage("userstories"))
			v.Detail = makeDetail(rally.Object{"Name": "Keep this editor"}, "HierarchicalRequirement", false)
			original := v.Detail
			v.resetFilterDraft(field.key, "is not")
			p := a.chooseRallyFilter(v)
			drain(t, a, func() bool { return !p.loading })
			if len(p.items) != 2 || p.kinds[0] != field.kind || !a.selectReference(p, p.items[1]) {
				t.Fatal("typed picker did not load or select", p.err)
			}
			if !strings.HasSuffix(text(v.FilterDraft.Value), "/2") || v.FilterDraft.Label != "Same name" || v.Detail != original {
				t.Fatal("selection used a name or replaced the open item")
			}
			p = a.chooseRallyFilter(v)
			drain(t, a, func() bool { return !p.loading })
			a.prefs.RallyProject = "new scope"
			if a.selectReference(p, p.items[0]) || !strings.HasSuffix(text(v.FilterDraft.Value), "/2") {
				t.Fatal("stale scope overwrote the choice")
			}
			a.prefs.RallyProject = ""
			v.resetFilterDraft("Tags", "contains")
			if !p.closed || p.items != nil {
				t.Fatal("replacement retained picker")
			}
			v.resetFilterDraft(field.key, "is")
			p = a.chooseRallyFilter(v)
			drain(t, a, func() bool { return !p.loading })
			id := a.state.Open(workspace.Rally, "Filters", "", "userstories")
			a.rallyViews[id] = v
			a.closeTabNow(id)
			if !p.closed || !v.Closed || a.selectReference(p, rally.Object{}) {
				t.Fatal("closed view retained live picker")
			}
		})
	}
}

func TestV64FilterPersistence(t *testing.T) {
	a := presetApp(t)
	id := a.state.Open(workspace.Rally, "Filters", "", "userstories")
	v := newRallyView(rally.FindPage("userstories"))
	a.rallyViews[id] = v
	v.StructuredFilters = []settings.RallyFilter{{Field: "Tags", Operator: "contains", Value: "kept"}}
	v.resetFilterDraft("Release", "contains")
	setText(v.FilterDraft.Value, "unsaved quarter")
	v.QueryApplied = `(Name contains "applied")`
	setText(v.Query, `(Name contains "draft")`)
	saved := v.savedView("Filters")
	data, _ := json.Marshal(saved)
	var read settings.SavedView
	if err := json.Unmarshal(data, &read); err != nil || !slices.Equal(read.Filters, v.StructuredFilters) || read.Query != v.QueryApplied {
		t.Fatal("saved view lost applied filters", err)
	}
	first, unchanged := a.sessionSnapshot(), a.sessionSnapshot()
	if first.Documents[0].Rally != unchanged.Documents[0].Rally {
		t.Fatal("unchanged filters rebuilt checkpoint")
	}
	v.QueryApplied = "" // Empty is distinct from a legacy missing field.
	setText(v.FilterDraft.Value, "new draft")
	latest := a.sessionSnapshot()
	if latest.Documents[0].Rally == first.Documents[0].Rally || first.Documents[0].Rally.FilterDraft.Value != "unsaved quarter" {
		t.Fatal("changed draft was missed or old checkpoint mutated")
	}
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, latest.Documents[0])); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[b.state.Active]
	if restored.QueryApplied != "" || text(restored.Query) != text(v.Query) || !restored.FilterDraft.matches(v.FilterDraft.snapshot()) || !slices.Equal(restored.StructuredFilters, v.StructuredFilters) || restored.Query.Flags&nucular.EditSigEnter == 0 {
		t.Fatal("transfer applied a draft or lost filters")
	}
	v.StructuredFilters[0].Value = "mutated"
	if saved.Filters[0].Value != "kept" || latest.Documents[0].Rally.StructuredFilters[0].Value != "kept" {
		t.Fatal("snapshots aliased active filters")
	}
	restored.applySavedView(read)
	if text(restored.Query) != read.Query || restored.QueryApplied != read.Query || text(restored.FilterDraft.Value) != "" || !slices.Equal(restored.StructuredFilters, read.Filters) {
		t.Fatal("revert retained unrelated draft")
	}
	restored.CurrentIteration = true
	restored.clearRallyFilters()
	if restored.CurrentIteration || len(b.activeRallyFilters(restored)) != 0 || text(restored.FilterDraft.Value) != "" {
		t.Fatal("clear-all restored a hidden filter")
	}
	legacy := latest.Documents[0]
	legacy.Rally.QueryApplied = nil
	if err := b.installTransfer(transferJSON(t, legacy)); err != nil || b.rallyViews[b.state.Active].QueryApplied != legacy.Rally.Query {
		t.Fatal("legacy query migration failed", err)
	}
}

func TestV64FilterControlsAtDisplayScales(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		for _, width := range []int{320, 900} {
			t.Run(fmt.Sprintf("%g/%d", scale, width), func(t *testing.T) {
				a := presetApp(t)
				v := newRallyView(rally.FindPage("userstories"))
				v.resetFilterDraft("Tags", "contains")
				setText(v.FilterDraft.Value, "specific tag")
				var click image.Point
				h := nucular.NewHeadlessHarness(0, image.Pt(int(float64(width)*scale), int(400*scale)), func(w *nucular.Window) {
					if click != (image.Point{}) {
						m := &w.Input().Mouse
						m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
						m.Buttons[mouse.ButtonLeft].Clicked = true
					}
					a.drawRallyFilterBuilder(w, v)
				})
				style := makeStyle(a.p, 13)
				style.Scale(scale)
				h.Master().SetStyle(style)
				h.Frame(false)
				clip := image.Rect(0, 0, int(float64(width)*scale), int(400*scale))
				for _, c := range h.Commands() {
					if c.Kind == command.ScissorCmd {
						clip = image.Rect(c.Rect.X, c.Rect.Y, c.Rect.X+c.Rect.W, c.Rect.Y+c.Rect.H)
						continue
					}
					if c.Kind != command.TextCmd {
						continue
					}
					visible := image.Rect(c.Rect.X, c.Rect.Y, c.Rect.X+c.Rect.W, c.Rect.Y+c.Rect.H).Intersect(clip)
					if !visible.Empty() && (visible.Min.X < 0 || visible.Max.X > int(float64(width)*scale)) {
						t.Fatal("filter control exceeds viewport", c.Text.String, c.Rect)
					}
					if c.Text.String == "Add filter" {
						click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
					}
				}
				if click == (image.Point{}) {
					t.Fatal("missing add control")
				}
				h.Frame(false)
				if len(v.StructuredFilters) != 1 || v.StructuredFilters[0].Value != "specific tag" || text(v.FilterDraft.Value) != "" {
					t.Fatal("click did not apply draft", v.StructuredFilters)
				}
			})
		}
	}
}

func TestV64EnterAppliesOnlyActiveFilterEditor(t *testing.T) {
	for _, structured := range []bool{false, true} {
		t.Run(fmt.Sprint(structured), func(t *testing.T) {
			a := presetApp(t)
			v := newRallyView(rally.FindPage("userstories"))
			v.resetFilterDraft("Iteration", "contains")
			setText(v.FilterDraft.Value, "Sprint")
			setText(v.Query, `(Blocked = true)`)
			activate := true
			h := nucular.NewHeadlessHarness(0, image.Pt(900, 400), func(w *nucular.Window) {
				if activate {
					e := v.Query
					if structured {
						e = v.FilterDraft.Value
					}
					w.Master().ActivateEditor(w, e)
					activate = false
				}
				a.drawRallyFilterBuilder(w, v)
				a.drawRallyAdvancedQuery(w, v)
			})
			h.Master().SetStyle(makeStyle(a.p, 13))
			h.Frame(false)
			if v.QueryApplied != "" || len(v.StructuredFilters) != 0 {
				t.Fatal("drawing applied unfinished input")
			}
			h.Key(key.CodeReturnEnter, 0)
			h.Frame(false)
			if structured {
				if len(v.StructuredFilters) != 1 || v.StructuredFilters[0].Value != "Sprint" || v.QueryApplied != "" {
					t.Fatal("Enter applied wrong filter editor")
				}
			} else if v.QueryApplied != `(Blocked = true)` || len(v.StructuredFilters) != 0 {
				t.Fatal("Enter did not apply only the advanced query", v.QueryApplied)
			}
		})
	}
}
