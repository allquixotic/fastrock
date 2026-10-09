package ui

import (
	"path/filepath"

	"github.com/aarzilli/nucular"

	"github.com/allquixotic/fastrock/internal/workspace"
)

type sidebarFolder struct {
	path, title string
	rows        []*workspace.Conversation
}
type sidebarCache struct {
	signature uint64
	query     string
	archived  bool
	ready     bool
	folders   []sidebarFolder
	count     int
}

func (a *App) invalidateSidebar() { a.sidebarCache.ready = false }

func (a *App) sidebarFolders() []sidebarFolder {
	query := text(a.sidebarSearch)
	signature := uint64(len(a.state.Chats))
	c := &a.sidebarCache
	if c.ready && c.signature == signature && c.query == query && c.archived == a.archived {
		return c.folders
	}
	c.ready, c.signature, c.query, c.archived = true, signature, query, a.archived
	clear(c.folders)
	c.folders = c.folders[:0]
	c.count = 0
	rows := a.state.Sidebar(query, a.archived)
	byPath := make(map[string]int)
	for _, row := range rows {
		i, exists := byPath[row.Cwd]
		if !exists {
			c.folders = append(c.folders, sidebarFolder{path: row.Cwd, title: filepath.Base(row.Cwd)})
			i = len(c.folders) - 1
			byPath[row.Cwd] = i
		}
		c.folders[i].rows = append(c.folders[i].rows, row)
		c.count++
	}
	return c.folders
}

// Skip whole offscreen runs in constant time; include a row of overscan. Heights
// include nucular row spacing, and do not depend on the number of conversations.
func sidebarVisible(top, clipTop, clipBottom, stride, count int) (int, int) {
	first := min(count, max(0, (clipTop-top)/stride-1))
	last := min(count, max(first, (clipBottom-top)/stride+2))
	return first, last
}
func sidebarSkip(w *nucular.Window, count, stride, spacing int) {
	if count > 0 {
		w.RowScaled(count*stride - spacing).Dynamic(1)
		w.Spacing(1)
	}
}
