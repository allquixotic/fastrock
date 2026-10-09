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
)

func TestV60ReloadKeepsLocalEditsAndRequiresConflictChoices(t *testing.T) {
	d := makeDetail(rally.Object{"Name": "Original", "Description": "<p>Original</p>", "Priority": "Low", "Owner": map[string]any{"_ref": "/user/1"}, "LastUpdateDate": "one"}, "HierarchicalRequirement", false)
	setText(d.Editors["Name"], "My name")
	d.Rich["Description"] = newRichEditor("<p>My description</p>")
	d.CommentRich = newRichEditor("<p>Unsent comment</p>")
	latest := rally.Object{"Name": "Rally name", "Description": "<p>Rally description</p>", "Priority": "High", "Owner": map[string]any{"_ref": "/user/2"}, "LastUpdateDate": "two"}
	d.Conflict = true
	d.mergeReload(latest)
	if d.Conflict || text(d.Editors["Name"]) != "My name" || d.Rich["Description"].html() != "<p>My description</p>" || text(d.Editors["Priority"]) != "High" || text(d.Editors["Owner"]) != "/user/2" || d.Original.String("LastUpdateDate") != "two" || len(d.fieldConflicts) != 2 {
		t.Fatal("reload discarded edits, failed to advance the baseline, or missed conflicts", d.values(), d.fieldConflicts)
	}
	if d.CommentRich.html() != "<p>Unsent comment</p>" {
		t.Fatal("reload lost unsent comment")
	}
	d.mergeReload(latest)
	if len(d.fieldConflicts) != 2 {
		t.Fatal("repeated reload silently accepted unresolved choices")
	}
	d.resolveFieldConflict("Name", false)
	d.resolveFieldConflict("Description", true)
	if len(d.fieldConflicts) != 0 || text(d.Editors["Name"]) != "My name" || d.Rich["Description"].html() != "<p>Rally description</p>" || !d.dirty() {
		t.Fatal("conflict choice did not keep the chosen value")
	}
}

func TestV60ConflictReloadAndRetryUseLatestRevision(t *testing.T) {
	var updates atomic.Int32
	body := make(chan rally.Object, 1)
	var ref string
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != rally.WSAPI+"hierarchicalrequirement/1" {
			fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
			return
		}
		object := rally.Object{"_ref": ref, "ObjectID": 1, "Name": "Server name", "LastUpdateDate": "two"}
		if r.Method == http.MethodPost {
			updates.Add(1)
			var fields map[string]rally.Object
			if err := json.NewDecoder(r.Body).Decode(&fields); err != nil {
				t.Error(err)
			}
			body <- fields["HierarchicalRequirement"]
			object["Name"], object["LastUpdateDate"] = fields["HierarchicalRequirement"]["Name"], "three"
			json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": object}})
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": object})
	}))
	defer s.Close()
	ref = s.URL + rally.WSAPI + "hierarchicalrequirement/1"
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	v := newRallyView(rally.FindPage("teamboard"))
	d := makeDetail(rally.Object{"_ref": ref, "ObjectID": 1, "Name": "Old name", "LastUpdateDate": "one"}, "HierarchicalRequirement", false)
	v.Detail = d
	setText(d.Editors["Name"], "My name")
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if !d.Conflict || updates.Load() != 0 || text(d.Editors["Name"]) != "My name" {
		t.Fatal("revision conflict was not retained", d.Error)
	}
	a.reloadDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if d.Error != "" || len(d.fieldConflicts) != 1 {
		t.Fatal("reload failed to compare conflicting field", d.Error, d.fieldConflicts)
	}
	a.saveDetail(v)
	if updates.Load() != 0 || d.Saving {
		t.Fatal("unreviewed conflict was sent")
	}
	d.resolveFieldConflict("Name", false)
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if updates.Load() != 1 || d.Error != "" || d.Original.String("LastUpdateDate") != "three" || d.dirty() {
		t.Fatal("resolved edit was not saved against current revision", d.Error, d.values())
	}
	if got := (<-body).String("Name"); got != "My name" {
		t.Fatal("retry did not send selected local value", got)
	}
}

func TestV60ReloadProtectsLaterTypingErrorsAndConnectionChanges(t *testing.T) {
	for _, outcome := range []string{"success", "failure", "replaced", "wrong-object", "closed"} {
		t.Run(outcome, func(t *testing.T) {
			entered, release := make(chan struct{}, 1), make(chan struct{})
			var reads atomic.Int32
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				reads.Add(1)
				entered <- struct{}{}
				<-release
				if outcome == "failure" {
					w.WriteHeader(500)
					fmt.Fprint(w, `{"Errors":["Read failed"]}`)
					return
				}
				id := 1
				if outcome == "wrong-object" {
					id = 2
				}
				fmt.Fprintf(w, `{"HierarchicalRequirement":{"_ref":"http://%s/slm/webservice/v2.0/hierarchicalrequirement/%d","ObjectID":%d,"Name":"Server","LastUpdateDate":"two"}}`, r.Host, id, id)
			}))
			defer s.Close()
			defer close(release)
			a := presetApp(t)
			a.rallyClient, _ = rally.New(s.URL, "test", nil)
			v := newRallyView(rally.FindPage("teamboard"))
			d := makeDetail(rally.Object{"_ref": s.URL + rally.WSAPI + "hierarchicalrequirement/1", "ObjectID": 1, "Name": "Original", "LastUpdateDate": "one"}, "HierarchicalRequirement", false)
			v.Detail = d
			a.reloadDetail(v)
			a.reloadDetail(v)
			<-entered
			setText(d.Editors["Name"], "Typed during reload")
			if outcome == "replaced" {
				a.rallyClient, _ = rally.New(s.URL, "new connection", nil)
			}
			if outcome == "closed" {
				v.Closed = true
			}
			release <- struct{}{}
			drain(t, a, func() bool { return !d.Saving })
			if text(d.Editors["Name"]) != "Typed during reload" || reads.Load() != 1 {
				t.Fatal("reload lost typing or read twice")
			}
			if outcome == "success" {
				if len(d.fieldConflicts) != 1 || d.Original.String("LastUpdateDate") != "two" {
					t.Fatal("late typing was not compared")
				}
			} else if d.Original.String("LastUpdateDate") != "one" || outcome != "closed" && d.Error == "" {
				t.Fatal("failed/stale reload changed the baseline or hid the error", d.Error)
			}
		})
	}
}

func TestV60ConflictChoicesPersistAndRender(t *testing.T) {
	a := presetApp(t)
	id := a.state.Open(workspace.Rally, "Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	a.rallyViews[id] = v
	d := makeDetail(rally.Object{"Name": "Original"}, "HierarchicalRequirement", false)
	setText(d.Editors["Name"], "My choice")
	d.fieldConflicts = []detailFieldConflict{{Name: "Name", Before: "Before", Server: "Their choice"}}
	d.Conflict, v.Detail = true, d
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, a.tabSnapshot(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[b.state.Active].Detail
	if !restored.Conflict || len(restored.fieldConflicts) != 1 || restored.fieldConflicts[0].Server != "Their choice" {
		t.Fatal("transfer lost pending review")
	}
	for _, scale := range []float64{1, 1.5, 2} {
		h := nucular.NewHeadlessHarness(0, image.Pt(int(800*scale), int(600*scale)), func(w *nucular.Window) { a.drawDetailRecovery(w, v, d) })
		style := makeStyle(a.p, 13)
		style.Scale(scale)
		h.Master().SetStyle(style)
		h.Frame(false)
		var content strings.Builder
		for _, c := range h.Commands() {
			if c.Kind == command.TextCmd {
				content.WriteString(c.Text.String)
				content.WriteByte('\n')
			}
		}
		for _, label := range []string{"Reload item", "Keep my edit", "Use Rally value", "View full comparison", "Their choice"} {
			if !strings.Contains(content.String(), label) {
				t.Fatal("missing recovery action at scale", scale, label)
			}
		}
	}
}
