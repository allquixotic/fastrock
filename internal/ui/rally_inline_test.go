//go:build fltk_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"math"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func inlineFixture(t *testing.T, endpoint string) (*App, *rallyView, rally.Object) {
	t.Helper()
	a := presetApp(t)
	a.prefs.RallyEndpoint = endpoint
	a.rallyClient, _ = rally.New(endpoint, "test", nil)
	v := newRallyView(rally.FindPage("userstories"))
	v.Mode = "list"
	v.Fields = []rally.Field{{Name: "Name", Required: true}, {Name: "PlanEstimate", AttributeType: "QUANTITY"}, {Name: "Blocked", AttributeType: "BOOLEAN"}, {Name: "c_Required", Required: true}}
	o := rally.Object{"_ref": endpoint + rally.WSAPI + "hierarchicalrequirement/1", "ObjectID": float64(1), "FormattedID": "US1", "Name": "Story", "PlanEstimate": float64(13.5), "Blocked": false, "VersionId": "1", "DragAndDropRank": "ABCD"}
	v.Items = []rally.Object{o}
	v.Total = 1
	return a, v, o
}

func TestV63InlinePayloadAndDraft(t *testing.T) {
	a, v, o := inlineFixture(t, "https://rally.test")
	a.startInline(v, o, "PlanEstimate", nil)
	d := v.Detail
	if d == nil || d.dirty() {
		t.Fatal("inline editor was not opened cleanly")
	}
	for _, bad := range []string{"-1", "NaN", "+Inf", "text"} {
		setText(d.Editors["PlanEstimate"], bad)
		if _, err := a.inlineChanges(d); err == nil {
			t.Fatal("invalid estimate accepted", bad)
		}
	}
	setText(d.Editors["PlanEstimate"], "3.5")
	fields, err := a.inlineChanges(d)
	if err != nil || !reflect.DeepEqual(fields, rally.Object{"PlanEstimate": 3.5}) {
		t.Fatal("inline edit included unrelated required fields", fields, err)
	}
	v.Page = 2
	v.Items = nil
	if !d.dirty() || text(d.Editors["PlanEstimate"]) != "3.5" {
		t.Fatal("paging lost inline draft")
	}
	setText(d.Editors["PlanEstimate"], "")
	fields, err = a.inlineChanges(d)
	if err != nil || len(fields) != 1 || fields["PlanEstimate"] != nil {
		t.Fatal("empty estimate did not clear", fields, err)
	}
	a.prefs.RallyProject = "changed"
	a.saveDetail(v)
	if d.Saving || d.Error == "" {
		t.Fatal("changed scope allowed inline save")
	}
}

func TestV63InlineConflictAndRetry(t *testing.T) {
	var base string
	var posts atomic.Int32
	body := make(chan rally.Object, 1)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/1") {
			o := rally.Object{"_ref": base + r.URL.Path, "ObjectID": float64(1), "Name": "Story", "FormattedID": "US1", "PlanEstimate": float64(4), "VersionId": "2"}
			if r.Method == http.MethodPost {
				var payload map[string]rally.Object
				json.NewDecoder(r.Body).Decode(&payload)
				fields := payload["HierarchicalRequirement"]
				body <- fields
				posts.Add(1)
				o["PlanEstimate"] = fields["PlanEstimate"]
				o["VersionId"] = "3"
				json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": o}})
			} else {
				json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": o})
			}
			return
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	base = s.URL
	a, v, o := inlineFixture(t, base)
	a.startInline(v, o, "PlanEstimate", nil)
	d := v.Detail
	setText(d.Editors["PlanEstimate"], "8")
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if posts.Load() != 0 || !d.Conflict || v.Detail != d {
		t.Fatal("inline conflict was not retained")
	}
	a.reloadDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if len(d.fieldConflicts) != 1 || text(d.Editors["PlanEstimate"]) != "8" {
		t.Fatal("reload lost inline draft or comparison", d.fieldConflicts)
	}
	d.resolveFieldConflict("PlanEstimate", false)
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if posts.Load() != 1 || v.Detail != nil || !reflect.DeepEqual(<-body, rally.Object{"PlanEstimate": float64(8)}) {
		t.Fatal("confirmed save did not finish", d.Error)
	}
}

func TestV63InlineTransfer(t *testing.T) {
	a, v, o := inlineFixture(t, "https://rally.test")
	id := a.state.Open(workspace.Rally, "Stories", "", "userstories")
	a.rallyViews[id] = v
	a.startInline(v, o, "PlanEstimate", nil)
	setText(v.Detail.Editors["PlanEstimate"], "17")
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, a.checkpointDocument(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	d := b.rallyViews[b.state.Active].Detail
	if d == nil || d.inlineField != "PlanEstimate" || d.inlineScope != v.Detail.inlineScope || !d.dirty() || text(d.Editors["PlanEstimate"]) != "17" || d.Editors["PlanEstimate"].Flags&desktop.EditSigEnter == 0 {
		t.Fatal("inline draft not transferred")
	}
}

func TestV63TableControlsAtDisplayScales(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a, v, _ := inlineFixture(t, "https://rally.test")
			v.Columns = []string{"Rank", "Name", "PlanEstimate", "Blocked"}
			var click image.Point
			h := desktop.NewHeadlessHarness(0, image.Pt(int(800*scale), int(550*scale)), func(w *desktop.Window) {
				if click != (image.Point{}) {
					m := &w.Input().Mouse
					m.Pos = click
					m.Buttons[mouse.ButtonLeft].Clicked = true
					m.Buttons[mouse.ButtonLeft].ClickedPos = click
				}
				a.drawInlineStatus(w, v)
				a.table(w, v, v.Items)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			var shown strings.Builder
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd {
					shown.WriteString(c.Text.String)
					shown.WriteByte('\n')
					if c.Text.String == "13.5" && click == (image.Point{}) {
						click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
					}
				}
			}
			for _, part := range []string{"ABCD", "Page totals · 1 items", "⋮"} {
				if !strings.Contains(shown.String(), part) {
					t.Fatal("table control missing", part, shown.String())
				}
			}
			if click == (image.Point{}) {
				t.Fatal("estimate cell not found")
			}
			h.Frame(false)
			click = image.Point{}
			if v.Detail == nil || v.Detail.inlineField != "PlanEstimate" {
				t.Fatal("estimate cell did not activate inline editor")
			}
			h.Frame(false)
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == "⋮" {
					click = image.Pt(c.Rect.X+2, c.Rect.Y+2)
				}
			}
			h.Frame(false)
			click = image.Point{}
			h.Frame(false)
			shown.Reset()
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd {
					shown.WriteString(c.Text.String)
					shown.WriteByte('\n')
				}
			}
			if !strings.Contains(shown.String(), "Copy ID") {
				t.Fatal("row menu did not open", shown.String())
			}
		})
	}
}

func TestV63PageTotals(t *testing.T) {
	columns := []string{"PlanEstimate", "Estimate", "ToDo", "Actuals", "Blocked", "ObjectID", "Rank"}
	objects := []rally.Object{{"PlanEstimate": float64(2), "Estimate": float64(1), "Blocked": true, "ObjectID": float64(42)}, {"PlanEstimate": float64(3), "ToDo": float64(4), "Actuals": float64(5)}, {"PlanEstimate": math.Inf(1)}}
	got := tableTotals(columns, objects[:2])
	want := map[string]string{"PlanEstimate": "5", "Estimate": "1", "ToDo": "4", "Actuals": "5", "Blocked": "1"}
	if !reflect.DeepEqual(got, want) {
		t.Fatal(got)
	}
	if tableTotals(columns, objects)["PlanEstimate"] != "5" {
		t.Fatal("non-finite values poisoned total")
	}
	v := newRallyView(rally.FindPage("userstories"))
	v.Items = objects
	v.Total = 3
	v.PageSize = 2
	v.Page = 2
	start, end, ok := v.tableRange(len(objects))
	if !ok || start != 2 || end != 3 || tableTotals(columns, objects[start:end])["PlanEstimate"] != "0" {
		t.Fatal("total crossed page boundary")
	}
}

func TestV63BlockedAndLateEdit(t *testing.T) {
	for _, field := range []string{"Blocked", "PlanEstimate"} {
		t.Run(field, func(t *testing.T) {
			entered, release := make(chan struct{}), make(chan struct{})
			body := make(chan rally.Object, 1)
			var base string
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if !strings.HasSuffix(r.URL.Path, "/1") {
					fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
					return
				}
				o := rally.Object{"_ref": base + r.URL.Path, "ObjectID": float64(1), "Name": "Story", "PlanEstimate": float64(13.5), "Blocked": false, "VersionId": "1"}
				if r.Method == http.MethodPost {
					var payload map[string]rally.Object
					json.NewDecoder(r.Body).Decode(&payload)
					changes := payload["HierarchicalRequirement"]
					body <- changes
					close(entered)
					<-release
					for k, v := range changes {
						o[k] = v
					}
					o["VersionId"] = "2"
					json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": o}})
				} else {
					json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": o})
				}
			}))
			defer s.Close()
			base = s.URL
			a, v, o := inlineFixture(t, base)
			if field == "Blocked" {
				value := "true"
				a.startInline(v, o, field, &value)
			} else {
				a.startInline(v, o, field, nil)
				setText(v.Detail.Editors[field], "5")
				a.saveDetail(v)
			}
			d := v.Detail
			<-entered
			if field == "PlanEstimate" {
				setText(d.Editors[field], "7")
			}
			close(release)
			drain(t, a, func() bool { return !d.Saving })
			sent := <-body
			if len(sent) != 1 {
				t.Fatal("inline save included other fields", sent)
			}
			if field == "Blocked" {
				if sent[field] != true || v.Detail != nil || !v.Items[0].Bool("Blocked") {
					t.Fatal("checkbox write did not complete", sent)
				}
			} else {
				if sent[field] != float64(5) || v.Detail != d || text(d.Editors[field]) != "7" || d.Original.Number(field) != 5 || !d.dirty() {
					t.Fatal("save lost typing made after dispatch", sent, d.Original)
				}
			}
		})
	}
}

func TestV63InlinePendingWriteAndMissingRevision(t *testing.T) {
	a, v, o := inlineFixture(t, "https://rally.test")
	a.startInline(v, o, "PlanEstimate", nil)
	d := v.Detail
	setText(d.Editors["PlanEstimate"], "2")
	v.Mutating = true
	a.saveDetail(v)
	if d.Saving || d.Error == "" {
		t.Fatal("inline save raced a pending mutation")
	}
	v.Mutating = false
	delete(d.Original, "VersionId")
	a.saveDetail(v)
	if d.Saving || d.Error == "" {
		t.Fatal("unversioned edit reached server")
	}
	v.Detail = nil
	v.Fields[1].ReadOnly = true
	a.startInline(v, o, "PlanEstimate", nil)
	if v.Detail != nil {
		t.Fatal("read-only field entered inline mode")
	}
}

func TestV63FilterRefreshKeepsInlineDraft(t *testing.T) {
	var calls atomic.Int32
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/1") {
			t.Error("filter refresh reloaded the edited object")
		}
		calls.Add(1)
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	a, v, o := inlineFixture(t, s.URL)
	v.signature = a.rallySignature(v)
	v.metadataSignature = v.signature
	a.startInline(v, o, "PlanEstimate", nil)
	d := v.Detail
	setText(d.Editors["PlanEstimate"], "17")
	setText(v.Search, "other rows")
	a.refreshRally(v)
	if d.Saving || !v.Loading {
		t.Fatal("query change did not refresh the table independently")
	}
	drain(t, a, func() bool { return !v.Loading })
	if calls.Load() == 0 || v.Detail != d || !d.dirty() || text(d.Editors["PlanEstimate"]) != "17" {
		t.Fatal("filter refresh discarded inline draft")
	}
}

func TestV63SwitchFieldsKeepsConfirmedRevision(t *testing.T) {
	var base string
	var revision atomic.Int32
	revision.Store(1)
	var writes atomic.Int32
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !strings.HasSuffix(r.URL.Path, "/1") {
			fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
			return
		}
		o := rally.Object{"_ref": base + r.URL.Path, "ObjectID": float64(1), "Name": "Story", "PlanEstimate": float64(13.5), "Blocked": false, "VersionId": fmt.Sprint(revision.Load())}
		if revision.Load() > 1 {
			o["PlanEstimate"] = float64(5)
		}
		if r.Method == http.MethodPost {
			writes.Add(1)
			o["VersionId"] = fmt.Sprint(revision.Add(1))
			var p map[string]rally.Object
			json.NewDecoder(r.Body).Decode(&p)
			for k, v := range p["HierarchicalRequirement"] {
				o[k] = v
			}
			json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": o}})
		} else {
			json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": o})
		}
	}))
	defer s.Close()
	base = s.URL
	a, v, o := inlineFixture(t, base)
	a.startInline(v, o, "PlanEstimate", nil)
	d := v.Detail
	setText(d.Editors["PlanEstimate"], "5")
	d.afterSave = func() { value := "true"; a.startInline(v, o, "Blocked", &value) }
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving && (v.Detail == nil || !v.Detail.Saving) })
	if writes.Load() != 2 || v.Detail != nil {
		t.Fatal("switching fields reused the pre-save revision", writes.Load())
	}
}
