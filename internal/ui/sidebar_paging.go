package ui

import (
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
)

type threadPageState struct {
	loading bool
	err     string
	cursor  string
	loaded  bool
	request uint64
	client  *codex.Client
	refresh *threadRefresh
}

func (a *App) threadPage(archived bool) *threadPageState {
	if a.historyPages == nil {
		a.historyPages = map[bool]*threadPageState{}
	}
	if a.historyPages[archived] == nil {
		a.historyPages[archived] = &threadPageState{}
	}
	return a.historyPages[archived]
}

// All admission and completion state is owned by the UI, including the first
// page. A failed page retains its cursor and needs an explicit retry.
func (a *App) requestThreads(archived bool, cursor string) {
	page, client := a.threadPage(archived), a.client
	if page.client != client {
		page.loading, page.loaded, page.refresh = false, false, nil
	}
	if page.loading || client == nil {
		return
	}
	page.loading, page.err, page.client, page.cursor = true, "", client, cursor
	page.request++
	request, revision := page.request, a.sidebarCache.generation
	a.work(func() { a.fetchThreads(client, archived, cursor, page, request, revision) }, func() {
		if page.request == request {
			page.loading, page.err = false, errWorkQueueFull.Error()
		}
	})
}

func (a *App) sidebarPaging(w *desktop.Window) {
	nearEnd := w.LayoutNextRowY() <= w.Bounds.Y+w.Bounds.H+int(120*w.Master().Style().Scaling)
	if text(a.sidebarSearch) != "" {
		s := &a.threadSearch
		if s.loading {
			muted(w, "Loading more matching conversations…", a.p)
		} else if s.cursor != "" {
			w.Row(28).Dynamic(1)
			clicked := w.ButtonText("Load more matching conversations")
			if clicked || nearEnd && s.err == "" {
				a.searchThreads(s.query, a.archived, s.cursor)
			}
		}
		return
	}
	page := a.threadPage(a.archived)
	if page.loading {
		muted(w, "Loading conversations…", a.p)
		return
	}
	if page.err != "" {
		muted(w, "Could not load conversations: "+page.err, a.p)
		w.Row(28).Dynamic(1)
		if w.ButtonText("Retry loading conversations") {
			a.requestThreads(a.archived, page.cursor)
		}
		return
	}
	if cursor := a.historyCursor[a.archived]; cursor != "" {
		w.Row(28).Dynamic(1)
		clicked := w.ButtonText("Load more conversations")
		if clicked || nearEnd {
			a.requestThreads(a.archived, cursor)
		}
	}
}
