//go:build nucular_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func selectionFixture(t *testing.T, endpoint string) (*App, *rallyView, *detailView) {
	t.Helper()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(endpoint, "test", nil)
	a.prefs.RallyEndpoint = endpoint
	v := newRallyView(rally.FindPage("teamboard"))
	v.Fields = []rally.Field{{Name: "ScheduleState", DisplayName: "Schedule State", AttributeType: "STATE", AllowedValues: []string{"Defined", "Accepted"}, Required: true}, {Name: "Owner", AttributeType: "OBJECT", ReferenceType: "User"}, {Name: "Iteration", AttributeType: "OBJECT", ReferenceType: "Iteration"}}
	for i := 1; i <= 3; i++ {
		v.selectItem(rally.Object{"_ref": fmt.Sprintf("%s%shierarchicalrequirement/%d", endpoint, rally.WSAPI, i), "FormattedID": fmt.Sprintf("US%d", i), "Name": fmt.Sprintf("Work %d", i), "ScheduleState": "Defined", "LastUpdateDate": "one"}, true)
	}
	a.openSelectionEditor(v)
	return a, v, v.Detail
}

func TestV62SelectionPreviewValidation(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Accepted")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	if len(d.selection.Rows) != 3 || len(d.selection.Rows[0].Fields) != 1 || d.selection.Rows[0].Fields["ScheduleState"] != "Accepted" {
		t.Fatal("wrong reviewed payload", d.selection.Rows)
	}
	v.SelectedItems[d.selection.Targets[0].String("_ref")]["Name"] = "Changed outside editor"
	if d.selection.Targets[0].String("Name") != "Work 1" {
		t.Fatal("selection not isolated")
	}
	setText(d.Editors["ScheduleState"], "Invented")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("invented state accepted")
	}
	setText(d.Editors["ScheduleState"], "Accepted")
	d.selection.Enabled["Owner"] = true
	setText(d.Editors["Owner"], "https://foreign.test"+rally.WSAPI+"user/1")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("foreign owner accepted")
	}
	setText(d.Editors["Owner"], "https://rally.test"+rally.WSAPI+"iteration/1")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("wrong owner type accepted")
	}
	setText(d.Editors["Owner"], "")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	if _, ok := d.selection.Rows[0].Fields["Owner"]; ok {
		t.Fatal("unchanged null owner sent")
	}
	d.Fields[0].ReadOnly = true
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("read-only field accepted")
	}
}

func TestV62SelectionStopsAtConflict(t *testing.T) {
	var posts atomic.Int32
	var base string
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := strings.TrimPrefix(r.URL.Path, rally.WSAPI+"hierarchicalrequirement/")
		if id == "1" || id == "2" || id == "3" {
			stamp := "one"
			if id == "2" {
				stamp = "two"
			}
			obj := rally.Object{"_ref": base + r.URL.Path, "FormattedID": "US" + id, "Name": "Work " + id, "ObjectID": id, "ScheduleState": "Defined", "LastUpdateDate": stamp}
			if r.Method == http.MethodPost {
				posts.Add(1)
				obj["ScheduleState"] = "Accepted"
				obj["LastUpdateDate"] = "saved"
				json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": obj}})
			} else {
				json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": obj})
			}
			return
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	base = s.URL
	a, v, d := selectionFixture(t, base)
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Accepted")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	a.applySelection(v)
	drain(t, a, func() bool { return !d.Saving })
	if posts.Load() != 1 || d.selection.Rows[0].Status != "Updated" || d.selection.Rows[1].Status != "Verify before retry" || d.selection.Rows[2].Status != "Not attempted" {
		t.Fatal("batch outcomes lost", posts.Load(), d.selection.Rows, d.Error)
	}
	if len(v.Selected) != 2 || !d.dirty() {
		t.Fatal("completed/remaining selection wrong")
	}
	a.applySelection(v)
	if d.Saving || posts.Load() != 1 {
		t.Fatal("partial batch could replay")
	}
	a.reloadSelection(v)
	drain(t, a, func() bool { return !d.Loading })
	if len(d.selection.Targets) != 2 || len(d.selection.Rows) != 0 || text(d.Editors["ScheduleState"]) != "Accepted" {
		t.Fatal("reload lost draft or retained completed item", d.Error)
	}
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	a.applySelection(v)
	drain(t, a, func() bool { return !d.Saving })
	if posts.Load() != 3 || !d.selection.Complete || d.dirty() || len(v.Selected) != 0 {
		t.Fatal("remaining work was not completed", d.Error, posts.Load())
	}
}

func TestV62SelectionRejectsStalePreview(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Accepted")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	setText(d.Editors["ScheduleState"], "Defined")
	a.applySelection(v)
	if d.Saving || d.Error == "" {
		t.Fatal("changed draft bypassed preview")
	}
	setText(d.Editors["ScheduleState"], "Accepted")
	a.prefs.RallyProject = "/project/other"
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("changed scope accepted")
	}
}

func TestV62SelectionTransfer(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	id := a.state.Open(workspace.Rally, "Board", "", "teamboard")
	a.rallyViews[id] = v
	d.selection.Enabled["Owner"] = true
	setText(d.Editors["Owner"], "https://rally.test"+rally.WSAPI+"user/8")
	d.referenceLabels = map[string]string{"Owner\x00https://rally.test" + rally.WSAPI + "user/8": "Eight"}
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	first := a.checkpointDocument(*a.state.Current())
	d.selection.Rows[0].Status = "Verify before retry"
	d.selection.Attempted = true
	d.snapshotRevision++
	x := a.checkpointDocument(*a.state.Current())
	if first.Rally.Detail.Selection.Rows[0].Status != "Ready" {
		t.Fatal("checkpoint shared outcome state")
	}
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, x)); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[b.state.Active].Detail
	if restored.selection == nil || !restored.selection.Attempted || !restored.selection.Enabled["Owner"] || !restored.dirty() || restored.selection.Rows[0].Status != "Verify before retry" {
		t.Fatal("selection draft/outcome not restored")
	}
}

func TestV62SelectionControlsAtDisplayScales(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a, v, d := selectionFixture(t, "https://rally.test")
			var click image.Point
			h := nucular.NewHeadlessHarness(0, image.Pt(int(850*scale), int(760*scale)), func(w *nucular.Window) {
				if click != (image.Point{}) {
					m := &w.Input().Mouse
					m.Pos = click
					m.Buttons[mouse.ButtonLeft].Clicked = true
					m.Buttons[mouse.ButtonLeft].ClickedPos = click
				}
				a.drawDetail(w, v)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			var all strings.Builder
			for _, cmd := range h.Commands() {
				if cmd.Kind == command.TextCmd {
					all.WriteString(cmd.Text.String)
					all.WriteByte('\n')
				}
			}
			for _, s := range []string{"Edit selected", "Change Schedule State", "Change Owner", "Change Iteration", "Review changes", "US1"} {
				if !strings.Contains(all.String(), s) {
					t.Fatalf("missing %q at %g: %s", s, scale, all.String())
				}
			}
			if d.Saving {
				t.Fatal("drawing wrote changes")
			}
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == "Change Schedule State" {
					click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
				}
			}
			if click == (image.Point{}) {
				t.Fatal("missing state checkbox")
			}
			h.Frame(false)
			click = image.Point{}
			if !d.selection.Enabled["ScheduleState"] {
				t.Fatal("state checkbox did not enable the field")
			}
			setText(d.Editors["ScheduleState"], "Accepted")
			h.Frame(false)
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == "Review changes" {
					click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
				}
			}
			if click == (image.Point{}) {
				t.Fatal("missing review button")
			}
			h.Frame(false)
			click = image.Point{}
			if len(d.selection.Rows) != 3 || d.Saving {
				t.Fatal("review action did not prepare exactly the selection", d.Error)
			}
		})
	}
}

func TestV62SelectionConnectionAndClose(t *testing.T) {
	for _, outcome := range []string{"connection", "scope", "closed"} {
		t.Run(outcome, func(t *testing.T) {
			entered, release := make(chan struct{}), make(chan struct{})
			var writes atomic.Int32
			var base string
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if strings.HasSuffix(r.URL.Path, "/1") {
					o := rally.Object{"_ref": base + r.URL.Path, "ObjectID": float64(1), "FormattedID": "US1", "LastUpdateDate": "one"}
					if r.Method == http.MethodPost {
						writes.Add(1)
						close(entered)
						<-release
						json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": o}})
					} else {
						json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": o})
					}
					return
				}
				if r.Method == http.MethodPost {
					writes.Add(1)
				}
				fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
			}))
			defer s.Close()
			base = s.URL
			a, v, d := selectionFixture(t, base)
			d.selection.Enabled["ScheduleState"] = true
			setText(d.Editors["ScheduleState"], "Accepted")
			if err := a.prepareSelection(v); err != nil {
				t.Fatal(err)
			}
			a.applySelection(v)
			<-entered
			switch outcome {
			case "connection":
				a.rallyClient, _ = rally.New(base, "changed", nil)
			case "scope":
				a.prefs.RallyProject = "other"
			case "closed":
				v.Closed = true
				a.disposeDetail(d)
			}
			close(release)
			drain(t, a, func() bool { return !d.Saving })
			if writes.Load() != 1 {
				t.Fatal("later write escaped changed/closed editor")
			}
			if outcome != "closed" && (d.selection.Rows[1].Status != "Not attempted" || d.Error == "") {
				t.Fatal("remaining outcome missing", d.selection.Rows, d.Error)
			}
		})
	}
}

func TestV62SelectionReloadFailureKeepsDraft(t *testing.T) {
	var base string
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/1") {
			json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": rally.Object{"_ref": base + r.URL.Path, "ObjectID": float64(1), "Name": "Server renamed", "LastUpdateDate": "two"}})
			return
		}
		w.WriteHeader(503)
	}))
	defer s.Close()
	base = s.URL
	a, v, d := selectionFixture(t, base)
	d.selection.Enabled["Owner"] = true
	setText(d.Editors["Owner"], base+rally.WSAPI+"user/8")
	a.reloadSelection(v)
	drain(t, a, func() bool { return !d.Loading })
	if d.Error == "" || d.selection.Targets[0].String("Name") != "Work 1" || text(d.Editors["Owner"]) != base+rally.WSAPI+"user/8" {
		t.Fatal("partial reload replaced baseline or draft", d.Error)
	}
}

func TestV62SelectionNoopAndReferenceStates(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Defined")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	a.applySelection(v)
	if d.Saving || !d.selection.Complete || d.dirty() {
		t.Fatal("no-op batch did not finish without writing")
	}
	a, v, d = selectionFixture(t, "https://rally.test")
	d.Kind = "PortfolioItem/Feature"
	d.selection.Kinds = []string{d.Kind}
	d.selection.StateField = "State"
	for i, o := range d.selection.Targets {
		o["_ref"] = fmt.Sprintf("https://rally.test%sportfolioitem/feature/%d", rally.WSAPI, i+1)
	}
	mergeSchemaEditors(d, []rally.Field{{Name: "State", AttributeType: "OBJECT", Required: true}})
	d.States = []rally.Object{{"Name": "Delivered", "_ref": "https://rally.test" + rally.WSAPI + "state/8"}}
	d.selection.Enabled["State"] = true
	setText(d.Editors["State"], d.States[0].String("_ref"))
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	if d.selection.Rows[0].Fields["State"] != d.States[0].String("_ref") {
		t.Fatal("portfolio state label sent instead of ref")
	}
	if !strings.Contains(selectionRowText(d, d.selection.Rows[0]), "Delivered") {
		t.Fatal("preview omitted portfolio state display label")
	}
	setText(d.Editors["State"], "https://rally.test"+rally.WSAPI+"state/9")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("another artifact workflow's state accepted")
	}
}

func TestV62SelectionWriteGuardsNavigation(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	id := a.state.Open(workspace.Rally, "Board", "", "teamboard")
	a.rallyViews[id] = v
	d.Saving, v.Mutating = true, true
	a.closeTab(id)
	if len(a.state.Tabs) != 1 {
		t.Fatal("in-flight write tab closed")
	}
	a.closeTabs([]string{id})
	if len(a.state.Tabs) != 1 {
		t.Fatal("in-flight write closed by batch action")
	}
	if !a.pendingRallyWrite(id) {
		t.Fatal("write not protected from transfer")
	}
	called := false
	a.leaveDetail(v, func() { called = true })
	if called {
		t.Fatal("in-flight write editor abandoned")
	}
}

func TestV62SelectionPreviewIsBoundedAndFullText(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	template := d.selection.Targets[0]
	d.selection.Targets = nil
	for i := 1; i <= 5000; i++ {
		o := template.Clone()
		o["_ref"] = fmt.Sprintf("https://rally.test%shierarchicalrequirement/%d", rally.WSAPI, i)
		o["FormattedID"] = fmt.Sprintf("US%d", i)
		d.selection.Targets = append(d.selection.Targets, o)
	}
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Accepted")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		h := nucular.NewHeadlessHarness(0, image.Pt(int(850*scale), int(760*scale)), func(w *nucular.Window) { a.drawDetail(w, v) })
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		if len(h.Commands()) > 1000 {
			t.Fatal("preview rendered the entire selection", scale, len(h.Commands()))
		}
	}
	detail := selectionRowText(d, d.selection.Rows[0])
	for _, part := range []string{"US1", "Work 1", "Defined", "Accepted", "Schedule State"} {
		if !strings.Contains(detail, part) {
			t.Fatal("full comparison omitted", part, detail)
		}
	}
}

func TestV62SelectionExplicitReferenceChanges(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	for _, o := range d.selection.Targets {
		o["Iteration"] = map[string]any{"_ref": "https://rally.test" + rally.WSAPI + "iteration/3", "_refObjectName": "Prior iteration"}
	}
	d.selection.Enabled["Owner"], d.selection.Enabled["Iteration"] = true, true
	owner := "https://rally.test" + rally.WSAPI + "user/8"
	setText(d.Editors["Owner"], owner)
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	for _, row := range d.selection.Rows {
		clear, exists := row.Fields["Iteration"]
		if len(row.Fields) != 2 || row.Fields["Owner"] != owner || !exists || clear != nil {
			t.Fatal("unselected field changed or explicit clear lost", row.Fields)
		}
	}
	d.selection.Targets[0]["_ref"] = "https://rally.test" + rally.WSAPI + "defect/1"
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("mixed-type selection accepted")
	}
	d.selection.Targets[0]["_ref"] = "https://rally.test" + rally.WSAPI + "hierarchicalrequirement/1"
	delete(d.selection.Targets[0], "LastUpdateDate")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("unversioned selection accepted")
	}
}

func TestV62SelectionWriteQueueFailure(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodPost {
			t.Error("rejected write reached the server")
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	a, v, d := selectionFixture(t, s.URL)
	entered, release := make(chan struct{}, 2), make(chan struct{})
	defer close(release)
	for range 2 {
		a.writeWork(func() { entered <- struct{}{}; <-release })
	}
	for range 2 {
		<-entered
	}
	for range 16 {
		if !a.writeWork(func() {}) {
			t.Fatal("queue filled early")
		}
	}
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Accepted")
	if err := a.prepareSelection(v); err != nil {
		t.Fatal(err)
	}
	a.applySelection(v)
	drain(t, a, func() bool { return !d.Saving })
	if v.Mutating || d.Error == "" || len(v.Selected) != 3 {
		t.Fatal("admission failure lost selection or left editor busy")
	}
	for _, row := range d.selection.Rows {
		if row.Status != "Not attempted" {
			t.Fatal("admission failure has misleading outcome", row.Status)
		}
	}
}

func TestV62SelectionRejectsAliasedDuplicates(t *testing.T) {
	a, v, d := selectionFixture(t, "https://rally.test")
	d.selection.Targets[1]["_ref"] = "/slm/webservice/v2.0/HierarchicalRequirement/0001"
	d.selection.Enabled["ScheduleState"] = true
	setText(d.Editors["ScheduleState"], "Accepted")
	if err := a.prepareSelection(v); err == nil {
		t.Fatal("relative/case/zero-padded alias accepted as another artifact")
	}
}
