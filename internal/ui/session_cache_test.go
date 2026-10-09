//go:build fltk_headless

package ui

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func snapshotFixture() *App {
	return &App{state: workspace.NewState(), chats: map[string]*chatView{}, files: map[string]*fileView{}, rallyViews: map[string]*rallyView{}, updates: make(chan func(), 16), sessions: make(chan session, 1)}
}

func addCheckpointDetail(a *App, id string) *rallyView {
	t := workspace.Tab{ID: id, Kind: workspace.Rally, Title: id, Page: "teamboard"}
	a.state.Tabs = append(a.state.Tabs, t)
	v := newRallyView(rally.FindPage(t.Page))
	v.Detail = makeDetail(rally.Object{"_ref": "/item/" + id, "Name": "original", "Description": "<p>original</p>"}, "HierarchicalRequirement", false)
	a.rallyViews[id] = v
	return v
}

func TestV4CheckpointsReuseUnchangedDocuments(t *testing.T) {
	a := snapshotFixture()
	v := addCheckpointDetail(a, "first")
	addCheckpointDetail(a, "second")
	s1 := a.sessionSnapshot()
	s2 := a.sessionSnapshot()
	if s1.Documents[0].Rally != s2.Documents[0].Rally || !sameSessionCheckpoint(&s1, &s2) {
		t.Fatal("unchanged document cloned")
	}
	v.Detail.Rich["Description"].ensureEditor()
	v.Detail.Rich["Description"].editor.Paste("updated ")
	s3 := a.sessionSnapshot()
	if s3.Documents[0].Rally == s1.Documents[0].Rally || s3.Documents[1].Rally != s1.Documents[1].Rally {
		t.Fatal("changed wrong document")
	}
	if s1.Documents[0].Rally.Detail.Rich["Description"].Changed {
		t.Fatal("mutated published snapshot")
	}
	before := s3.Documents[0].Rally
	v.Detail.setStates([]rally.Object{{"Name": "New state"}})
	s4 := a.sessionSnapshot()
	if s4.Documents[0].Rally == before || len(s4.Documents[0].Rally.Detail.States) != 1 {
		t.Fatal("metadata change not checkpointed")
	}
	v.selectItem(rally.Object{"_ref": "/item/3", "Name": "selected"}, true)
	s5 := a.sessionSnapshot()
	if len(s5.Documents[0].Rally.Selected) != 1 {
		t.Fatal("selection change lost")
	}
	a.state.Tabs = a.state.Tabs[:1]
	a.sessionSnapshot()
	if len(a.documentCheckpoints) != 1 {
		t.Fatal("closed document snapshot retained")
	}
}

func TestV4CheckpointChatCopiesOnlyChanges(t *testing.T) {
	a := snapshotFixture()
	c := &workspace.Conversation{ID: "chat", Draft: "draft", Agents: []workspace.Agent{{Name: "old"}}}
	c.Enqueue("queued", []string{"before"})
	a.state.Chats[c.ID] = c
	a.state.Tabs = []workspace.Tab{{ID: "tab", Kind: workspace.Chat, Target: c.ID}}
	s1, s2 := a.sessionSnapshot(), a.sessionSnapshot()
	if s1.Chats[c.ID] != s2.Chats[c.ID] {
		t.Fatal("unchanged chat copied")
	}
	c.Queue[0].Attachments[0] = "after"
	c.Agents[0].Name = "new"
	s3 := a.sessionSnapshot()
	if s3.Chats[c.ID] == s2.Chats[c.ID] || s1.Chats[c.ID].Queue[0].Attachments[0] != "before" || s1.Chats[c.ID].Agents[0].Name != "old" {
		t.Fatal("published chat snapshot mutated")
	}
}

func TestV4CheckpointDebouncesLatestSnapshot(t *testing.T) {
	a := snapshotFixture()
	a.ctx = context.Background()
	v := addCheckpointDetail(a, "detail")
	a.checkpoint()
	if a.checkpointTimer != nil {
		defer a.checkpointTimer.Stop()
	}
	setText(v.Detail.Editors["Name"], "latest")
	a.lastCheckpoint = time.Time{}
	a.checkpoint()
	defer a.checkpointTimer.Stop()
	deadline := time.After(2 * time.Second)
	for {
		select {
		case update := <-a.updates:
			update()
		case got := <-a.sessions:
			if got.Documents[0].Rally.Detail.Values["Name"] != "latest" {
				t.Fatal("stale checkpoint published")
			}
			a.lastCheckpoint = time.Time{}
			a.checkpoint()
			if a.checkpointGeneration != 2 {
				t.Fatal("idle frame rescheduled persistence")
			}
			return
		case <-deadline:
			t.Fatal("debounced snapshot was not published")
		}
	}
}

func TestV4CheckpointObservesLastEditWithoutAnotherInput(t *testing.T) {
	a := snapshotFixture()
	a.ctx = context.Background()
	defer a.stopCheckpointTimers()
	v := addCheckpointDetail(a, "detail")
	a.checkpoint()
	setText(v.Detail.Editors["Name"], "last keystroke")
	// The next event falls inside the sampling interval and no later user
	// input arrives. The timer must wake the UI and observe this final edit.
	a.checkpoint()
	deadline := time.After(2 * time.Second)
	for {
		select {
		case update := <-a.updates:
			update()
			a.checkpoint()
		case got := <-a.sessions:
			if got.Documents[0].Rally.Detail.Values["Name"] == "last keystroke" {
				return
			}
		case <-deadline:
			t.Fatal("last edit never checkpointed")
		}
	}
}

func TestV4FinalSessionFlushCapturesPendingEdits(t *testing.T) {
	a := snapshotFixture()
	a.store = &settings.Store{Dir: t.TempDir()}
	a.ctx = context.Background()
	defer a.stopCheckpointTimers()
	c := &workspace.Conversation{ID: "chat", Draft: "old"}
	a.state.Chats[c.ID] = c
	a.state.Tabs = []workspace.Tab{{ID: "tab", Kind: workspace.Chat, Target: c.ID}}
	a.chats[c.ID] = &chatView{Editor: textEditor("old", true)}
	v := addCheckpointDetail(a, "detail")
	a.checkpoint()
	setText(a.chats[c.ID].Editor, "last unsent draft")
	setText(v.Detail.Editors["Name"], "last title")
	if err := a.saveSession(); err != nil {
		t.Fatal(err)
	}
	loaded := readSessions(a.store.Dir)
	if loaded.primary == nil || loaded.primary.Chats[c.ID].Draft != "last unsent draft" || loaded.primary.Documents[0].Rally.Detail.Values["Name"] != "last title" {
		t.Fatalf("final flush missed pending edits: %+v", loaded)
	}
}

func BenchmarkV4UnchangedLargeCheckpoints(b *testing.B) {
	a := snapshotFixture()
	for i := 0; i < 8; i++ {
		v := addCheckpointDetail(a, fmt.Sprint(i))
		v.Detail.Rich["Description"].ensureEditor()
		setText(v.Detail.Rich["Description"].editor, strings.Repeat("body ", 100000))
	}
	t := workspace.Tab{ID: "file", Kind: workspace.File, Target: "/tmp/file"}
	a.state.Tabs = append(a.state.Tabs, t)
	a.files[t.ID] = &fileView{Editor: textEditor(strings.Repeat("x", 4<<20), true)}
	a.sessionSnapshot()
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		a.sessionSnapshot()
	}
}
