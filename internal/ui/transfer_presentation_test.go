package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func transferFixture() *App {
	return &App{state: workspace.NewState(), chats: map[string]*chatView{}, rallyViews: map[string]*rallyView{}, files: map[string]*fileView{}}
}

func transferJSON(t *testing.T, p tabTransfer) tabTransfer {
	t.Helper()
	raw, err := json.Marshal(p)
	if err != nil {
		t.Fatal(err)
	}
	var out tabTransfer
	if err = json.Unmarshal(raw, &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func TestV23TransferPreservesRichAndBoardPresentation(t *testing.T) {
	a := transferFixture()
	id := a.state.Open(workspace.Rally, "Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	v.Start, v.Total, v.Page = 513, 4000, 22
	v.Items = make([]rally.Object, 256)
	v.ViewName, v.Sort, v.Descending = "My view", "PlanEstimate", true
	v.Filters, v.Rules, v.ExitAgreements, v.ShowFields, v.AIView = true, true, true, true, true
	v.LaneScroll = map[string]int{"Defined": 1200}
	v.CollapsedLanes = map[string]bool{"Completed": true}
	v.Detail = makeDetail(rally.Object{"Name": "Story", "Description": "<p>long formatted content</p>"}, "HierarchicalRequirement", false)
	r := v.Detail.Rich["Description"]
	r.ensureEditor()
	r.editor.Cursor, r.editor.SelectStart, r.editor.SelectEnd = 9, 2, 9
	r.editor.Scrollbar.X, r.editor.Scrollbar.Y = 12, 80
	r.mode = "Preview"
	name := v.Detail.Editors["Name"]
	name.Cursor, name.SelectStart, name.SelectEnd = 3, 1, 3
	v.Detail.CommentRich.ensureEditor()
	setText(v.Detail.CommentRich.editor, "comment draft")
	v.Detail.CommentRich.editor.Scrollbar.Y = 40
	a.rallyViews[id] = v
	saved := transferJSON(t, a.tabSnapshot(*a.state.Current()))
	b := transferFixture()
	b.installTransfer(saved)
	got := b.rallyViews[id]
	if got.Start != 513 || got.RestoreCount != 256 || got.Page != 22 || !got.Descending || got.Sort != v.Sort || got.ViewName != v.ViewName || !got.Filters || !got.Rules || !got.ExitAgreements || !got.ShowFields || !got.AIView || got.RestoreLaneScroll["Defined"] != 1200 || !got.CollapsedLanes["Completed"] {
		t.Fatal("board presentation lost")
	}
	restored := got.Detail.Rich["Description"]
	if restored.editor != nil {
		t.Fatal("transfer eagerly allocated native rich editor")
	}
	if restored.presentationState() != r.presentationState() {
		t.Fatalf("rich view lost: %+v", restored.presentationState())
	}
	restored.ensureEditor()
	if restored.presentationState() != r.presentationState() || position(got.Detail.Editors["Name"]) != position(name) {
		t.Fatal("materialization lost caret/scroll")
	}
	if got.Detail.CommentRich.html() != "<p>comment draft</p>" || got.Detail.CommentRich.presentationState().Position.ScrollY != 40 {
		t.Fatal("comment presentation lost")
	}
	// HTML source coordinates have an independent offset space.
	r.mode = "HTML"
	r.source = textEditor(r.doc.HTML(), true)
	r.synchronized()
	r.source.Cursor, r.source.SelectStart, r.source.SelectEnd = 12, 4, 12
	r.source.Scrollbar.Y = 160
	saved = transferJSON(t, a.tabSnapshot(*a.state.Current()))
	b.installTransfer(saved)
	restored = b.rallyViews[id].Detail.Rich["Description"]
	restored.ensureEditor()
	if restored.presentationState() != r.presentationState() {
		t.Fatal("source presentation lost")
	}
}

func TestV23TransferPreservesChatExpansionAndQueueEdit(t *testing.T) {
	a := transferFixture()
	c := &workspace.Conversation{ID: "chat", Title: "Chat", Draft: "unsent"}
	queued := c.Enqueue("queued", []string{"attachment"})
	c.EditQueue = queued
	c.QueueDraft = workspace.Draft{Text: "parked draft", Attachments: []string{"parked"}}
	a.state.Chats[c.ID] = c
	id := a.state.Open(workspace.Chat, c.Title, c.ID, "")
	v := newChatView()
	setText(v.Editor, "editing queue")
	v.Expanded["tool"] = true
	v.QueuePage = 3
	v.Scroll = 800
	v.ShowBlocks = 600
	v.Follow = false
	v.Editor.Cursor, v.Editor.SelectStart, v.Editor.SelectEnd = 7, 1, 7
	a.chats[c.ID] = v
	p := a.tabSnapshot(*a.state.Current())
	v.Expanded["tool"] = false
	b := transferFixture()
	b.installTransfer(transferJSON(t, p))
	got := b.chats[c.ID]
	if b.state.Active != id || !got.Expanded["tool"] || got.QueuePage != 3 || got.Scroll != 800 || got.ShowBlocks != 600 || got.Follow || position(got.Editor) != position(v.Editor) || b.state.Chats[c.ID].EditQueue != queued || b.state.Chats[c.ID].QueueDraft.Text != "parked draft" {
		t.Fatal("chat working context lost")
	}
}

func TestV23PresentationCheckpointReusesSavedRichContent(t *testing.T) {
	a := transferFixture()
	id := a.state.Open(workspace.Rally, "Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	a.rallyViews[id] = v
	v.Detail = makeDetail(rally.Object{"Name": "Story", "Description": "<p>original</p>"}, "HierarchicalRequirement", false)
	r := v.Detail.Rich["Description"]
	r.ensureEditor()
	setText(r.editor, strings.Repeat("content ", 10000))
	first := a.sessionSnapshot()
	saved := first.Documents[0].Rally.Detail.Rich["Description"]
	r.editor.Cursor, r.editor.SelectStart, r.editor.SelectEnd = 100, 20, 100
	r.editor.Scrollbar.Y = 900
	second := a.sessionSnapshot()
	next := second.Documents[0].Rally.Detail.Rich["Description"]
	if first.Documents[0].Rally == second.Documents[0].Rally || first.Documents[0].Rally.Detail.RichViews["Description"].Position.ScrollY == 900 || second.Documents[0].Rally.Detail.RichViews["Description"].Position.ScrollY != 900 {
		t.Fatal("presentation not immutable/current")
	}
	if &saved.Spans[0] != &next.Spans[0] {
		t.Fatal("caret-only movement copied rich content")
	}
}

func TestV23TransferReloadsResidentRange(t *testing.T) {
	request := make(chan string, 2)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		request <- r.URL.Query().Get("start") + ":" + r.URL.Query().Get("pagesize")
		fmt.Fprint(w, `{"QueryResult":{"StartIndex":513,"TotalResultCount":4000,"Results":[`)
		for i := range 256 {
			if i > 0 {
				fmt.Fprint(w, ",")
			}
			fmt.Fprintf(w, `{"_ref":"/slm/webservice/v2.0/hierarchicalrequirement/%d","Name":"Story"}`, 513+i)
		}
		fmt.Fprint(w, `]}}`)
	}))
	defer server.Close()
	client, err := rally.New(server.URL, "fixture", nil)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := transferFixture()
	a.ctx = ctx
	a.updates = make(chan func(), 16)
	a.installTransfer(tabTransfer{Tab: workspace.Tab{ID: "board", Kind: workspace.Rally, Page: "teamboard"}, Rally: &rallyTransfer{Mode: "board", Group: "None", ResidentStart: 513, ResidentCount: 256, Total: 4000}})
	v := a.rallyViews["board"]
	a.rallyClient = client
	v.Fields = []rally.Field{{Name: "Name"}}
	v.metadataSignature = v.signature
	a.refreshRallyItems(v)
	select {
	case got := <-request:
		if got != "513:256" {
			t.Fatal("wrong restored query", got)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("missing request")
	}
	drain(t, a, func() bool { return !v.Loading })
	if v.Start != 513 || len(v.Items) != 256 || v.RestoreCount != 0 {
		t.Fatalf("range lost: %d/%d/%d", v.Start, len(v.Items), v.RestoreCount)
	}
}
