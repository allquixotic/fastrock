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
)

func TestV59TaskTableColumnsAndWrappedNames(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		a := presetApp(t)
		d := makeDetail(rally.Object{}, "HierarchicalRequirement", false)
		d.Tab = "Tasks"
		d.Items = []rally.Object{{"FormattedID": "TA12", "Name": strings.Repeat("A long task name ", 10), "State": "In-Progress", "Estimate": 4, "ToDo": 2, "Actuals": 1, "Owner": map[string]any{"DisplayName": "Task owner"}}}
		v := newRallyView(rally.FindPage("teamboard"))
		v.Detail = d
		h := nucular.NewHeadlessHarness(0, image.Pt(int(1200*scale), int(700*scale)), func(w *nucular.Window) { a.detailCollection(w, v, d) })
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		labels := map[string]bool{}
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd {
				labels[c.Text.String] = true
			}
		}
		for _, label := range []string{"ID", "Name", "State", "Estimate", "To Do", "Actuals", "Owner", "TA12", "Task owner", "×"} {
			if !labels[label] {
				t.Fatal(scale, "missing task column/control", label)
			}
		}
		layout := d.taskLayout
		minimum := len(layout.Rows[0].Name.lines)*layout.Face.Metrics().Height.Ceil() + 2*int(6*scale)
		if strings.Join(strings.Fields(strings.Join(layout.Rows[0].Name.lines, " ")), " ") != strings.Join(strings.Fields(d.Items[0].String("Name")), " ") || len(layout.Rows[0].Name.lines) < 2 || layout.Rows[0].Height < minimum {
			t.Fatalf("task name was truncated or row did not grow: scale=%g width=%d height=%d lines=%q", scale, layout.Widths[1], layout.Rows[0].Height, layout.Rows[0].Name.lines)
		}
		h.Frame(false)
		if d.taskLayout != layout {
			t.Fatal("idle frame rebuilt task measurements")
		}
		a.disposeDetail(d)
		if d.taskLayout != nil {
			t.Fatal("closed task table retained its source")
		}
	}
}

func TestV59TaskFormCreatesFullPayload(t *testing.T) {
	var creates atomic.Int32
	body := make(chan rally.Object, 1)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case rally.WSAPI + "typedefinition":
			fmt.Fprintf(w, `{"QueryResult":{"Results":[{"ObjectID":1,"Attributes":{"_ref":"http://%s/slm/webservice/v2.0/typedefinition/1/Attributes"}}],"TotalResultCount":1,"StartIndex":1}}`, r.Host)
		case rally.WSAPI + "typedefinition/1/Attributes":
			fields := []rally.Object{{"ElementName": "Name", "AttributeType": "STRING", "Required": true}, {"ElementName": "State", "AttributeType": "RATING", "AllowedValues": map[string]any{"_ref": rally.WSAPI + "allowedattributevalue"}}, {"ElementName": "Owner", "AttributeType": "OBJECT", "Type": "User"}, {"ElementName": "WorkProduct", "AttributeType": "OBJECT", "Type": "Artifact"}}
			for _, name := range []string{"Estimate", "ToDo", "Actuals"} {
				fields = append(fields, rally.Object{"ElementName": name, "AttributeType": "QUANTITY"})
			}
			json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": fields, "TotalResultCount": len(fields), "StartIndex": 1}})
		case rally.WSAPI + "allowedattributevalue":
			fmt.Fprint(w, `{"QueryResult":{"Results":[{"StringValue":"Defined"},{"StringValue":"In-Progress"}],"TotalResultCount":2,"StartIndex":1}}`)
		case rally.WSAPI + "task/create":
			creates.Add(1)
			var envelope map[string]rally.Object
			json.NewDecoder(r.Body).Decode(&envelope)
			created := envelope["Task"].Clone()
			body <- created.Clone()
			created["ObjectID"], created["_ref"] = 12, "http://"+r.Host+rally.WSAPI+"task/12"
			json.NewEncoder(w).Encode(map[string]any{"CreateResult": map[string]any{"Object": created}})
		default:
			fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
		}
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	a.rallyUser = rally.Object{"_ref": s.URL + rally.WSAPI + "user/7", "DisplayName": "Current owner"}
	parent := makeDetail(rally.Object{"Name": "Parent", "_ref": s.URL + rally.WSAPI + "hierarchicalrequirement/5", "Project": map[string]any{"_ref": rally.WSAPI + "project/1"}}, "HierarchicalRequirement", false)
	parent.Tab = "Tasks"
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = parent
	a.newDetailTask(v, parent)
	d := v.Detail
	if d == parent || d.Kind != "Task" || !d.New || len(v.DetailHistory) != 1 {
		t.Fatal("Add task did not open the full task form with parent history")
	}
	drain(t, a, func() bool { return !d.SchemaLoading })
	if d.SchemaError != "" || text(d.Editors["State"]) != "Defined" || text(d.Editors["Owner"]) != a.rallyUser.String("_ref") {
		t.Fatal("task schema/defaults missing", d.SchemaError, d.values())
	}
	setText(d.Editors["Name"], "New task")
	setText(d.Editors["Estimate"], "4")
	setText(d.Editors["ToDo"], "2")
	setText(d.Editors["Actuals"], "1")
	a.saveDetail(v)
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if d.Error != "" || d.New || creates.Load() != 1 {
		t.Fatal("task creation failed or duplicated", d.Error, creates.Load())
	}
	got := <-body
	if got.String("WorkProduct") != parent.Original.String("_ref") || got.String("Owner") != a.rallyUser.String("_ref") || got.String("State") != "Defined" || got.Number("Estimate") != 4 || got.Number("ToDo") != 2 || got.Number("Actuals") != 1 {
		t.Fatal("task form lost fields in request", got)
	}
}

func TestV59TaskTableHorizontalScroll(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		a := presetApp(t)
		d := makeDetail(rally.Object{}, "HierarchicalRequirement", false)
		d.Tab = "Tasks"
		d.Items = []rally.Object{{"FormattedID": "TA12", "Name": "Task", "Owner": map[string]any{"DisplayName": "Task owner"}}}
		v := newRallyView(rally.FindPage("teamboard"))
		v.Detail = d
		scroll := 0
		h := nucular.NewHeadlessHarness(0, image.Pt(int(550*scale), int(500*scale)), func(w *nucular.Window) {
			w.Row(420).Dynamic(1)
			if body := w.GroupBegin("task-table", 0); body != nil {
				body.Scrollbar.X = scroll
				a.detailCollection(body, v, d)
				body.GroupEnd()
			}
		})
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		visible := func(label string) bool {
			for _, c := range h.Commands() {
				if c.Kind == command.TextCmd && c.Text.String == label && c.Rect.X >= 0 && c.Rect.X+c.Rect.W <= int(550*scale) {
					return true
				}
			}
			return false
		}
		if !visible("TA12") {
			t.Fatal("left task column unavailable", scale)
		}
		scroll = int(500 * scale)
		h.Frame(false)
		if !visible("×") || !visible("Task owner") {
			t.Fatal("horizontal scrolling did not reveal the owner/removal columns", scale)
		}
	}
}

func TestV59TaskDeleteChecksOwnershipRevisionAndPending(t *testing.T) {
	for _, outcome := range []string{"success", "changed", "moved", "failure"} {
		t.Run(outcome, func(t *testing.T) {
			var deletes atomic.Int32
			release := make(chan struct{})
			var parentRef string
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if strings.HasSuffix(r.URL.Path, "/task/12") {
					if r.Method == http.MethodDelete {
						deletes.Add(1)
						if outcome == "failure" {
							w.WriteHeader(http.StatusInternalServerError)
							fmt.Fprint(w, `{"OperationResult":{"Errors":["fixture failure"]}}`)
							return
						}
						fmt.Fprint(w, `{"OperationResult":{"Errors":[]}}`)
						return
					}
					<-release
					owner, stamp := parentRef, "one"
					if outcome == "moved" {
						owner += "0"
					}
					if outcome == "changed" {
						stamp = "two"
					}
					json.NewEncoder(w).Encode(map[string]any{"Task": rally.Object{"ObjectID": 12, "WorkProduct": map[string]any{"_ref": owner}, "LastUpdateDate": stamp}})
					return
				}
				fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
			}))
			defer s.Close()
			defer close(release)
			a := presetApp(t)
			a.rallyClient, _ = rally.New(s.URL, "test", nil)
			parentRef = s.URL + rally.WSAPI + "hierarchicalrequirement/5"
			d := makeDetail(rally.Object{"_ref": parentRef, "Tasks": map[string]any{"_ref": parentRef + "/Tasks"}}, "HierarchicalRequirement", false)
			d.Tab = "Tasks"
			item := rally.Object{"_ref": s.URL + rally.WSAPI + "task/12", "FormattedID": "TA12", "LastUpdateDate": "one"}
			d.Items, d.collectionTab = []rally.Object{item}, "Tasks"
			v := newRallyView(rally.FindPage("teamboard"))
			v.Detail = d
			a.deleteDetailTask(v, d, item)
			a.deleteDetailTask(v, d, item)
			if !d.Pending {
				t.Fatal("missing pending guard")
			}
			release <- struct{}{}
			drain(t, a, func() bool { return !d.Pending && !d.CollectionLoading })
			want := int32(0)
			if outcome == "success" || outcome == "failure" {
				want = 1
			}
			if deletes.Load() != want {
				t.Fatal("unsafe or duplicate task deletion", deletes.Load())
			}
			if outcome == "success" {
				if d.Error != "" || len(d.Items) != 0 || a.toast != "Deleted TA12" {
					t.Fatal("delete did not refresh collection", d.Error, d.Items, a.toast)
				}
			} else if d.Error == "" || len(d.Items) != 1 {
				t.Fatal("failed delete lost the row or error", d.Error, d.Items)
			}
		})
	}
}
