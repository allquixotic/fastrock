//go:build nucular_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func TestV58ReferenceLabelUsesHumanName(t *testing.T) {
	for _, tc := range []struct {
		object rally.Object
		want   string
	}{
		{rally.Object{"_ref": "https://rally.test/user/1", "DisplayName": "Current owner"}, "Current owner"},
		{rally.Object{"_ref": "https://rally.test/feature/2", "_refObjectName": "Existing feature"}, "Existing feature"},
		{rally.Object{"_ref": "https://rally.test/feature/3"}, ""},
		{rally.Object{"FormattedID": "F4", "Name": "Named feature"}, "F4 · Named feature"},
	} {
		if got := referenceLabel(tc.object); got != tc.want {
			t.Fatalf("referenceLabel(%v) = %q; want %q", tc.object, got, tc.want)
		}
	}
}

func TestV58ReferenceSchemaUsesIDsWithoutDirtying(t *testing.T) {
	for _, field := range []string{"Feature", "Parent", "WorkProduct", "Requirement", "PortfolioItem", "c_Reviewer"} {
		t.Run(field, func(t *testing.T) {
			ref := "/slm/webservice/v2.0/user/42"
			d := makeDetail(rally.Object{"Name": "Existing", field: map[string]any{"_ref": ref, "_refObjectName": "Displayed name"}}, "HierarchicalRequirement", false)
			mergeSchemaEditors(d, []rally.Field{{Name: field, AttributeType: "OBJECT", ReferenceType: "User"}})
			if text(d.Editors[field]) != ref || d.dirty() {
				t.Fatal("opening reference field changed its identity or dirtied the item", d.values())
			}
			setText(d.Editors[field], "")
			changes, err := d.changes()
			if value, ok := changes[field]; err != nil || !ok || value != nil {
				t.Fatal("cleared reference is not null", changes, err)
			}
			setText(d.Editors[field], "Plain display name")
			if _, err := d.changes(); err == nil {
				t.Fatal("display name was accepted as a reference")
			}
		})
	}
	d := makeDetail(rally.Object{"Name": "Defect", "State": "Open"}, "Defect", false)
	setText(d.Editors["State"], "Closed")
	if _, err := d.changes(); err != nil {
		t.Fatal("string workflow state was treated as a reference", err)
	}
}

func TestV58ReferencePickerTypesAndScope(t *testing.T) {
	d := makeDetail(rally.Object{"Name": "Story"}, "HierarchicalRequirement", false)
	for name, want := range map[string]string{"Owner": "User", "Feature": "PortfolioItem/Feature", "Parent": "HierarchicalRequirement", "Requirement": "HierarchicalRequirement", "PortfolioItem": "PortfolioItem/Feature"} {
		if got := referenceKinds(d, rally.Field{Name: name}); len(got) == 0 || got[0] != want {
			t.Fatal(name, got)
		}
	}
	p := &referencePicker{kinds: []string{"User"}, workspace: "/workspace/1", project: "/project/1", search: textEditor(`Alex "name"`, false)}
	q := referenceQuery(p, 51)
	if q.Project != "" || q.Workspace != "/workspace/1" || q.Start != 51 || q.PageSize != 50 || strings.Contains(q.Expression, "FormattedID") || !strings.Contains(q.Expression, "UserName contains") {
		t.Fatal(q)
	}
	p.kinds = []string{"HierarchicalRequirement"}
	q = referenceQuery(p, 1)
	if q.Project != "/project/1" || !strings.Contains(q.Expression, "FormattedID contains") || !strings.Contains(q.Expression, rally.Quote(`Alex "name"`)) {
		t.Fatal(q)
	}
}

func TestV58ReferencePickerRejectsStaleSearchAndPreservesTransfer(t *testing.T) {
	first, release, firstDone := make(chan struct{}), make(chan struct{}), make(chan struct{})
	requests := make(chan string, 8)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		query := r.URL.Query().Get("query")
		requests <- r.URL.Query().Get("start")
		name, id := "New result", 2
		if query == "" {
			defer close(firstDone)
			close(first)
			<-release
			name, id = "Old result", 1
		}
		base := "http://" + r.Host + rally.WSAPI
		rows := []rally.Object{
			{"ObjectID": id, "_ref": fmt.Sprintf("%shierarchicalrequirement/%d", base, id), "Name": name, "FormattedID": fmt.Sprintf("US%d", id)},
			{"ObjectID": 3, "_ref": "https://foreign.invalid" + rally.WSAPI + "hierarchicalrequirement/3", "Name": "Foreign"},
			{"ObjectID": 4, "_ref": base + "defect/4", "Name": "Wrong type"},
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": rows, "TotalResultCount": 120, "StartIndex": 1}})
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	id := a.state.Open(workspace.Rally, "Story", "", "userstories")
	v := newRallyView(rally.FindPage("userstories"))
	d := makeDetail(rally.Object{"Name": "Story", "_ref": s.URL + rally.WSAPI + "hierarchicalrequirement/99"}, "HierarchicalRequirement", false)
	v.Detail, a.rallyViews[id] = d, v
	p := a.openReferencePicker(d, rally.Field{Name: "Parent", AttributeType: "OBJECT"})
	select {
	case <-first:
	case <-time.After(time.Second):
		t.Fatal("initial search did not start")
	}
	setText(p.search, "new")
	a.searchReferences(p, 51) // Changing the query must reset the old cursor.
	drain(t, a, func() bool { return !p.loading })
	if len(p.items) != 1 || p.items[0].String("Name") != "New result" || p.start != 1 {
		t.Fatal("stale, foreign or wrong-type result published", p.items, p.start, p.err)
	}
	close(release)
	select {
	case <-firstDone:
	case <-time.After(time.Second):
		t.Fatal("superseded server handler did not finish")
	}
	for len(a.updates) > 0 {
		(<-a.updates)()
	}
	if len(p.items) != 1 || p.items[0].String("Name") != "New result" {
		t.Fatal("old search overwrote the newer result", p.items)
	}
	if !a.selectReference(p, p.items[0]) || !p.closed || !d.dirty() {
		t.Fatal("selection did not update the draft")
	}
	changes, err := d.changes()
	if err != nil || changes.String("Parent") != s.URL+rally.WSAPI+"hierarchicalrequirement/2" || d.Original.Ref("Parent") != "" {
		t.Fatal("selection did not preserve a reference and baseline", changes, err)
	}
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, a.tabSnapshot(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[b.state.Active].Detail
	if text(restored.Editors["Parent"]) != changes.String("Parent") || len(restored.referenceLabels) != 1 {
		t.Fatal("transfer lost reference value or display label")
	}
	for range 2 {
		if start := <-requests; start != "1" {
			t.Fatal("changed query used stale pagination", start)
		}
	}
}

func TestV58ReferencePickerProtectsNewerEditsAndClose(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, `{"QueryResult":{"Results":[{"ObjectID":1,"_ref":"http://%s/slm/webservice/v2.0/hierarchicalrequirement/1","Name":"Result"}],"TotalResultCount":1,"StartIndex":1}}`, r.Host)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	d := makeDetail(rally.Object{"Name": "Story"}, "HierarchicalRequirement", false)
	p := a.openReferencePicker(d, rally.Field{Name: "Parent", AttributeType: "OBJECT"})
	drain(t, a, func() bool { return !p.loading })
	setText(d.Editors["Parent"], s.URL+rally.WSAPI+"hierarchicalrequirement/7")
	if a.selectReference(p, p.items[0]) || p.err == "" || !strings.HasSuffix(text(d.Editors["Parent"]), "/7") {
		t.Fatal("picker overwrote an edit made after opening")
	}
	a.disposeDetail(d)
	if !p.closed || p.items != nil || d.referencePicker != nil {
		t.Fatal("detail close retained picker ownership")
	}
}

func TestV58ReferenceControlsShowLabelsAndOpenPicker(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, `{"QueryResult":{"Results":[{"ObjectID":1,"_ref":"http://%s/slm/webservice/v2.0/portfolioitem/feature/1","Name":"Selected feature","FormattedID":"F1"}],"TotalResultCount":1,"StartIndex":1}}`, r.Host)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	d := makeDetail(rally.Object{"Name": "Story", "Feature": map[string]any{"_ref": s.URL + rally.WSAPI + "portfolioitem/feature/2", "_refObjectName": "Existing feature"}}, "HierarchicalRequirement", false)
	f := rally.Field{Name: "Feature", DisplayName: "Parent feature", AttributeType: "OBJECT"}
	mergeSchemaEditors(d, []rally.Field{f})
	var click image.Point
	h := nucular.NewHeadlessHarness(0, image.Pt(600, 500), func(w *nucular.Window) {
		if click != (image.Point{}) {
			m := &w.Input().Mouse
			m.Pos = click
			m.Buttons[mouse.ButtonLeft].Clicked = true
			m.Buttons[mouse.ButtonLeft].ClickedPos = click
		}
		a.detailProperty(w, d, f)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	label := false
	for _, c := range h.Commands() {
		if c.Kind != command.TextCmd {
			continue
		}
		if strings.Contains(c.Text.String, "http://") {
			t.Fatal("raw reference rendered as editable text")
		}
		label = label || c.Text.String == "Existing feature"
		if c.Text.String == "Choose…" {
			click = image.Pt(c.Rect.X+5, c.Rect.Y+5)
		}
	}
	if !label || click == (image.Point{}) {
		t.Fatal("missing labeled picker")
	}
	h.Frame(false)
	if d.referencePicker == nil {
		t.Fatal("Choose control did not open its picker")
	}
	p := d.referencePicker
	drain(t, a, func() bool { return !p.loading })
	if len(p.items) != 1 || !a.selectReference(p, p.items[0]) {
		t.Fatal("feature choice failed", p.err)
	}
	if !strings.HasSuffix(text(d.Editors["Feature"]), "/feature/1") {
		t.Fatal("feature editor kept a display name")
	}
}
