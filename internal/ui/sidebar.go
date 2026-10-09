package ui

import (
	"path/filepath"
	"slices"

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

func sidebarHash(s string) uint64 {
	h := uint64(14695981039346656037)
	for i := 0; i < len(s); i++ {
		h ^= uint64(s[i])
		h *= 1099511628211
	}
	return h
}

func (a *App) sidebarFolders() []sidebarFolder {
	query := text(a.sidebarSearch)
	signature := uint64(len(a.state.Chats))
	for _, c := range a.state.Chats {
		h := sidebarHash(c.ID) ^ sidebarHash(c.Title)<<1 ^ sidebarHash(c.Cwd)<<2 ^ uint64(c.Updated)
		if c.Archived {
			h ^= 1 << 63
		}
		signature ^= h
	}
	c := &a.sidebarCache
	if c.ready && c.signature == signature && c.query == query && c.archived == a.archived {
		return c.folders
	}
	c.ready, c.signature, c.query, c.archived = true, signature, query, a.archived
	c.folders = c.folders[:0]
	c.count = 0
	rows := a.state.Sidebar(query, a.archived)
	for _, row := range rows {
		i := slices.IndexFunc(c.folders, func(f sidebarFolder) bool { return f.path == row.Cwd })
		if i < 0 {
			c.folders = append(c.folders, sidebarFolder{path: row.Cwd, title: filepath.Base(row.Cwd)})
			i = len(c.folders) - 1
		}
		c.folders[i].rows = append(c.folders[i].rows, row)
		c.count++
	}
	return c.folders
}
