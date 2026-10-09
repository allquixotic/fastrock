//go:build fltk_headless

package ui

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/allquixotic/fastrock/internal/rally"
)

func TestV60SaveKeepsDraftWhenConnectionChanges(t *testing.T) {
	entered, release := make(chan struct{}, 1), make(chan struct{})
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		entered <- struct{}{}
		<-release
		fmt.Fprintf(w, `{"CreateResult":{"Object":{"ObjectID":1,"Name":"Saved on old connection","_ref":"http://%s/slm/webservice/v2.0/task/1"}}}`, r.Host)
	}))
	defer s.Close()
	defer close(release)
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "old", nil)
	d := makeDetail(rally.Object{"Name": "New task"}, "Task", true)
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = d
	a.saveDetail(v)
	<-entered
	a.rallyClient, _ = rally.New(s.URL, "new", nil)
	release <- struct{}{}
	drain(t, a, func() bool { return !d.Saving })
	if !d.New || text(d.Editors["Name"]) != "New task" || d.Error == "" {
		t.Fatal("old connection overwrote the current draft", d.Error, d.Original)
	}
}

func TestV60CreateConflictCanBeCorrectedWithoutReload(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusConflict)
		fmt.Fprint(w, `{"OperationResult":{"Errors":["Name is already in use"]}}`)
	}))
	defer s.Close()
	a := presetApp(t)
	a.rallyClient, _ = rally.New(s.URL, "test", nil)
	d := makeDetail(rally.Object{"Name": "New task"}, "Task", true)
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = d
	a.saveDetail(v)
	drain(t, a, func() bool { return !d.Saving })
	if d.Conflict || d.Error == "" || !d.New {
		t.Fatal("new-item validation conflict incorrectly requires an impossible reload", d.Error)
	}
}
