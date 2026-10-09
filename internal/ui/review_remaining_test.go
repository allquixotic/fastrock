//go:build fltk_headless

package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestV5RefreshFullResidentWindow(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start, _ := strconv.Atoi(r.URL.Query().Get("start"))
		count, _ := strconv.Atoi(r.URL.Query().Get("pagesize"))
		count = min(count, 128) // A server may use a smaller page than requested.
		items := make([]rally.Object, count)
		for i := range items {
			items[i] = rally.Object{"ObjectID": start + i}
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": items, "StartIndex": start, "TotalResultCount": 10000}})
	}))
	defer server.Close()
	c, _ := rally.New(server.URL, "test", nil)
	page, err := loadRallyWindow(context.Background(), c, "HierarchicalRequirement", rally.Query{Start: 257}, 2048, true)
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Results) != 2048 || page.Results[2047].String("ObjectID") != "2304" {
		t.Fatalf("incomplete range: %d / %v", len(page.Results), page.Results[len(page.Results)-1])
	}
}

func TestV5RefreshResidentViewAndJitter(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start, _ := strconv.Atoi(r.URL.Query().Get("start"))
		count, _ := strconv.Atoi(r.URL.Query().Get("pagesize"))
		items := make([]rally.Object, min(count, 128))
		for i := range items {
			items[i] = rally.Object{"_ref": fmt.Sprintf("%shierarchicalrequirement/%d", rally.WSAPI, start+i), "ObjectID": start + i, "Name": "fresh"}
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": items, "StartIndex": start, "TotalResultCount": 10000}})
	}))
	defer server.Close()
	client, _ := rally.New(server.URL, "test", nil)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, updates: make(chan func(), 32), rallyClient: client}
	for _, restored := range []bool{false, true} {
		v := newRallyView(rally.FindPage("teamboard"))
		v.Mode, v.Start = "board", 257
		v.LaneScroll = map[string]int{"Defined": 850}
		v.Fields = []rally.Field{{Name: "Name"}}
		v.signature = a.rallySignature(v)
		v.metadataSignature = v.signature
		if restored {
			v.RestoreCount = 2048
		} else {
			v.Items = make([]rally.Object, 2048)
			v.Items[0] = rally.Object{"Name": "old"}
		}
		a.refreshRallyItems(v)
		if !restored && (len(v.Items) != 2048 || v.Items[0].String("Name") != "old") {
			t.Fatal("refresh discarded the displayed range before completion")
		}
		drain(t, a, func() bool { return !v.Loading })
		if v.Start != 257 || len(v.Items) != 2048 || v.Items[2047].String("ObjectID") != "2304" || v.LaneScroll["Defined"] != 850 || v.RestoreCount != 0 {
			t.Fatalf("resident range or position lost (restored=%v): start=%d len=%d", restored, v.Start, len(v.Items))
		}
		if delay := v.RefreshAt.Sub(v.Refreshed); delay < 55*time.Second || delay > 65*time.Second {
			t.Fatalf("refresh jitter out of range: %s", delay)
		}
	}
}

func TestV5RefreshFailureRetainsRange(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("start") != "1" {
			w.WriteHeader(400)
			return
		}
		fmt.Fprint(w, `{"QueryResult":{"Results":[{"Name":"new"}],"StartIndex":1,"TotalResultCount":2048}}`)
	}))
	defer server.Close()
	c, _ := rally.New(server.URL, "test", nil)
	a := &App{ctx: context.Background(), updates: make(chan func(), 32), rallyClient: c}
	v := newRallyView(rally.FindPage("teamboard"))
	v.Items = make([]rally.Object, 2048)
	v.Items[0] = rally.Object{"Name": "old"}
	v.Start = 1
	a.requestRallyPage(v, 1, true)
	drain(t, a, func() bool { return !v.Loading })
	if len(v.Items) != 2048 || v.Items[0].String("Name") != "old" || v.Error == "" {
		t.Fatal("failed refresh replaced old range")
	}
}

func TestV4DeltaInboxPreservesBarriers(t *testing.T) {
	var q eventInbox
	push := func(method, delta string, seq uint64) {
		raw, _ := json.Marshal(map[string]any{"threadId": "a", "itemId": "x", "delta": delta})
		m := codex.Message{Method: method, Params: raw, Sequence: seq}
		if !q.push(context.Background(), decodedEvent{message: m, params: codex.Decode(raw)}) {
			t.Fatal("push failed")
		}
	}
	push("item/agentMessage/delta", "one", 1)
	push("item/agentMessage/delta", "two", 2)
	push("item/completed", "", 3)
	push("item/agentMessage/delta", "three", 4)
	e := q.take()
	if str(e.params, "delta") != "onetwo" || e.message.Sequence != 2 || str(codex.Decode(e.message.Params), "delta") != "onetwo" {
		t.Fatal("coalesced event lost content or sequence")
	}
	if q.take().message.Method != "item/completed" {
		t.Fatal("completion moved across delta")
	}
	if str(q.take().params, "delta") != "three" || q.take() != nil {
		t.Fatal("unexpected tail")
	}
}

func TestV11UnreadBackgroundOnly(t *testing.T) {
	a := &App{state: workspace.NewState()}
	a.state.Chats["a"] = &workspace.Conversation{ID: "a"}
	a.state.Open(workspace.Chat, "a", "a", "")
	a.state.Chats["b"] = &workspace.Conversation{ID: "b"}
	a.eventDecoded(codex.Message{Method: "item/completed"}, map[string]any{"threadId": "b"})
	if !a.state.Chats["b"].Unread {
		t.Fatal("background completion lost")
	}
	a.eventDecoded(codex.Message{Method: "item/completed"}, map[string]any{"threadId": "a"})
	if a.state.Chats["a"].Unread {
		t.Fatal("active completion marked unread")
	}
}

func TestV11ContextMeterUsesLastTurnNotLifetime(t *testing.T) {
	a := &App{state: workspace.NewState()}
	c := &workspace.Conversation{ID: "a", Model: "model"}
	a.state.Chats[c.ID] = c
	a.eventDecoded(codex.Message{Method: "thread/tokenUsage/updated"}, map[string]any{"threadId": c.ID, "tokenUsage": map[string]any{"total": map[string]any{"totalTokens": float64(500000)}, "last": map[string]any{"totalTokens": float64(25000)}, "modelContextWindow": float64(100000)}})
	if got := contextLabel(c); got != "model · 75% context left" {
		t.Fatal(got)
	}
	if c.Tokens != 500000 {
		t.Fatal("lost lifetime total")
	}
}

func TestV4BoardSkipsOffscreenGroups(t *testing.T) {
	a := &App{p: colors(false)}
	v := newRallyView(rally.FindPage("teamboard"))
	v.Group = "Owner"
	for i := range 2048 {
		v.Items = append(v.Items, rally.Object{"_ref": fmt.Sprint(i), "Name": "Card", "Owner": fmt.Sprint(i), "ScheduleState": "Defined"})
	}
	v.prepareCards()
	v.prepareBoardLayout(v.Items)
	h := desktop.NewHeadlessHarness(desktop.WindowNoHScrollbar, image.Pt(1200, 800), func(w *desktop.Window) { a.drawTeamBoard(w, v, v.Items) })
	a.window = h.Master()
	a.window.SetStyle(makeStyle(a.p, 13))
	if n := h.Frame(true); n > 1000 {
		t.Fatalf("offscreen groups emitted %d drawing commands", n)
	}
}

func TestV11NewTabSignInOnlyWhenRequired(t *testing.T) {
	for _, test := range []struct {
		data map[string]any
		want bool
	}{
		{map[string]any{"requiresOpenaiAuth": true, "account": nil}, true},
		{map[string]any{"requiresOpenaiAuth": false, "account": nil}, false},
		{map[string]any{"requiresOpenaiAuth": true, "account": map[string]any{"type": "apiKey"}}, false},
	} {
		if accountNeedsLogin(test.data) != test.want {
			t.Fatal(test.data)
		}
	}
}
