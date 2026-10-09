//go:build fltk_headless

package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func presetApp(t *testing.T) *App {
	t.Helper()
	a := transferFixture()
	a.ctx, a.cancel = context.WithCancel(context.Background())
	t.Cleanup(a.cancel)
	a.updates = make(chan func(), 128)
	a.prefs, a.p = settings.Defaults(), colors(false)
	a.navigationFace = typeFace(13, regularFont)
	return a
}

func TestV55PagePresetQueries(t *testing.T) {
	a := presetApp(t)
	backlog := newRallyView(rally.FindPage("backlog"))
	stories := newRallyView(rally.FindPage("userstories"))
	backlog.QueryApplied = `(Name contains "kept")`
	if q := a.rallyQuery(backlog).Expression; !strings.Contains(q, "Iteration = null") || !strings.Contains(q, `Name contains "kept"`) {
		t.Fatal(q)
	}
	if q := a.rallyQuery(stories).Expression; strings.Contains(q, "Iteration = null") {
		t.Fatal("story list became backlog", q)
	}
	v := newRallyView(rally.FindPage("mywork"))
	if a.prepareRallyPreset(v) || !strings.Contains(a.rallyQuery(v).Expression, "ObjectID = 0") {
		t.Fatal("missing identity exposed everybody's work")
	}
	a.rallyUser = rally.Object{"_ref": "/slm/webservice/v2.0/user/42"}
	if !a.prepareRallyPreset(v) || !strings.Contains(a.rallyQuery(v).Expression, `Owner = "/slm/webservice/v2.0/user/42"`) {
		t.Fatal(a.rallyQuery(v))
	}
	v = newRallyView(rally.FindPage("iterationstatus"))
	if !v.CurrentIteration || !v.Widgets || a.prepareRallyPreset(v) {
		t.Fatal("iteration defaults or waiting state absent")
	}
	a.iterationScope = a.rallyPresetScope()
	if !a.prepareRallyPreset(v) || !strings.Contains(a.rallyQuery(v).Expression, "ObjectID = 0") {
		t.Fatal("no current iteration queried every timebox")
	}
}

func TestV55CurrentIterationSelection(t *testing.T) {
	now := time.Date(2026, 10, 9, 12, 0, 0, 0, time.UTC)
	iteration := func(ref, project string, start, end time.Time) rally.Object {
		return rally.Object{"_ref": ref, "Project": map[string]any{"_ref": project}, "StartDate": start.Format(time.RFC3339), "EndDate": end.Format(time.RFC3339)}
	}
	old := iteration("/old", "/project/1", now.Add(-48*time.Hour), now)
	other := iteration("/other", "/project/2", now.Add(-time.Hour), now.Add(time.Hour))
	first := iteration("/first", "/project/1", now.Add(-4*time.Hour), now.Add(time.Hour))
	latest := iteration("/latest", "/project/1", now.Add(-2*time.Hour), now.Add(time.Hour))
	rows := []rally.Object{other, old, latest, first, {"_ref": "/bad", "StartDate": "bad"}, iteration("/future", "/project/1", now.Add(time.Hour), now.Add(2*time.Hour))}
	if got := currentIteration(rows, "/project/1", now); got != "/latest" {
		t.Fatal("did not prefer exact project and latest start", got)
	}
	if got := currentIteration(rows, "", now); got != "/other" {
		t.Fatal("unscoped current iteration", got)
	}
	if got := currentIteration([]rally.Object{old}, "/project/1", now); got != "" {
		t.Fatal("end boundary included", got)
	}
	if got := currentIteration([]rally.Object{first}, "/project/1", now.Add(-4*time.Hour)); got != "/first" {
		t.Fatal("start boundary excluded", got)
	}
}

func TestV55BlockedPresetCancelsStaleRequest(t *testing.T) {
	a := presetApp(t)
	v := newRallyView(rally.FindPage("iterationstatus"))
	cancelled := false
	v.cancel = func() { cancelled = true }
	v.Loading, v.Generation = true, 7
	a.refreshRallyItems(v)
	if !cancelled || v.Loading || v.Generation != 8 || v.PresetError == "" {
		t.Fatal("prior scope can still publish while preset waits", v)
	}
	a.iterationError = "fixture failure"
	a.refreshRallyItems(v)
	if !strings.Contains(v.PresetError, "fixture failure") {
		t.Fatal(v.PresetError)
	}
}

func TestV55IdentityLoadsBeforeMyWorkQuery(t *testing.T) {
	var reads atomic.Int32
	queries := make(chan string, 2)
	gate := make(chan struct{})
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == rally.WSAPI+"user" {
			<-gate
			json.NewEncoder(w).Encode(map[string]any{"User": rally.Object{"ObjectID": 42, "_ref": "http://" + r.Host + rally.WSAPI + "user/42"}})
			return
		}
		if r.URL.Path == rally.WSAPI+"hierarchicalrequirement" {
			reads.Add(1)
			queries <- r.URL.Query().Get("query")
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	v := newRallyView(rally.FindPage("mywork"))
	a.rallyViews["mywork"] = v
	a.loadRallyUser()
	a.refreshRallyItems(v)
	if reads.Load() != 0 || v.Loading || v.PresetError == "" {
		t.Fatal("My Work loaded before identity")
	}
	close(gate)
	drain(t, a, func() bool { return !a.rallyUserLoading && !v.Loading && v.PresetError == "" })
	if reads.Load() != 1 {
		t.Fatal("identity did not wake exactly one page load", reads.Load())
	}
	select {
	case query := <-queries:
		if !strings.Contains(query, `Owner = "`+s.URL+rally.WSAPI+`user/42"`) {
			t.Fatal(query)
		}
	default:
		t.Fatal("missing owner-scoped request")
	}
}

func TestV55IdentityFailureAndReplacedClient(t *testing.T) {
	for _, replaced := range []bool{false, true} {
		t.Run(fmt.Sprint(replaced), func(t *testing.T) {
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusUnauthorized) }))
			defer s.Close()
			a := presetApp(t)
			a.rallyClient, _ = rally.New(s.URL, "test", nil)
			v := newRallyView(rally.FindPage("mywork"))
			a.rallyViews["mywork"] = v
			a.loadRallyUser()
			if replaced {
				a.rallyClient = nil
				a.rallyUserLoading, a.rallyUserError = false, "current"
			}
			select {
			case update := <-a.updates:
				update()
			case <-time.After(3 * time.Second):
				t.Fatal("identity did not finish")
			}
			if replaced {
				if a.rallyUserError != "current" || a.rallyUserLoading {
					t.Fatal("replaced client published identity failure")
				}
			} else if a.rallyUserLoading || a.rallyUserError == "" || v.PresetError == "" {
				t.Fatal("identity failure was not actionable", a.rallyUserError, v.PresetError)
			}
		})
	}
}

func TestV55IterationScopeWakesCurrentPage(t *testing.T) {
	queries := make(chan string, 2)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rows := []rally.Object{}
		if r.URL.Path == rally.WSAPI+"iteration" {
			if !strings.Contains(r.URL.Query().Get("fetch"), "Project") {
				t.Error("current iteration has no project metadata")
			}
			rows = append(rows, rally.Object{"_ref": "http://" + r.Host + rally.WSAPI + "iteration/9", "Name": "Current", "StartDate": time.Now().Add(-time.Hour).Format(time.RFC3339), "EndDate": time.Now().Add(time.Hour).Format(time.RFC3339)})
		}
		if r.URL.Path == rally.WSAPI+"hierarchicalrequirement" {
			queries <- r.URL.Query().Get("query")
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": rows, "TotalResultCount": len(rows), "StartIndex": 1}})
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	v := newRallyView(rally.FindPage("iterationstatus"))
	a.rallyViews["current"] = v
	a.loadScope()
	drain(t, a, func() bool { return a.iterationScope == a.rallyPresetScope() && !v.Loading })
	if v.Timebox != s.URL+rally.WSAPI+"iteration/9" || v.PresetError != "" {
		t.Fatal("scope did not resolve current iteration", v.Timebox, v.PresetError)
	}
	select {
	case query := <-queries:
		if !strings.Contains(query, `Iteration.Name = "Current"`) {
			t.Fatal(query)
		}
	default:
		t.Fatal("current iteration page was not loaded")
	}
}

func TestV55NewOwnerDefaultsRespectEditsAndTransfer(t *testing.T) {
	user := rally.Object{"_ref": "/slm/webservice/v2.0/user/42", "_refObjectName": "Current User"}
	for _, edit := range []string{"untouched", "chosen", "cleared"} {
		t.Run(edit, func(t *testing.T) {
			a := presetApp(t)
			id := a.state.Open(workspace.Rally, "Backlog", "", "backlog")
			v := newRallyView(rally.FindPage("backlog"))
			a.rallyViews[id] = v
			a.newArtifact(v)
			d := v.Detail
			if !d.pendingDefaultOwner() {
				t.Fatal("creation did not retain pending owner default")
			}
			if edit != "untouched" {
				setText(d.Editors["Owner"], "/slm/webservice/v2.0/user/99")
				if edit == "cleared" {
					setText(d.Editors["Owner"], "")
				}
			}
			transfer := transferJSON(t, a.tabSnapshot(*a.state.Current()))
			b := presetApp(t)
			b.rallyUser = user
			if err := b.installTransfer(transfer); err != nil {
				t.Fatal(err)
			}
			want := text(d.Editors["Owner"])
			if edit == "untouched" {
				want = user.String("_ref")
			}
			applyDefaultOwner(d, user)
			if got := text(d.Editors["Owner"]); got != want {
				t.Fatal("late identity replaced explicit input", got, want)
			}
			restored := b.rallyViews[b.state.Active].Detail
			if got := text(restored.Editors["Owner"]); got != want {
				t.Fatal("transfer lost pending default or explicit input", got, want)
			}
		})
	}
	a := presetApp(t)
	a.rallyUser = user
	a.prefs.RallyProject = "/project/1"
	v := newRallyView(rally.FindPage("teamboard"))
	v.Timebox = "/iteration/9"
	v.Workflow = []rally.Object{{"Name": "In-Progress"}}
	a.newArtifactInState(v, "In-Progress")
	if o := v.Detail.Original; o.Ref("Owner") != user.String("_ref") || o.Ref("Project") != "/project/1" || o.Ref("Iteration") != "/iteration/9" || o.String("ScheduleState") != "In-Progress" {
		t.Fatal("creation lost contextual defaults", o)
	}
}

func TestV55CurrentIterationSavedViewFollowsTime(t *testing.T) {
	v := newRallyView(rally.FindPage("iterationstatus"))
	v.Timebox = "/iteration/old"
	saved := v.savedView("Current")
	v.Timebox = "/iteration/new"
	if saved.Timebox != "" || !saved.CurrentIteration || !savedViewEqual(saved, v.savedView("Current")) {
		t.Fatal("derived timebox dirtied or pinned a saved current view", saved)
	}
	v.applySavedView(settings.SavedView{Name: "All", Page: "iterationstatus"})
	if v.CurrentIteration || v.Timebox != "" {
		t.Fatal("explicit All timeboxes did not survive")
	}
	v.applySavedView(settings.SavedView{})
	if !v.CurrentIteration {
		t.Fatal("Standard View did not restore current iteration")
	}
	if chips := (&App{}).activeRallyFilters(v); len(chips) != 1 || chips[0].Label != "Timebox: Current iteration" {
		t.Fatal("current-iteration constraint is hidden", chips)
	}
	v.removeFilter("timebox")
	if v.CurrentIteration || v.Timebox != "" {
		t.Fatal("removed timebox constraint reapplies itself")
	}
	v.applySavedView(settings.SavedView{})
	a := presetApp(t)
	id := a.state.Open(workspace.Rally, "Current", "", "iterationstatus")
	a.rallyViews[id] = v
	b := presetApp(t)
	if err := b.installTransfer(transferJSON(t, a.tabSnapshot(*a.state.Current()))); err != nil {
		t.Fatal(err)
	}
	if !b.rallyViews[b.state.Active].CurrentIteration {
		t.Fatal("transfer pinned the current iteration")
	}
}

func TestV55IterationRolloverRestartsPagination(t *testing.T) {
	starts := make(chan string, 2)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == rally.WSAPI+"hierarchicalrequirement" {
			starts <- r.URL.Query().Get("start")
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[],"TotalResultCount":0,"StartIndex":1}}`)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	a.iterationScope = a.rallyPresetScope()
	a.iterations = []rally.Object{{"_ref": "/slm/webservice/v2.0/iteration/next", "StartDate": time.Now().Add(-time.Hour).Format(time.RFC3339), "EndDate": time.Now().Add(time.Hour).Format(time.RFC3339)}}
	v := newRallyView(rally.FindPage("iterationstatus"))
	v.Timebox, v.Start = "/slm/webservice/v2.0/iteration/prior", 129
	v.signature = a.rallySignature(v)
	a.requestRallyPage(v, 257, false)
	drain(t, a, func() bool { return !v.Loading })
	select {
	case start := <-starts:
		if start != "1" {
			t.Fatal("new iteration used the old continuation cursor", start)
		}
	default:
		t.Fatal("rollover did not reload")
	}
}

func TestV55PresetQueueRejectionIsRetryable(t *testing.T) {
	for _, page := range []string{"mywork", "iterationstatus"} {
		t.Run(page, func(t *testing.T) {
			a := presetApp(t)
			a.rallyClient = &rally.Client{}
			v := newRallyView(rally.FindPage(page))
			a.rallyViews[page] = v
			entered := make(chan struct{}, 6)
			release := make(chan struct{})
			defer close(release)
			for range 6 {
				a.work(func() { entered <- struct{}{}; <-release })
			}
			for range 6 {
				select {
				case <-entered:
				case <-time.After(time.Second):
					t.Fatal("worker did not enter fixture")
				}
			}
			for range 64 {
				if !a.work(func() {}) {
					t.Fatal("fixture failed to fill read queue")
				}
			}
			if page == "mywork" {
				a.loadRallyUser()
			} else {
				a.loadScope()
			}
			drain(t, a, func() bool { return strings.Contains(v.PresetError, errWorkQueueFull.Error()) })
			if a.rallyUserLoading || v.Loading {
				t.Fatal("rejected work remained in a loading state")
			}
		})
	}
}

func TestV55CreateSendsContextualDefaults(t *testing.T) {
	body := make(chan rally.Object, 1)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var envelope map[string]rally.Object
		if r.Method != http.MethodPost || r.URL.Path != rally.WSAPI+"hierarchicalrequirement/create" {
			t.Errorf("unexpected request: %s %s", r.Method, r.URL)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		if err := json.NewDecoder(r.Body).Decode(&envelope); err != nil {
			t.Error(err)
		}
		body <- envelope["HierarchicalRequirement"]
		json.NewEncoder(w).Encode(map[string]any{"CreateResult": map[string]any{"Object": rally.Object{"ObjectID": 4, "_ref": "http://" + r.Host + rally.WSAPI + "hierarchicalrequirement/4", "Name": "New story"}}})
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	d := makeDetail(rally.Object{"Owner": map[string]any{"_ref": "/user/42"}, "Iteration": map[string]any{"_ref": "/iteration/9"}, "Project": map[string]any{"_ref": "/project/1"}, "ScheduleState": "In-Progress"}, "HierarchicalRequirement", true)
	setText(d.Editors["Name"], "New story")
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = d
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if d.Error != "" || d.New {
		t.Fatal("create failed", d.Error)
	}
	if text(d.Editors["Owner"]) != "/user/42" || text(d.Editors["Iteration"]) != "/iteration/9" || text(d.Editors["ScheduleState"]) != "In-Progress" || d.dirty() {
		t.Fatal("sparse save response cleared acknowledged defaults", d.values())
	}
	select {
	case sent := <-body:
		for k, want := range map[string]string{"Owner": "/user/42", "Iteration": "/iteration/9", "Project": "/project/1", "ScheduleState": "In-Progress", "Name": "New story"} {
			if sent.String(k) != want {
				t.Fatalf("create lost default %s: %#v", k, sent)
			}
		}
		if _, exists := sent["Ready"]; exists {
			t.Fatal("absent fallback field was sent", sent)
		}
	default:
		t.Fatal("creation did not reach fixture")
	}
}
