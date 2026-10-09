package ui

import (
	"context"
	"fmt"
	"math/rand"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/workspace"
)

func waitSidebar(t testing.TB, a *App) {
	t.Helper()
	if a.updates == nil {
		a.updates = make(chan func(), 64)
	}
	if a.ctx == nil {
		ctx, cancel := context.WithCancel(context.Background())
		a.ctx = ctx
		t.Cleanup(cancel)
		t.Cleanup(a.stopSidebarPreparation)
	}
	deadline := time.Now().Add(10 * time.Second)
	for {
		a.sidebarFolders()
		if a.sidebarCache.ready {
			return
		}
		if a.sidebarCache.err != "" {
			t.Fatal(a.sidebarCache.err)
		}
		select {
		case update := <-a.updates:
			update()
		default:
			time.Sleep(100 * time.Microsecond)
		}
		if time.Now().After(deadline) {
			t.Fatal("sidebar preparation did not finish")
		}
	}
}

func TestV4SidebarIndexIsImmutableAndBalanced(t *testing.T) {
	var root *sidebarNode
	rows := make(map[string]*sidebarRow)
	rng := rand.New(rand.NewSource(17))
	for range 2000 {
		id := fmt.Sprint(rng.Intn(512))
		var row *sidebarRow
		if rng.Intn(3) != 0 {
			row = &sidebarRow{ID: id, Title: fmt.Sprint(rng.Intn(100))}
			rows[id] = row
		} else {
			delete(rows, id)
		}
		previous := root
		before, err := prepareSidebar(context.Background(), previous, "", false, nil)
		if err != nil {
			t.Fatal(err)
		}
		root = sidebarSet(root, id, row)
		after, _ := prepareSidebar(context.Background(), previous, "", false, nil)
		if before.count != after.count {
			t.Fatal("mutated captured root")
		}
		if len(rows) != sidebarNodeCount(t, root) {
			t.Fatal("lost index row")
		}
	}
}

func sidebarNodeCount(t *testing.T, n *sidebarNode) int {
	t.Helper()
	if n == nil {
		return 0
	}
	l, r := sidebarNodeCount(t, n.left), sidebarNodeCount(t, n.right)
	if delta := sidebarHeight(n.left) - sidebarHeight(n.right); delta < -1 || delta > 1 {
		t.Fatal("unbalanced index")
	}
	if n.height != max(sidebarHeight(n.left), sidebarHeight(n.right))+1 {
		t.Fatal("invalid height")
	}
	return l + r + 1
}

func TestV4SidebarKeepsOldRowsUntilNewestSearchPublishes(t *testing.T) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	a.state.Chats["a"] = &workspace.Conversation{ID: "a", Title: "First", Cwd: "/one", Updated: 1}
	a.state.Chats["b"] = &workspace.Conversation{ID: "b", Title: "Second", Cwd: "/two", Updated: 2}
	waitSidebar(t, a)
	if a.sidebarCache.count != 2 {
		t.Fatal("initial count")
	}
	setText(a.sidebarSearch, "first")
	a.sidebarFolders()
	if a.sidebarCache.count != 2 {
		t.Fatal("discarded previous list while worker runs")
	}
	setText(a.sidebarSearch, "second")
	a.sidebarFolders()
	waitSidebar(t, a)
	if a.sidebarCache.count != 1 || a.sidebarCache.folders[0].rows[0].ID != "b" {
		t.Fatal("stale search published")
	}
	// Only b's immutable path changes, while the running worker may still hold
	// an earlier root. Its result must not overwrite the renamed row.
	a.state.Chats["b"].Title = "Newest"
	a.invalidateSidebar("b")
	setText(a.sidebarSearch, "newest")
	waitSidebar(t, a)
	if a.sidebarCache.count != 1 || a.sidebarCache.folders[0].rows[0].Title != "Newest" {
		t.Fatal("metadata change not indexed")
	}
	delete(a.state.Chats, "b")
	a.invalidateSidebar("b")
	waitSidebar(t, a)
	if a.sidebarCache.count != 0 {
		t.Fatal("deleted row retained")
	}
}

func TestV4SidebarWorkerDoesNotReadMutableConversations(t *testing.T) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	for i := range 2000 {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: "original", Cwd: "/repo", Updated: int64(i)}
	}
	waitSidebar(t, a)
	root := a.sidebarCache.root
	done := make(chan sidebarPrepared, 1)
	go func() {
		result, _ := prepareSidebar(context.Background(), root, "original", false, nil)
		done <- result
	}()
	for _, c := range a.state.Chats {
		c.Title = "changed"
	}
	if result := <-done; result.count != 2000 || result.folders[0].rows[0].Title != "original" {
		t.Fatal("worker captured mutable metadata")
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := prepareSidebar(ctx, root, "", false, nil); err != context.Canceled {
		t.Fatal("cancel ignored", err)
	}
}

func TestV4SidebarRecentAndServerMatches(t *testing.T) {
	var root *sidebarNode
	for i := range 15 {
		id := fmt.Sprint(i)
		root = sidebarSet(root, id, &sidebarRow{ID: id, Title: "Title", search: "title", Updated: int64(i), Archived: i == 14})
	}
	result, err := prepareSidebar(context.Background(), root, "server-only", false, map[string]bool{"3": true, "14": true})
	if err != nil || result.count != 1 || result.folders[0].rows[0].ID != "3" {
		t.Fatal("server-confirmed match lost")
	}
	var ids []string
	for _, r := range result.recent {
		ids = append(ids, r.ID)
	}
	if !slices.Equal(ids, []string{"13", "12", "11", "10", "9", "8", "7", "6"}) {
		t.Fatal("incorrect recents", ids)
	}
}

func TestV21SidebarSearchResetDropsOldServerMatches(t *testing.T) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("server-only", false)}
	a.state.Chats["a"] = &workspace.Conversation{ID: "a", Title: "local title"}
	a.threadSearch = threadSearchState{query: "server-only", matches: map[string]bool{"a": true}}
	waitSidebar(t, a)
	if a.sidebarCache.count != 1 {
		t.Fatal("missing server result")
	}
	a.searchThreads("server-only", false, "")
	waitSidebar(t, a)
	if a.sidebarCache.count != 0 {
		t.Fatal("reset retained obsolete server-only result")
	}
}

func BenchmarkV4SidebarMetadataChange(b *testing.B) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	for i := range 100000 {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: "Original", Cwd: "/repo"}
	}
	waitSidebar(b, a)
	a.sidebarCache.working = true // measure UI admission without waiting for sort
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		a.state.Chats["1"].Title = strings.Repeat("x", 1+int(a.sidebarCache.generation%2))
		a.invalidateSidebar("1")
		a.sidebarFolders()
	}
}

func TestV21SidebarBootstrapAndMetadataWorkAreBounded(t *testing.T) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	for i := range 100000 {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: "Original", Cwd: "/repo"}
	}
	if a.updateSidebarIndex() || sidebarNodeCount(t, a.sidebarCache.scanningRoot) != 256 {
		t.Fatal("initial metadata admission exceeded one batch")
	}
	waitSidebar(t, a)
	old := a.sidebarCache.root
	a.state.Chats["1"].Title = "Changed"
	a.invalidateSidebar("1")
	if !a.updateSidebarIndex() {
		t.Fatal("single metadata update deferred")
	}
	var copied func(*sidebarNode, *sidebarNode) int
	copied = func(a, b *sidebarNode) int {
		if a == b {
			return 0
		}
		if a == nil || b == nil {
			t.Fatal("update changed tree topology")
		}
		return 1 + copied(a.left, b.left) + copied(a.right, b.right)
	}
	if n := copied(old, a.sidebarCache.root); n > sidebarHeight(old) {
		t.Fatalf("metadata update copied %d nodes, height %d", n, sidebarHeight(old))
	}
	// Captured roots retain the exact old values, not just the old row count.
	rows, _, _ := querySidebar(context.Background(), old, "original", false, nil)
	if len(rows) != 100000 {
		t.Fatal("metadata edit mutated a published snapshot")
	}
}

func TestV21ConversationPickerLatestQueryAndClose(t *testing.T) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	a.state.Chats["a"] = &workspace.Conversation{ID: "a", Title: "First"}
	a.state.Chats["b"] = &workspace.Conversation{ID: "b", Title: "Second"}
	waitSidebar(t, a)
	p := &conversationPicker{}
	a.prepareConversationPicker(p, "first")
	a.prepareConversationPicker(p, "second")
	deadline := time.Now().Add(10 * time.Second)
	for !p.ready {
		select {
		case f := <-a.updates:
			f()
		default:
			time.Sleep(time.Millisecond)
		}
		a.prepareConversationPicker(p, "second")
		if time.Now().After(deadline) {
			t.Fatal("picker did not settle")
		}
	}
	if len(p.rows) != 1 || p.rows[0].ID != "b" {
		t.Fatal("stale picker query")
	}
	a.prepareConversationPicker(p, "first")
	p.close()
	for p.working {
		select {
		case f := <-a.updates:
			f()
		default:
			time.Sleep(time.Millisecond)
		}
		if time.Now().After(deadline) {
			t.Fatal("closed picker worker did not settle")
		}
	}
	if p.rows != nil || p.root != nil {
		t.Fatal("closed picker resurrected results")
	}
}

func TestV21SidebarEmptyLoadingAndErrorStates(t *testing.T) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	waitSidebar(t, a)
	if !strings.Contains(a.sidebarEmptyMessage(), "Start one") {
		t.Fatal("missing initial empty state")
	}
	a.archived = true
	if a.sidebarEmptyMessage() != "No archived conversations" {
		t.Fatal("missing archived empty state")
	}
	setText(a.sidebarSearch, "absent")
	if a.sidebarEmptyMessage() != "No conversations match your search" {
		t.Fatal("missing search empty state")
	}
	page := a.threadPage(true)
	page.loading = true
	if a.sidebarEmptyMessage() != "" {
		t.Fatal("loading presented as empty")
	}
	page.loading, page.err = false, "load failed"
	if a.sidebarEmptyMessage() != "" {
		t.Fatal("failed load presented as empty")
	}
	page.err, a.sidebarCache.ready = "", false
	if a.sidebarEmptyMessage() != "" {
		t.Fatal("preparation presented as empty")
	}
}

func BenchmarkV21SidebarColdPreparation(b *testing.B) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	for i := range 100000 {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: "Conversation " + id, Cwd: "/repo", Updated: int64(i)}
	}
	waitSidebar(b, a)
	root := a.sidebarCache.root
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		if _, err := prepareSidebar(context.Background(), root, "conversation", false, nil); err != nil {
			b.Fatal(err)
		}
	}
}

func BenchmarkV21SidebarBootstrapBatch(b *testing.B) {
	a := &App{state: workspace.NewState(), sidebarSearch: textEditor("", false)}
	for i := range 100000 {
		id := fmt.Sprint(i)
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: "Conversation " + id, Cwd: "/repo"}
	}
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		if a.updateSidebarIndex() {
			a.sidebarCache = sidebarCache{}
		}
	}
}

func TestV21SidebarQueueFailureRequiresRetry(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	a := &App{ctx: ctx, state: workspace.NewState(), sidebarSearch: textEditor("", false), updates: make(chan func(), 8), readJobs: make(chan workTask, 1)}
	a.state.Chats["a"] = &workspace.Conversation{ID: "a", Title: "Title"}
	a.workOnce.Do(func() {}) // Hold a deterministic full admission queue.
	a.readJobs <- workTask{}
	a.sidebarFolders()
	(<-a.updates)()
	if a.sidebarCache.err == "" || a.sidebarCache.working {
		t.Fatal("queue rejection did not expose recoverable state")
	}
	a.sidebarFolders()
	if len(a.updates) != 0 {
		t.Fatal("rejected preparation retried during draw")
	}
	a.invalidateSidebarView()
	a.sidebarFolders()
	if len(a.updates) != 1 {
		t.Fatal("explicit retry did not readmit preparation")
	}
	(<-a.updates)()
}
