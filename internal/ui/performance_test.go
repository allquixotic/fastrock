package ui

import (
	"fmt"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func BenchmarkBoardFilter(b *testing.B) {
	v := newRallyView(rally.FindPage("teamboard"))
	for i := 0; i < 1000; i++ {
		v.Items = append(v.Items, rally.Object{"FormattedID": fmt.Sprintf("US%d", i), "Name": fmt.Sprintf("Searchable story %d", i), "Owner": "Alex Morgan", "ScheduleState": "In-Progress"})
	}
	setText(v.Search, "story")
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		_ = v.filtered()
	}
}

func TestWarmBoardAndSidebarDoNotAllocate(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	v.Items = []rally.Object{{"_ref": "/story/1", "Name": "First", "Owner": "Alex", "ScheduleState": "Defined"}, {"_ref": "/story/2", "Name": "Second", "Owner": "Sam", "ScheduleState": "Completed"}}
	v.prepareCards()
	items := v.filtered()
	v.prepareBoardLayout(items)
	if n := testing.AllocsPerRun(100, func() { v.prepareCards(); v.prepareBoardLayout(v.filtered()) }); n != 0 {
		t.Fatalf("warm board allocates %g", n)
	}
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	a.state.Chats["1"] = &workspace.Conversation{ID: "1", Title: "Thread", Cwd: "/repo"}
	waitSidebar(t, a)
	if n := testing.AllocsPerRun(100, func() { a.sidebarFolders() }); n != 0 {
		t.Fatalf("warm sidebar allocates %g", n)
	}
}

func TestFilterInvalidationAndHTMLSearch(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	v.Items = []rally.Object{{"_ref": "/1", "Description": "<b>Find</b> &amp; <em>me</em>", "Owner": "Alex", "Blocked": true}, {"_ref": "/2", "Owner": "Sam", "Ready": true}}
	setText(v.Search, "find & me")
	if len(v.filtered()) != 1 {
		t.Fatal("HTML search failed")
	}
	setText(v.Search, "")
	v.OnlyReady = true
	if len(v.filtered()) != 1 || v.filtered()[0].String("_ref") != "/2" {
		t.Fatal("ready filter")
	}
	v.OnlyReady = false
	v.OwnerFilter = "Alex"
	if len(v.filtered()) != 1 {
		t.Fatal("owner filter")
	}
	v.Items = []rally.Object{{"_ref": "/3", "Owner": "Alex"}}
	v.Generation++
	if v.filtered()[0].String("_ref") != "/3" {
		t.Fatal("stale cache")
	}
}

func BenchmarkEditorSnapshot(b *testing.B) {
	e := textEditor(strings.Repeat("A Rally description with Unicode 🚀. ", 100), true)
	e.Snapshot()
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		_ = e.Snapshot()
	}
}

func BenchmarkFontWidthCached(b *testing.B) {
	f := makeStyle(colors(false), 13).Font
	nucular.FontWidth(f, "Searchable story with an owner")
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		_ = nucular.FontWidth(f, "Searchable story with an owner")
	}
}
