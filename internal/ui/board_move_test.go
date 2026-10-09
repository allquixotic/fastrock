//go:build fltk_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/mouse"
)

type boardServer struct {
	mu      sync.Mutex
	items   map[string]rally.Object
	writes  []rally.Object
	ranks   []map[string]string
	queries int
	hold    chan struct{}
	started chan struct{}
	fail    bool
}

func boardMutationFixture(t *testing.T) (*App, *rallyView, *boardServer) {
	t.Helper()
	a, v := keyboardBoard(t, 3)
	v.Group = "Owner"
	v.Workflow = []rally.Object{{"Name": "Defined"}, {"Name": "Accepted"}}
	v.Fields = []rally.Field{{Name: "Owner", AttributeType: "OBJECT"}, {Name: "ScheduleState", AttributeType: "STRING"}}
	for i, o := range v.Items {
		o["_ref"] = fmt.Sprintf("/slm/webservice/v2.0/hierarchicalrequirement/%d", i+1)
		o["ObjectID"] = i + 1
		o["VersionId"] = "1"
		o["DragAndDropRank"] = []string{"C", "B", "D"}[i]
		owner := "b"
		if i == 0 {
			owner = "a"
		} else {
			o["ScheduleState"] = "Accepted"
		}
		o["Owner"] = map[string]any{"_ref": "/slm/webservice/v2.0/user/" + owner, "_refObjectName": "Owner " + owner}
	}
	state := &boardServer{items: map[string]rally.Object{}}
	for _, o := range v.Items {
		state.items[o.String("_ref")] = o.Clone()
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		state.mu.Lock()
		if r.Method == "POST" {
			var body map[string]rally.Object
			err := json.NewDecoder(r.Body).Decode(&body)
			if err != nil {
				state.mu.Unlock()
				t.Error(err)
				w.WriteHeader(400)
				return
			}
			fields := body["HierarchicalRequirement"]
			state.writes = append(state.writes, fields.Clone())
			state.ranks = append(state.ranks, map[string]string{"above": r.URL.Query().Get("rankAbove"), "below": r.URL.Query().Get("rankBelow")})
			hold, started := state.hold, state.started
			state.hold, state.started = nil, nil
			state.mu.Unlock()
			if started != nil {
				close(started)
			}
			if hold != nil {
				select {
				case <-hold:
				case <-r.Context().Done():
					return
				}
			}
			state.mu.Lock()
			if state.fail {
				state.mu.Unlock()
				w.WriteHeader(409)
				fmt.Fprint(w, `{"OperationResult":{"Errors":["fixture rejected"]}}`)
				return
			}
			o := state.items[r.URL.Path]
			for key, value := range fields {
				if key == "Owner" && value != nil {
					ref, _ := value.(string)
					value = map[string]any{"_ref": ref, "_refObjectName": "Owner " + ref[len(ref)-1:]}
				}
				o[key] = value
			}
			o["VersionId"] = fmt.Sprint(len(state.writes) + 1)
			if target := r.URL.Query().Get("rankAbove"); target != "" {
				o["DragAndDropRank"] = "A"
				if strings.HasSuffix(target, "/3") {
					o["DragAndDropRank"] = "C"
				}
			}
			if r.URL.Query().Get("rankBelow") != "" {
				o["DragAndDropRank"] = "Z"
			}
			result := o.Clone()
			state.mu.Unlock()
			json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": result}})
			return
		}
		if o := state.items[r.URL.Path]; o != nil {
			result := o.Clone()
			state.mu.Unlock()
			json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": result})
			return
		}
		state.queries++
		items := make([]rally.Object, 0, len(state.items))
		for _, o := range state.items {
			items = append(items, o.Clone())
		}
		state.mu.Unlock()
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": items, "TotalResultCount": len(items), "StartIndex": 1}})
	}))
	t.Cleanup(server.Close)
	a.rallyClient, _ = rally.New(server.URL, "fixture", nil)
	v.signature = a.rallySignature(v)
	v.metadataSignature = v.signature
	return a, v, state
}
func boardDestination(v *rallyView, o rally.Object) boardGroup {
	key, name, value := boardGroupIdentity(o, v.Group)
	return boardGroup{key: key, name: name, value: value}
}
func boardItem(v *rallyView, ref string) rally.Object {
	for _, o := range v.Items {
		if o.String("_ref") == ref {
			return o
		}
	}
	return nil
}

func TestV45BoardMoveCommitUndoAndConflict(t *testing.T) {
	a, v, server := boardMutationFixture(t)
	v.prepareCards()
	source, target := v.Items[0], v.Items[1]
	original := slices.Clone(v.Items)
	v.selectItem(source, true)
	server.hold, server.started = make(chan struct{}), make(chan struct{})
	release, started := server.hold, server.started
	a.dropBoardCard(v, v.cards[source.String("_ref")], boardDrop{State: "Accepted", Group: boardDestination(v, target), Target: target.String("_ref")})
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("move did not start")
	}
	if !v.PendingCards[source.String("_ref")] || boardItem(v, source.String("_ref")).String("ScheduleState") != "Accepted" || boardItem(v, source.String("_ref")).Ref("Owner") != target.Ref("Owner") || original[0].String("ScheduleState") != "Defined" {
		t.Fatal("optimistic mutation or immutable baseline failed")
	}
	v.prepareBoardLayout(v.filtered())
	var order []string
	for _, g := range v.boardGroups {
		for _, lane := range g.lanes {
			for _, c := range lane.cards {
				order = append(order, c.ref)
			}
		}
	}
	if slices.Index(order, source.String("_ref")) >= slices.Index(order, target.String("_ref")) {
		t.Fatal("optimistic rank did not apply", order)
	}
	a.dropBoardCard(v, v.cards[source.String("_ref")], boardDrop{State: "Defined", Group: boardDestination(v, source)})
	tab := *a.state.Current()
	a.closeTab(tab.ID)
	a.popOut(tab)
	if a.state.Current() == nil || a.beginWindowClose() {
		t.Fatal("pending board write lost its owner")
	}
	close(release)
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	if boardItem(v, source.String("_ref")).String("VersionId") != "2" || v.SelectedItems[source.String("_ref")].String("VersionId") != "2" || len(a.notices) != 1 || a.notices[0].Action == nil {
		t.Fatal("acknowledgment or Undo is missing")
	}
	server.mu.Lock()
	queries, writes := server.queries, len(server.writes)
	server.mu.Unlock()
	if queries != 0 || writes != 1 {
		t.Fatal("move reloaded the board or repeated the write", queries, writes)
	}
	undo := a.notices[0].Action
	undo()
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	got := boardItem(v, source.String("_ref"))
	server.mu.Lock()
	fields := server.writes[1]
	rank := server.ranks[1]
	server.mu.Unlock()
	if got.String("ScheduleState") != "Defined" || got.Ref("Owner") != source.Ref("Owner") || fields.Ref("Owner") != source.Ref("Owner") || rank["above"] != v.Items[2].String("_ref") {
		t.Fatal("Undo failed to restore fields and original relative rank", got, fields, rank)
	}
	undo()
	server.mu.Lock()
	writes = len(server.writes)
	server.mu.Unlock()
	if writes != 2 {
		t.Fatal("old Undo repeated a write")
	}
	// A fresh move succeeds; a remote edit before Undo must reject the reversal.
	v.prepareCards()
	a.dropBoardCard(v, v.cards[source.String("_ref")], boardDrop{State: "Accepted", Group: boardDestination(v, target)})
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	u := v.boardUndo
	server.mu.Lock()
	server.items[source.String("_ref")]["VersionId"] = "99"
	server.mu.Unlock()
	a.undoBoardWrite(v, u)
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	if boardItem(v, source.String("_ref")).String("ScheduleState") != "Accepted" || !strings.Contains(a.toast, "changed on Rally") {
		t.Fatal("conflicting Undo overwrote or hid the current item", a.toast)
	}
	server.mu.Lock()
	writes = len(server.writes)
	server.mu.Unlock()
	if writes != 3 {
		t.Fatal("conflicting Undo issued a POST", writes)
	}
}

func TestV45BoardMoveRollbackAndRefresh(t *testing.T) {
	a, v, server := boardMutationFixture(t)
	v.prepareCards()
	before := v.Items[0].Clone()
	server.fail = true
	server.hold, server.started = make(chan struct{}), make(chan struct{})
	release, started := server.hold, server.started
	a.dropBoardCard(v, v.cards[before.String("_ref")], boardDrop{State: "Accepted", Group: boardDestination(v, v.Items[1])})
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("move did not start")
	}
	a.refreshRallyItems(v)
	if !v.boardRefreshQueued || v.Loading {
		t.Fatal("refresh ran over a pending optimistic state")
	}
	close(release)
	drain(t, a, func() bool { return len(v.PendingCards) == 0 && !v.Loading })
	if boardItem(v, before.String("_ref")).String("ScheduleState") != "Defined" || v.boardUndo != nil || !strings.Contains(a.toast, "restored locally") {
		t.Fatal("failed move was not restored", a.toast)
	}
	server.mu.Lock()
	queries := server.queries
	server.mu.Unlock()
	if queries != 1 {
		t.Fatal("deferred refresh did not run once", queries)
	}
	// Rejected admission must unwind immediately, without leaving Saving forever.
	a.cancel()
	v.prepareCards()
	a.dropBoardCard(v, v.cards[before.String("_ref")], boardDrop{State: "Accepted", Group: boardDestination(v, v.Items[1])})
	if len(v.PendingCards) != 0 || boardItem(v, before.String("_ref")).String("ScheduleState") != "Defined" {
		t.Fatal("cancelled write left optimistic state")
	}
}

func TestV45BoardGroupIdentityAndRankOrder(t *testing.T) {
	a, v, _ := boardMutationFixture(t)
	for _, o := range v.Items {
		o["Owner"].(map[string]any)["_refObjectName"] = "Same Name"
	}
	v.prepareCards()
	v.prepareBoardLayout(v.filtered())
	if len(v.boardGroups) != 2 || v.boardGroups[0].key == v.boardGroups[1].key {
		t.Fatal("duplicate owner names collapsed separate groups")
	}
	field := v.Fields[0]
	v.Fields[0].ReadOnly = true
	if _, _, err := boardGroupChange(v, v.boardGroups[1]); err == nil {
		t.Fatal("read-only group accepted")
	}
	v.Fields[0] = field
	if a.rallyQuery(v).Order != "Owner ASC,DragAndDropRank ASC" || slices.Contains(strings.Split(rallyFetch(v), ","), "Rank") || !slices.Contains(strings.Split(rallyFetch(v), ","), "DragAndDropRank") {
		t.Fatal("invented Rank wire field remains", a.rallyQuery(v), rallyFetch(v))
	}
	if compareRally(rally.Object{"DragAndDropRank": "a"}, rally.Object{"DragAndDropRank": "B"}, "Rank", nil) <= 0 {
		t.Fatal("opaque rank was lowercased")
	}
}

func TestV45BoardDropAndUndoNotice(t *testing.T) {
	a, v, server := boardMutationFixture(t)
	var pos, origin image.Point
	var down, clicked bool
	var commands []command.Command
	h := desktop.NewHeadlessHarness(0, image.Pt(1200, 1400), func(w *desktop.Window) {
		in := &w.Input().Mouse
		in.Pos = pos
		in.Buttons[mouse.ButtonLeft].Down = down
		in.Buttons[mouse.ButtonLeft].Clicked = clicked
		in.Buttons[mouse.ButtonLeft].ClickedPos = origin
		a.drawTeamBoard(w, v, v.filtered())
		commands = append(commands[:0], w.Commands().Commands...)
	})
	a.window = h.Master()
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	centers := map[string]image.Point{}
	for _, c := range commands {
		if c.Kind == command.TextCmd {
			centers[c.Text.String] = image.Pt(c.Rect.X+5, c.Rect.Y+5)
		}
	}
	origin, pos = centers["US0000"], centers["US0000"]
	if origin.X == 0 || centers["US0001"].X == 0 {
		t.Fatal("missing card geometry", centers)
	}
	down, clicked = true, true
	h.Frame(false)
	pos = centers["US0001"]
	clicked = false
	h.Frame(false)
	down, clicked = false, true
	h.Frame(false)
	if !v.PendingCards[v.Items[0].String("_ref")] {
		t.Fatal("actual cross-swimlane drop did not queue a mutation")
	}
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	server.mu.Lock()
	fields := server.writes[0]
	rank := server.ranks[0]
	server.mu.Unlock()
	if fields.String("ScheduleState") != "Accepted" || fields.Ref("Owner") != v.Items[1].Ref("Owner") || rank["above"] != v.Items[1].String("_ref") {
		t.Fatal("drop lost its group/card target", fields, rank)
	}
	// Exercise the overlay path separately; its click must not reach content.
	count := 0
	n := transferFixture()
	n.p, n.prefs.FontSize = colors(false), 13
	var button rect.Rect
	click := false
	h = desktop.NewHeadlessHarness(0, image.Pt(600, 400), func(w *desktop.Window) {
		layouts := n.noticeLayouts(w)
		if len(layouts) > 0 {
			button = layouts[0].button
		}
		m := &w.Input().Mouse
		m.Pos = image.Pt(button.X+5, button.Y+5)
		m.Buttons[mouse.ButtonLeft].Clicked = click
		m.Buttons[mouse.ButtonLeft].Down = false
		m.Buttons[mouse.ButtonLeft].ClickedPos = m.Pos
		n.handleNoticeInput(w)
		if click && m.Buttons[mouse.ButtonLeft].Clicked {
			t.Error("Undo clicked through the overlay")
		}
		n.drawNotices(w)
	})
	n.window = h.Master()
	n.actionNotice("Moved item", "Undo", func() { count++ })
	h.Frame(false)
	click = true
	h.Frame(false)
	h.Frame(false)
	if count != 1 {
		t.Fatal("Undo was not single-use", count)
	}
}

func TestV45PendingMarkerRemainsVisibleOnBlockedCards(t *testing.T) {
	a, v := keyboardBoard(t, 1)
	v.Items[0]["Blocked"] = true
	v.prepareCards()
	c := v.cards[v.Items[0].String("_ref")]
	v.PendingCards = map[string]bool{c.ref: true}
	var labels []string
	h := desktop.NewHeadlessHarness(0, image.Pt(500, 300), func(w *desktop.Window) {
		w.Row(196).Dynamic(1)
		a.drawBoardCard(w, v, c)
		for _, cmd := range w.Commands().Commands {
			if cmd.Kind == command.TextCmd {
				labels = append(labels, cmd.Text.String)
			}
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	if len(labels) == 0 || labels[len(labels)-1] != "Saving…" {
		t.Fatal("pending marker was hidden behind the blocked banner", labels)
	}
}
