package ui

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func drain(t *testing.T, a *App, done func() bool) {
	t.Helper()
	until := time.After(3 * time.Second)
	for !done() {
		select {
		case f := <-a.updates:
			f()
		case <-until:
			t.Fatal("background work did not finish")
		}
	}
}
func TestRallyLoadIsDemandDrivenAndBounded(t *testing.T) {
	var calls atomic.Int64
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		start := r.URL.Query().Get("start")
		fmt.Fprintf(w, `{"QueryResult":{"Results":[{"_ref":"/item/%s","Name":"Card","FormattedID":"US%s"}],"StartIndex":%s,"TotalResultCount":10000,"PageSize":128}}`, start, start, start)
	}))
	defer s.Close()
	client, _ := rally.New(s.URL, "test", nil)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 10), rallyClient: client}
	v := newRallyView(rally.FindPage("teamboard"))
	a.refreshRally(v)
	drain(t, a, func() bool { return !v.Loading })
	if calls.Load() != 1 || v.Total != 10000 || len(v.Items) != 1 || !v.More {
		t.Fatal("eagerly queried whole workspace or lost pagination")
	}
	a.needRallyPage(v)
	drain(t, a, func() bool { return !v.Loading })
	if calls.Load() != 2 || len(v.Items) != 2 {
		t.Fatal("did not advance on demand")
	}
}
func TestTabTransferKeepsFormattedDraftAndQueue(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, state: workspace.NewState(), chats: map[string]*chatView{}, files: map[string]*fileView{}, rallyViews: map[string]*rallyView{}, prefs: settings.Defaults()}
	tabID := a.state.Open(workspace.Rally, "Team Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = makeDetail(rally.Object{"Name": "Story", "Description": "<p>original</p>"}, "HierarchicalRequirement", false)
	setText(v.Detail.Rich["Description"].editor, "Unsaved 🚀")
	a.rallyViews[tabID] = v
	transfer := a.tabSnapshot(*a.state.Current())
	setText(v.Detail.Rich["Description"].editor, "later edit")
	b := &App{ctx: ctx, state: workspace.NewState(), chats: map[string]*chatView{}, files: map[string]*fileView{}, rallyViews: map[string]*rallyView{}, prefs: settings.Defaults()}
	b.installTransfer(transfer)
	d := b.rallyViews[b.state.Active].Detail
	if got := d.Rich["Description"].html(); got != "<p>Unsaved 🚀</p>" {
		t.Fatalf("draft changed during transfer: %q", got)
	}
	snapshot := b.sessionSnapshot()
	if len(snapshot.Documents) != 1 {
		t.Fatal("unsaved Rally draft missing from session")
	}
}
func TestMergeTranscriptKeepsLiveDeltas(t *testing.T) {
	history := []workspace.Block{{ID: "1", Text: "hello"}}
	live := []workspace.Block{{ID: "1", Text: "hello world"}, {ID: "2", Text: "later"}}
	merged := mergeTranscript(history, live)
	if len(merged) != 2 || merged[0].Text != "hello world" {
		t.Fatal("lost live text during resume")
	}
}

func TestClosedRallyResponseCannotRepopulate(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"QueryResult":{"Results":[{"Name":"late"}],"StartIndex":1,"TotalResultCount":1}}`)
	}))
	defer server.Close()
	client, _ := rally.New(server.URL, "test", nil)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 10), rallyClient: client}
	v := newRallyView(rally.FindPage("teamboard"))
	a.refreshRally(v)
	v.Closed = true
	select {
	case f := <-a.updates:
		f()
	case <-time.After(2 * time.Second):
		t.Fatal("request did not finish")
	}
	if len(v.Items) != 0 {
		t.Fatal("closed view was resurrected")
	}
}
func TestRallyFailureBackoff(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusBadRequest) }))
	defer server.Close()
	client, _ := rally.New(server.URL, "test", nil)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 10), rallyClient: client}
	v := newRallyView(rally.FindPage("teamboard"))
	a.refreshRally(v)
	drain(t, a, func() bool { return !v.Loading })
	if v.Failures != 1 || time.Until(v.RetryAfter) < 4*time.Second {
		t.Fatal("failed refresh can spin")
	}
}

func TestRallyResidentWindowDoesNotMutatePageCache(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"QueryResult":{"Results":[{"Name":"cached"}],"StartIndex":1,"TotalResultCount":1}}`)
	}))
	defer server.Close()
	client, _ := rally.New(server.URL, "test", nil)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 10), rallyClient: client}
	v := newRallyView(rally.FindPage("teamboard"))
	a.refreshRally(v)
	drain(t, a, func() bool { return !v.Loading })
	q := a.rallyQuery(v)
	q.Start = 1
	q.PageSize = rallyPageSize
	q.Fetch = cardFields
	page, err := client.CachedQuery(ctx, v.Spec.Kind, q, false)
	if err != nil {
		t.Fatal(err)
	}
	clear(v.Items)
	if len(page.Results) != 1 || page.Results[0].String("Name") != "cached" {
		t.Fatal("eviction mutated shared cache")
	}
}

func TestRealFileContentsExcludedFromSession(t *testing.T) {
	a := &App{state: workspace.NewState(), files: map[string]*fileView{}, chats: map[string]*chatView{}, rallyViews: map[string]*rallyView{}}
	id := a.state.Open(workspace.File, "secret", "/test/config.toml", "")
	a.files[id] = &fileView{Editor: textEditor("secret content", true)}
	doc := a.sessionSnapshot().Documents[0]
	if doc.File == nil || doc.File.Text != "" || doc.File.TextLoaded {
		t.Fatal("disk file contents persisted in session")
	}
}

func TestIncomingRallyDoesNotReplaceExistingDraft(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, state: workspace.NewState(), chats: map[string]*chatView{}, files: map[string]*fileView{}, rallyViews: map[string]*rallyView{}}
	id := a.state.Open(workspace.Rally, "Team Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = makeDetail(rally.Object{"Description": "<p>local draft</p>"}, "HierarchicalRequirement", false)
	a.rallyViews[id] = v
	a.transferPending = true
	a.incomingTicket = "incoming"
	a.installTransfer(tabTransfer{Tab: workspace.Tab{ID: "source-tab", Kind: workspace.Rally, Title: "Team Board", Page: "teamboard"}, Rally: &rallyTransfer{}})
	if len(a.state.Tabs) != 2 || a.rallyViews[id] != v || a.incomingTab == id {
		t.Fatal("incoming document replaced destination draft")
	}
	a.cancelIncoming()
	if len(a.state.Tabs) != 1 || a.rallyViews[id] != v {
		t.Fatal("cancel removed original document")
	}
}

func TestRallyScopeAndTimeboxResetPaging(t *testing.T) {
	a := &App{prefs: settings.Defaults()}
	v := newRallyView(rally.FindPage("teamboard"))
	original := a.rallySignature(v)
	v.Timebox = "/iteration/2"
	if a.rallySignature(v) == original {
		t.Fatal("timebox omitted from query identity")
	}
	original = a.rallySignature(v)
	a.prefs.RallyProject = "/project/other"
	if a.rallySignature(v) == original {
		t.Fatal("project omitted from query identity")
	}
}
