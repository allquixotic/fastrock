package ui

import (
	"context"
	"reflect"
	"sort"

	"github.com/aarzilli/nucular"

	"github.com/allquixotic/fastrock/internal/workspace"
)

type sidebarFolder struct {
	path, title string
	rows        []*sidebarRow
}
type sidebarCache struct {
	starts                       []int
	totalRows                    int
	layoutReady                  bool
	query                        string
	archived, requested, ready   bool
	folders                      []sidebarFolder
	count                        int
	recent                       []*sidebarRow
	root, scanningRoot           *sidebarNode
	indexed, rebuild             bool
	observedSize                 int
	dirty                        map[string]bool
	scan                         *reflect.MapIter
	generation, failedGeneration uint64
	working                      bool
	cancel                       context.CancelFunc
	err                          string
}

// Specific IDs update only affected metadata. A full invalidation is reserved
// for an imported/replaced state, and its initial scan is bounded per call.
func (a *App) invalidateSidebar(ids ...string) {
	a.markSidebar(true, ids...)
}

func (a *App) markSidebar(reveal bool, ids ...string) {
	c := &a.sidebarCache
	a.invalidateSidebarView()
	if len(ids) == 0 {
		c.rebuild = true
		c.scan = nil
		return
	}
	if c.dirty == nil {
		c.dirty = map[string]bool{}
	}
	for _, id := range ids {
		if row := a.state.Chats[id]; reveal && row != nil {
			row.SidebarHidden = false
			row.SidebarRevision = c.generation
		}
		c.dirty[id] = true
	}
}

func (a *App) invalidateSidebarView() {
	c := &a.sidebarCache
	c.ready, a.recentReady = false, false
	c.generation++
	c.err = ""
	if c.cancel != nil {
		c.cancel()
	}
}

func (a *App) stopSidebarPreparation() {
	c := &a.sidebarCache
	if c.cancel != nil {
		c.cancel()
	}
	c.scan = nil
}

func (a *App) sidebarEmptyMessage() string {
	c, page := &a.sidebarCache, a.threadPage(a.archived)
	if !c.ready || c.count != 0 || c.err != "" || page.loading || page.err != "" || a.threadSearch.loading || a.threadSearch.err != "" {
		return ""
	}
	if text(a.sidebarSearch) != "" {
		return "No conversations match your search"
	}
	if a.archived {
		return "No archived conversations"
	}
	return "No conversations yet. Start one from the new tab page."
}

// Metadata is copied only on the UI owner, in bounded batches. Workers never
// inspect the mutable State.Chats map or any mutable Conversation.
func (a *App) updateSidebarIndex() bool {
	const batch = 256
	c := &a.sidebarCache
	if (!c.indexed || c.rebuild) && c.scan == nil {
		c.scanningRoot = nil
		c.scan = reflect.ValueOf(a.state.Chats).MapRange()
		c.rebuild = false
	}
	if c.scan != nil {
		for range batch {
			if !c.scan.Next() {
				c.scan = nil
				c.root, c.scanningRoot = c.scanningRoot, nil
				c.indexed = true
				break
			}
			row, _ := c.scan.Value().Interface().(*workspace.Conversation)
			if row != nil {
				c.scanningRoot = sidebarSet(c.scanningRoot, row.ID, sidebarMetadata(row))
			}
		}
		if c.scan != nil {
			if a.window != nil {
				a.window.Changed()
			}
			return false
		}
	}
	n := 0
	for id := range c.dirty {
		var metadata *sidebarRow
		if row := a.state.Chats[id]; row != nil {
			metadata = sidebarMetadata(row)
		}
		c.root = sidebarSet(c.root, id, metadata)
		delete(c.dirty, id)
		n++
		if n == batch {
			break
		}
	}
	if len(c.dirty) > 0 {
		if a.window != nil {
			a.window.Changed()
		}
		return false
	}
	return true
}

func (a *App) recentConversations() []*workspace.Conversation {
	a.sidebarFolders()
	if !a.recentReady {
		a.recentRows = a.recentRows[:0]
		for _, row := range a.sidebarCache.recent {
			if c := a.state.Chats[row.ID]; c != nil {
				a.recentRows = append(a.recentRows, c)
			}
		}
		a.recentReady = true
	}
	return a.recentRows
}

func (a *App) sidebarFolders() []sidebarFolder {
	c := &a.sidebarCache
	query := text(a.sidebarSearch)
	if !c.requested || c.query != query || c.archived != a.archived {
		c.requested, c.query, c.archived = true, query, a.archived
		a.invalidateSidebarView()
	}
	// Guard direct imports that did not produce a metadata event. Production
	// mutation paths supply IDs; this fallback is also bounded during startup.
	if c.observedSize != len(a.state.Chats) && len(c.dirty) == 0 && c.scan == nil {
		a.invalidateSidebar()
	}
	c.observedSize = len(a.state.Chats)
	if c.ready || !a.updateSidebarIndex() || c.working || c.failedGeneration == c.generation {
		return c.folders
	}
	if c.root == nil {
		c.folders, c.recent, c.count = nil, nil, 0
		c.ready, c.layoutReady, a.recentReady = true, false, false
		return c.folders
	}
	ctx := a.ctx
	if ctx == nil {
		ctx = context.Background()
	}
	ctx, cancel := context.WithCancel(ctx)
	c.cancel, c.working = cancel, true
	root, generation := c.root, c.generation
	var matches map[string]bool
	if query != "" && a.threadSearch.query == query && a.threadSearch.archived == a.archived {
		matches = a.threadSearch.matches
	}
	archived := a.archived
	a.work(func() {
		defer cancel()
		prepared, err := prepareSidebar(ctx, root, query, archived, matches)
		a.post(func() {
			c.working = false
			if generation != c.generation {
				return
			}
			if err != nil {
				c.err = err.Error()
				c.failedGeneration = generation
				return
			}
			c.folders, c.recent, c.count = prepared.folders, prepared.recent, prepared.count
			c.ready, c.layoutReady, a.recentReady = true, false, false
		})
	}, func() { c.working = false; c.err = errWorkQueueFull.Error(); c.failedGeneration = generation })
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

// Prefix row offsets make even a large number of projects cheap to scroll.
// Collapse changes rebuild one small index, not a flattened copy of history.
func (a *App) sidebarLayout() *sidebarCache {
	c := &a.sidebarCache
	if c.layoutReady {
		return c
	}
	if cap(c.starts) < len(c.folders)+1 {
		c.starts = make([]int, len(c.folders)+1)
	} else {
		c.starts = c.starts[:len(c.folders)+1]
	}
	n := 0
	for i, f := range c.folders {
		c.starts[i] = n
		n++
		if !a.collapsed[f.path] {
			n += len(f.rows)
		}
	}
	c.starts[len(c.folders)] = n
	c.totalRows = n
	c.layoutReady = true
	return c
}
func (a *App) collapseFolder(path string, collapsed bool) {
	a.collapsed[path] = collapsed
	a.sidebarCache.layoutReady = false
}
func (c *sidebarCache) folderAt(row int) int {
	return sort.Search(len(c.folders), func(i int) bool { return c.starts[i+1] > row })
}
