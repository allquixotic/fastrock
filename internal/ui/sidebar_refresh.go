package ui

import (
	"reflect"

	"github.com/allquixotic/fastrock/internal/workspace"
)

// A keyset refresh replaces only its covered head. Retained conversation
// objects still own drafts, queues and open tabs when absent from the server.
// The mutable map is scanned only on the UI owner, in bounded frame batches.
type threadRefresh struct {
	scan                *reflect.MapIter
	seen                map[string]bool
	revision            uint64
	oldest              int64
	next, oldCursor     string
	keepTail, wasLoaded bool
}

func (a *App) applyThreadPage(archived bool, cursor, next string, rows []map[string]any, page *threadPageState, revision uint64) {
	if a.historyCursor == nil {
		a.historyCursor = map[bool]string{}
	}
	seen := make(map[string]bool, len(rows))
	var oldest int64
	for _, row := range rows {
		id := str(row, "id")
		if id == "" {
			continue
		}
		updated := integer(row, "updatedAt")
		if len(seen) == 0 || updated < oldest {
			oldest = updated
		}
		seen[id] = true
		c := a.state.Chats[id]
		if c == nil {
			c = &workspace.Conversation{ID: id, Status: "idle"}
			a.state.Chats[id] = c
		}
		// A rename, archive or live update after this request wins over its
		// older response. Observation must not replace conversation content.
		if c.SidebarRevision > revision {
			continue
		}
		c.Title, c.Cwd, c.Updated, c.Archived = threadTitle(row), str(row, "cwd"), updated, archived
		a.invalidateSidebar(id)
	}
	if cursor != "" {
		a.historyCursor[archived] = next
		page.loading, page.loaded = false, true
		return
	}
	page.refresh = &threadRefresh{
		scan: reflect.ValueOf(a.state.Chats).MapRange(), seen: seen, revision: revision,
		oldest: oldest, next: next, oldCursor: a.historyCursor[archived], wasLoaded: page.loaded,
	}
	a.advanceThreadRefresh(archived, page)
}

func (a *App) advanceThreadRefreshes() {
	for archived, page := range a.historyPages {
		if page.refresh != nil {
			a.advanceThreadRefresh(archived, page)
		}
	}
}

func (a *App) advanceThreadRefresh(archived bool, page *threadPageState) {
	r := page.refresh
	if r == nil {
		return
	}
	if page.client != a.client {
		page.refresh, page.loading = nil, false
		return
	}
	for range 256 {
		if !r.scan.Next() {
			a.historyCursor[archived] = r.next
			if r.wasLoaded && r.keepTail && r.next != "" {
				a.historyCursor[archived] = r.oldCursor
			}
			page.refresh, page.loading, page.loaded = nil, false, true
			return
		}
		c, _ := r.scan.Value().Interface().(*workspace.Conversation)
		if c == nil || c.Archived != archived || c.SidebarHidden || r.seen[c.ID] {
			continue
		}
		// Equal timestamps at the page boundary may straddle the keyset;
		// retain them until a response covers the entire list.
		if r.next != "" && c.Updated <= r.oldest && len(r.seen) != 0 {
			r.keepTail = true
			continue
		}
		if c.SidebarRevision > r.revision {
			continue
		}
		c.SidebarHidden = true
		a.markSidebar(false, c.ID)
	}
	if a.window != nil {
		a.window.Changed()
	}
}
