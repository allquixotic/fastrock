//go:build fltk_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"slices"
	"strconv"
	"strings"
	"sync"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
)

func mixedMetadata() map[string]rallyTypeMetadata {
	out := map[string]rallyTypeMetadata{}
	for _, kind := range rally.FindPage("teamboard").ArtifactTypes() {
		out[kind] = rallyTypeMetadata{Fields: []rally.Field{
			{Name: "ScheduleState", AttributeType: "STATE", Required: true, AllowedValues: []string{"Defined", "Completed"}},
			{Name: "Owner", AttributeType: "OBJECT", ReferenceType: "User"},
			{Name: "Iteration", AttributeType: "OBJECT", ReferenceType: "Iteration"},
			{Name: "PlanEstimate", AttributeType: "DECIMAL"},
		}}
	}
	defect := out["Defect"]
	defect.Fields[0].AllowedValues = append(defect.Fields[0].AllowedValues, "Defect only")
	out["Defect"] = defect
	scheduleMetadata(out)
	return out
}

func TestV66TeamBoardTypeFilters(t *testing.T) {
	a := presetApp(t)
	v := newRallyView(rally.FindPage("teamboard"))
	if v.Spec.QueryKind() != "Artifact" || a.rallyQuery(v).ArtifactTypes != "HierarchicalRequirement,Defect,TestSet,DefectSuite" {
		t.Fatal("team board is not mixed")
	}
	for _, tc := range []struct{ op, value, want string }{
		{"is", "Defect", "Defect"}, {"contains", "test", "TestSet"}, {"is not", "Defect", "HierarchicalRequirement,TestSet,DefectSuite"},
	} {
		t.Run(tc.op+tc.value, func(t *testing.T) {
			v.StructuredFilters = []settings.RallyFilter{{Field: "Type", Operator: tc.op, Value: tc.value}}
			if got := a.rallyQuery(v).ArtifactTypes; got != tc.want {
				t.Fatal(got, tc.want)
			}
		})
	}
	v.StructuredFilters = []settings.RallyFilter{{Field: "Type", Operator: "is", Value: "Defect"}, {Field: "Type", Operator: "is", Value: "TestSet"}}
	if q := a.rallyQuery(v); !strings.Contains(q.Expression, "ObjectID = 0") {
		t.Fatal("contradictory filters broadened scope", q)
	}
	v = newRallyView(rally.FindPage("userstories"))
	if v.Spec.QueryKind() != "HierarchicalRequirement" || a.rallyQuery(v).ArtifactTypes != "" {
		t.Fatal("single-type page changed")
	}
}

func TestV66TeamBoardPagedResults(t *testing.T) {
	queries := make(chan string, 4)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != rally.WSAPI+"artifact" {
			fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
			return
		}
		queries <- r.URL.Query().Get("types")
		kinds := strings.Split(r.URL.Query().Get("types"), ",")
		start, _ := strconv.Atoi(r.URL.Query().Get("start"))
		count, _ := strconv.Atoi(r.URL.Query().Get("pagesize"))
		var results []rally.Object
		for i := range count {
			kind := kinds[i%len(kinds)]
			results = append(results, rally.Object{"_ref": fmt.Sprintf("%s%s/%d", rally.WSAPI, strings.ToLower(kind), start+i), "ObjectID": start + i, "Name": "Page item", "ScheduleState": "Defined"})
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": results, "TotalResultCount": 256, "StartIndex": start, "PageSize": count}})
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "fixture", nil)
	v := newRallyView(rally.FindPage("teamboard"))
	a.refreshRallyItems(v)
	drain(t, a, func() bool { return !v.Loading })
	if len(v.Items) != 128 || v.Next != 129 || !v.More || v.Error != "" {
		t.Fatal(v.Error, len(v.Items), v.Next)
	}
	seen := map[string]bool{}
	for _, o := range v.Items {
		seen[o.String("_type")] = true
	}
	if len(seen) != 4 {
		t.Fatal("missing card types", seen)
	}
	a.needRallyPage(v)
	drain(t, a, func() bool { return !v.Loading })
	if len(v.Items) != 256 || v.More || v.Error != "" {
		t.Fatal(v.Error, len(v.Items), v.More)
	}
	v.StructuredFilters = []settings.RallyFilter{{Field: "Type", Operator: "is", Value: "Defect"}}
	a.refreshRallyItems(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.Start != 1 || len(v.Items) != 256 || v.Error != "" {
		t.Fatal(v.Error, v.Start, len(v.Items))
	}
	for _, o := range v.Items {
		if o.String("_type") != "Defect" {
			t.Fatal("old type survived filter", o)
		}
	}
	for i := range 3 {
		got := <-queries
		want := "HierarchicalRequirement,Defect,TestSet,DefectSuite"
		if i == 2 {
			want = "Defect"
		}
		if got != want {
			t.Fatal(got, want)
		}
	}
}

func TestV66MetadataIntersectionAndCardTypes(t *testing.T) {
	metadata := mixedMetadata()
	kinds := rally.FindPage("teamboard").ArtifactTypes()
	common := commonRallyMetadata(metadataList(metadata, kinds))
	if slices.Contains(common.Fields[0].AllowedValues, "Defect only") || len(metadata["Defect"].Fields[0].AllowedValues) != 3 {
		t.Fatal("common choices unsafe or mutated source", common)
	}
	if len(common.Workflow) != 2 {
		t.Fatal("workflow did not intersect")
	}
	restricted := mixedMetadata()
	restricted["TestSet"].Fields[1].ReadOnly = true
	restricted["TestSet"].Fields[3].AttributeType = "INTEGER"
	restricted["TestSet"].Fields[3].ReadOnly = true
	intersection := commonRallyMetadata(metadataList(restricted, kinds))
	if !intersection.Fields[1].ReadOnly || len(intersection.Fields) != 3 || restricted["HierarchicalRequirement"].Fields[1].ReadOnly {
		t.Fatal("type restrictions were lost or mutated another schema", intersection)
	}
	typedView := newRallyView(rally.FindPage("teamboard"))
	typedView.TypeMetadata = restricted
	if _, ok := inlineField(typedView, rally.Object{"_type": "HierarchicalRequirement"}, "PlanEstimate"); !ok {
		t.Fatal("unshared field disabled for writable row type")
	}
	if _, ok := inlineField(typedView, rally.Object{"_type": "TestSet"}, "PlanEstimate"); ok {
		t.Fatal("read-only field enabled for row type")
	}
	a := presetApp(t)
	v := newRallyView(rally.FindPage("teamboard"))
	v.TypeMetadata = metadata
	v.Fields, v.Workflow = common.Fields, common.Workflow
	for i, kind := range kinds {
		v.Items = append(v.Items, rally.Object{"_ref": fmt.Sprintf("%s%s/%d", rally.WSAPI, strings.ToLower(kind), i+1), "_type": kind, "FormattedID": fmt.Sprintf("%s%d", artifactBadge(kind), i+1), "Name": "Example " + kind, "ScheduleState": "Defined", "State": "Open"})
	}
	v.prepareCards()
	v.prepareBoardLayout(v.Items)
	if len(v.boardGroups) != 1 || len(v.boardGroups[0].lanes[0].cards) != 4 {
		t.Fatal("mixed cards split by defect lifecycle state")
	}
	if _, ok := v.objectStateValue(v.Items[0], "Defect only"); ok {
		t.Fatal("story accepted defect-only state")
	}
	if _, ok := v.objectStateValue(v.Items[1], "Defect only"); !ok {
		t.Fatal("defect lost own workflow")
	}
	for _, scale := range []float64{1, 1.5, 2} {
		h := desktop.NewHeadlessHarness(0, image.Pt(int(1100*scale), int(1150*scale)), func(w *desktop.Window) { a.drawTeamBoard(w, v, v.Items) })
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		h.Frame(true)
		seen := map[string]bool{}
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd {
				seen[c.Text.String] = true
			}
		}
		for _, kind := range kinds {
			if !seen[artifactBadge(kind)] {
				t.Fatalf("missing %s badge at %g", kind, scale)
			}
		}
	}
}

func TestV66MixedBoardMovesAndSelection(t *testing.T) {
	var mu sync.Mutex
	items := map[string]rally.Object{}
	var envelopes []string
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		o := items[r.URL.Path]
		if o == nil {
			fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
			return
		}
		if r.Method == http.MethodPost {
			var body map[string]rally.Object
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
				t.Error(err)
				return
			}
			if len(body) != 1 {
				t.Error(body)
			}
			for kind, fields := range body {
				if kind != o.String("_type") {
					t.Error("wrong mutation envelope", kind, o)
				}
				envelopes = append(envelopes, kind)
				for k, value := range fields {
					o[k] = value
				}
			}
			o["VersionId"] = fmt.Sprint(len(envelopes) + 1)
			json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": o}})
		} else {
			json.NewEncoder(w).Encode(map[string]any{o.String("_type"): o})
		}
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "fixture", nil)
	a.prefs.RallyEndpoint = s.URL
	v := newRallyView(rally.FindPage("teamboard"))
	v.TypeMetadata = mixedMetadata()
	common := commonRallyMetadata(metadataList(v.TypeMetadata, v.Spec.ArtifactTypes()))
	v.Fields, v.Workflow = common.Fields, common.Workflow
	for i, kind := range v.Spec.ArtifactTypes() {
		path := fmt.Sprintf("%s%s/%d", rally.WSAPI, strings.ToLower(kind), i+1)
		o := rally.Object{"_ref": s.URL + path, "_type": kind, "ObjectID": float64(i + 1), "FormattedID": fmt.Sprint(i + 1), "Name": "Work " + kind, "ScheduleState": "Defined", "State": "Open", "VersionId": "1"}
		items[path] = o.Clone()
		v.Items = append(v.Items, o)
	}
	v.prepareCards()
	a.dropBoardCard(v, v.cards[v.Items[1].String("_ref")], boardDrop{State: "Completed"})
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	if v.Items[1].String("ScheduleState") != "Completed" || v.Items[1].String("State") != "Open" {
		t.Fatal("defect lifecycle state changed", v.Items[1])
	}
	for _, o := range v.Items {
		v.selectItem(o, true)
	}
	a.openSelectionEditor(v)
	d := v.Detail
	if d == nil || len(d.selection.Kinds) != 4 || len(d.selectionFields()) != 3 {
		t.Fatal("mixed selection schema unavailable", d)
	}
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Defect only")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("type-specific state accepted for mixed selection")
	}
	setText(d.Editors["ScheduleState"], "Completed")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	a.applySelection(v)
	drain(t, a, func() bool { return !d.Saving })
	if !d.selection.Complete || len(v.Selected) != 1 || !v.Selected[v.Items[1].String("_ref")] {
		t.Fatal(d.Error, d.selection.Rows)
	}
	mu.Lock()
	defer mu.Unlock()
	if !slices.Equal(envelopes, []string{"Defect", "DefectSuite", "HierarchicalRequirement", "TestSet"}) {
		t.Fatal("wrong concrete update envelopes", envelopes)
	}
	for _, o := range items {
		if o.String("ScheduleState") != "Completed" || o.String("State") != "Open" {
			t.Fatal(o)
		}
	}
	copy := d.selection.clone()
	copy.Kinds[0] = "bad"
	if d.selection.Kinds[0] == "bad" {
		t.Fatal("transfer shares mutable type scope")
	}
}
