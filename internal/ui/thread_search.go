package ui

import (
	"context"
	"maps"
	"time"

	"github.com/allquixotic/fastrock/internal/workspace"
)

type threadSearchState struct {
	query, cursor, err string
	archived, loading  bool
	generation         uint64
	cancel             context.CancelFunc
	matches            map[string]bool
}

func (a *App) searchThreads(query string, archived bool, cursor string) {
	s := &a.threadSearch
	if s.cancel != nil {
		s.cancel()
	}
	s.generation++
	generation := s.generation
	s.query, s.archived, s.err = query, archived, ""
	if cursor == "" {
		s.matches = map[string]bool{}
		s.cursor = ""
		a.invalidateSidebarView()
	}
	if query == "" || a.client == nil {
		s.loading = false
		return
	}
	ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
	s.cancel = cancel
	s.loading = true
	client := a.client
	a.work(func() {
		defer cancel()
		if cursor == "" {
			timer := time.NewTimer(250 * time.Millisecond)
			defer timer.Stop()
			select {
			case <-ctx.Done():
				return
			case <-timer.C:
			}
		}
		var page struct {
			Data []struct {
				Thread  map[string]any
				Snippet string
			}
			Next string `json:"nextCursor"`
		}
		params := map[string]any{"searchTerm": query, "archived": archived, "limit": 100, "sortKey": "updated_at", "sortDirection": "desc"}
		if cursor != "" {
			params["cursor"] = cursor
		}
		err := client.Call(ctx, "thread/search", params, &page)
		a.post(func() {
			if generation != s.generation || client != a.client {
				return
			}
			s.loading = false
			if err != nil {
				s.err = err.Error()
				return
			}
			s.cursor = page.Next
			if page.Next == cursor {
				s.cursor = ""
			}
			// Previous maps may be captured by a sidebar worker.
			s.matches = maps.Clone(s.matches)
			var changed []string
			for _, row := range page.Data {
				if len(s.matches) >= 5000 {
					s.cursor = ""
					s.err = "Showing the first 5,000 matching conversations; refine the search to see others."
					break
				}
				t := row.Thread
				id := str(t, "id")
				if id == "" {
					continue
				}
				s.matches[id] = true
				changed = append(changed, id)
				if a.state.Chats[id] == nil {
					a.state.Chats[id] = &workspace.Conversation{ID: id, Title: threadTitle(t), Cwd: str(t, "cwd"), Updated: integer(t, "updatedAt"), Archived: archived, Status: "idle"}
				}
			}
			if len(changed) > 0 {
				a.invalidateSidebar(changed...)
			}
		})
	}, func() { s.loading = false; s.err = errWorkQueueFull.Error(); cancel() })
}
