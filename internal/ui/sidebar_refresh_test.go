//go:build fltk_headless

package ui

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"testing"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func refreshRow(id string, updated int64) map[string]any {
	return map[string]any{"id": id, "name": "Server " + id, "updatedAt": float64(updated)}
}

func TestV51SidebarRefreshReplacesHeadPreservesTailAndDrafts(t *testing.T) {
	a := &App{state: workspace.NewState(), ctx: context.Background(), historyCursor: map[bool]string{false: "old-next"}}
	for id, updated := range map[string]int64{"gone": 450, "same": 350, "boundary": 310, "older": 100, "archived": 450} {
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: id, Updated: updated, Archived: id == "archived"}
	}
	draft := a.state.Chats["gone"]
	draft.Draft = "unsent draft"
	draft.DraftAttachments = []string{"picture.png"}
	draft.Enqueue("queued", nil)
	a.state.Open(workspace.Chat, "Gone", "gone", "")
	page := a.threadPage(false)
	page.loaded, page.loading = true, true
	a.applyThreadPage(false, "", "new-next", []map[string]any{refreshRow("same", 350), refreshRow("new", 310)}, page, a.sidebarCache.generation)
	if page.loading || a.historyCursor[false] != "old-next" || !draft.SidebarHidden {
		t.Fatalf("head not reconciled, or tail cursor lost: %+v, %q, hidden=%v", page, a.historyCursor[false], draft.SidebarHidden)
	}
	if a.state.Chats["boundary"].SidebarHidden || a.state.Chats["older"].SidebarHidden || a.state.Chats["archived"].SidebarHidden {
		t.Fatal("uncovered tail, equal boundary, or other archive mode removed")
	}
	if a.state.Chats["gone"] != draft || draft.Draft != "unsent draft" || len(draft.Queue) != 1 || len(draft.DraftAttachments) != 1 || a.state.Current().Target != "gone" {
		t.Fatal("refresh changed an owned conversation or open tab")
	}
	for !a.updateSidebarIndex() {
	}
	rows, recent, err := querySidebar(a.ctx, a.sidebarCache.root, "", false, nil)
	if err != nil || len(rows) != 4 || len(recent) != 4 || len(a.state.Sidebar("", false)) != 4 {
		t.Fatalf("hidden row remains indexed: %d / %d / %v", len(rows), len(recent), err)
	}
	// Exhaustion is authoritative even for the older cached tail. Reappearing
	// IDs become visible without replacing their retained local conversation.
	page.loading = true
	a.applyThreadPage(false, "", "", []map[string]any{refreshRow("gone", 500)}, page, a.sidebarCache.generation)
	if draft.SidebarHidden || !a.state.Chats["older"].SidebarHidden || !a.state.Chats["same"].SidebarHidden || a.historyCursor[false] != "" {
		t.Fatal("complete refresh did not replace the list or restore a returned row")
	}
	if draft.Draft != "unsent draft" || draft.Title != "Server gone" {
		t.Fatal("server metadata restoration overwrote local draft")
	}
	page.loading = true
	a.applyThreadPage(false, "", "", nil, page, a.sidebarCache.generation)
	if !draft.SidebarHidden || a.state.Chats["archived"].SidebarHidden {
		t.Fatal("empty successful refresh was not scoped to its archive mode")
	}
}

func TestV51SidebarRefreshProtectsNewerUpdatesAndBoundsScanning(t *testing.T) {
	a := &App{state: workspace.NewState(), ctx: context.Background()}
	for i := range 1200 {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: id, Updated: 100}
	}
	revision := a.sidebarCache.generation
	a.state.Chats["0"].Title = "Newer local rename"
	a.invalidateSidebar("0")
	a.state.Chats["1"].Archived = true
	a.invalidateSidebar("1")
	page := a.threadPage(false)
	page.loading = true
	a.applyThreadPage(false, "", "", []map[string]any{refreshRow("0", 50), refreshRow("1", 50)}, page, revision)
	if !page.loading || page.refresh == nil {
		t.Fatal("large history was scanned synchronously")
	}
	hidden := 0
	for _, c := range a.state.Chats {
		if c.SidebarHidden {
			hidden++
		}
	}
	if hidden > 256 {
		t.Fatalf("one frame hid %d rows", hidden)
	}
	for i := 0; page.loading && i < 10; i++ {
		a.advanceThreadRefreshes()
	}
	if page.loading || a.state.Chats["0"].Title != "Newer local rename" || a.state.Chats["0"].SidebarHidden || !a.state.Chats["1"].Archived {
		t.Fatal("stale listing replaced newer live metadata")
	}
	if !a.state.Chats["500"].SidebarHidden {
		t.Fatal("batched scan did not finish")
	}
	// A later event proves the row exists again and exposes it to selectors.
	a.invalidateSidebar("500")
	if a.state.Chats["500"].SidebarHidden {
		t.Fatal("new live metadata did not reveal a hidden row")
	}
}

func TestV51SidebarRefreshFailureKeepsList(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client, err := codex.StartCommand(ctx, exe, []string{"-test.run=^TestV51SidebarServerFixture$", "--", "sidebar-server-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	a := &App{ctx: ctx, client: client, state: workspace.NewState(), updates: make(chan func(), 128)}
	a.requestThreads(false, "")
	drain(t, a, func() bool { return !a.threadPage(false).loading })
	recent := a.state.Chats["recent"]
	recent.Draft = "keep"
	a.requestThreads(false, "")
	drain(t, a, func() bool { return !a.threadPage(false).loading })
	if recent.SidebarHidden || recent.Draft != "keep" || a.historyCursor[false] != "next" || a.threadPage(false).err == "" || a.threadPage(false).cursor != "" {
		t.Fatal("failed response changed sidebar data or retry cursor")
	}
	a.requestThreads(false, a.threadPage(false).cursor)
	drain(t, a, func() bool { return !a.threadPage(false).loading })
	if !recent.SidebarHidden || recent.Draft != "keep" || a.state.Chats["fresh"] == nil || a.threadPage(false).err != "" {
		t.Fatal("successful retry did not reconcile the head and retain the draft")
	}
}

func TestV51SidebarServerFixture(t *testing.T) {
	if os.Args[len(os.Args)-1] != "sidebar-server-fixture" {
		return
	}
	scanner, writer := bufio.NewScanner(os.Stdin), json.NewEncoder(os.Stdout)
	calls := 0
	for scanner.Scan() {
		var m codex.Message
		if json.Unmarshal(scanner.Bytes(), &m) != nil || m.Method == "" {
			continue
		}
		var result any = map[string]any{}
		var rpcError *codex.RPCError
		if m.Method == "thread/list" {
			calls++
			switch calls {
			case 1:
				result = map[string]any{"data": []any{refreshRow("recent", 100)}, "nextCursor": "next"}
			case 2:
				rpcError = &codex.RPCError{Code: -32000, Message: "refresh unavailable"}
			default:
				result = map[string]any{"data": []any{refreshRow("fresh", 200)}, "nextCursor": nil}
			}
		}
		encoded, _ := json.Marshal(result)
		_ = writer.Encode(codex.Message{ID: m.ID, Result: encoded, Error: rpcError})
	}
	os.Exit(0)
}
