//go:build fltk_headless

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
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestV70DetailRefreshRetainsDraftAndCollection(t *testing.T) {
	var fail atomic.Bool
	var ref, collection string
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/Tasks") {
			if fail.Load() {
				w.WriteHeader(http.StatusBadRequest)
				fmt.Fprint(w, `{"QueryResult":{"Errors":["collection unavailable"]}}`)
				return
			}
			json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": []rally.Object{{"_ref": "/task/2", "Name": "Updated task"}}, "TotalResultCount": 1, "StartIndex": 1}})
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": rally.Object{"_ref": ref, "ObjectID": 1, "Name": "Server name", "LastUpdateDate": "two", "Tasks": map[string]any{"_ref": collection, "Count": 1}}})
	}))
	defer s.Close()
	ref = s.URL + rally.WSAPI + "hierarchicalrequirement/1"
	collection = ref + "/Tasks"
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	v := newRallyView(rally.FindPage("teamboard"))
	d := makeDetail(rally.Object{"_ref": ref, "ObjectID": 1, "Name": "Original", "LastUpdateDate": "one", "Tasks": map[string]any{"_ref": collection, "Count": 1}}, "HierarchicalRequirement", false)
	v.Detail = d
	d.Tab = "Tasks"
	d.Refreshed = time.Now().Add(-time.Hour)
	d.Items = []rally.Object{{"_ref": "/task/1", "Name": "Old task"}}
	setText(d.Editors["Name"], "My name")
	d.CommentRich = newRichEditor("<p>Unsent comment</p>")
	a.refreshRally(v)
	busy, enabled, _ := rallyRefreshState(v)
	if !busy || enabled {
		t.Fatal("refresh did not disable overlapping action")
	}
	drain(t, a, func() bool { return !d.Saving && !d.CollectionLoading })
	if d.Error != "" || text(d.Editors["Name"]) != "My name" || d.CommentRich.html() != "<p>Unsent comment</p>" || len(d.fieldConflicts) != 1 || d.Tab != "Tasks" || len(d.Items) != 1 || d.Items[0].String("Name") != "Updated task" || time.Since(d.Refreshed) > time.Second {
		t.Fatal("refresh lost draft, comparison, collection, or freshness", d.Error, d.Items, d.fieldConflicts)
	}
	prior := d.Refreshed
	fail.Store(true)
	a.refreshRally(v)
	drain(t, a, func() bool { return !d.Saving && !d.CollectionLoading })
	if d.Error == "" || d.Refreshed != prior || text(d.Editors["Name"]) != "My name" || d.Items[0].String("Name") != "Updated task" {
		t.Fatal("failed collection refresh changed freshness or discarded prior content", d.Error)
	}
}

func TestV70RallyRefreshState(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	v.Refreshed = time.Now().Add(-10 * time.Second)
	busy, enabled, at := rallyRefreshState(v)
	if busy || !enabled || at != v.Refreshed {
		t.Fatal("idle page state")
	}
	v.Loading = true
	busy, enabled, _ = rallyRefreshState(v)
	if !busy || enabled {
		t.Fatal("loading page allowed refresh")
	}
	v.Loading = false
	d := makeDetail(rally.Object{}, "Task", false)
	v.Detail = d
	for _, flag := range []*bool{&d.Loading, &d.SchemaLoading, &d.Saving, &d.Pending, &d.CollectionLoading} {
		*flag = true
		busy, enabled, _ = rallyRefreshState(v)
		if !busy || enabled {
			t.Fatal("busy detail allowed refresh")
		}
		*flag = false
	}
	d.New = true
	_, enabled, _ = rallyRefreshState(v)
	if enabled {
		t.Fatal("new item allowed refresh")
	}
	d.New = false
	d.Refreshed = time.Now()
	_, _, at = rallyRefreshState(v)
	if at != d.Refreshed {
		t.Fatal("detail showed underlying page freshness")
	}
	for _, tc := range []struct {
		age  time.Duration
		want string
	}{{3 * time.Second, "Updated 3 s ago"}, {90 * time.Second, "Updated 1 min ago"}, {2 * time.Hour, "Updated 2 h ago"}, {48 * time.Hour, "Updated 2 d ago"}, {-time.Second, "Updated 0 s ago"}} {
		now := time.Now()
		if got := refreshAge(now.Add(-tc.age), now); got != tc.want {
			t.Fatal(got, tc.want)
		}
	}
	a := presetApp(t)
	a.state.Tabs = []workspace.Tab{{ID: "rally", Kind: workspace.Rally}}
	a.state.Active = "rally"
	a.rallyViews["rally"] = v
	a.rallyClient, _ = rally.New("https://rally.invalid", "test", nil)
	d.Saving = true
	a.scheduleTimeUpdate()
	if a.timeRefreshInterval != 100*time.Millisecond {
		t.Fatal("spinner not scheduled")
	}
	d.Saving = false
	a.scheduleTimeUpdate()
	if a.timeRefreshInterval != 5*time.Second {
		t.Fatal("busy cadence persisted while idle")
	}
	a.timeRefresh.Stop()
}

func TestV70RallyFreshnessRendering(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := &App{p: colors(false)}
			v := newRallyView(rally.FindPage("teamboard"))
			v.Loading = true
			v.Refreshed = time.Now().Add(-10 * time.Second)
			h := desktop.NewHeadlessHarness(0, image.Pt(int(500*scale), int(120*scale)), func(w *desktop.Window) { a.drawRallyFreshness(w, v) })
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			lines, label := 0, ""
			for _, c := range h.Commands() {
				if c.Kind == command.LineCmd {
					lines++
				}
				if c.Kind == command.TextCmd {
					label += c.Text.String
				}
			}
			if lines != 12 || !strings.Contains(label, "Updating…") || !strings.Contains(label, "Updated 10 s ago") {
				t.Fatal("missing busy/freshness indication", lines, label)
			}
			v.Loading = false
			h.Frame(false)
			for _, c := range h.Commands() {
				if c.Kind == command.LineCmd {
					t.Fatal("idle spinner still drawn")
				}
			}
		})
	}
}
