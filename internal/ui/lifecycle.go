package ui

import (
	"context"
	"github.com/allquixotic/fastrock/internal/platform"
	"runtime/debug"
	"time"

	"github.com/allquixotic/fastrock/internal/workspace"
)

// Drop only reconstructible state. Drafts and pending edits stay owned by their
// tabs. This also runs for close-other/close-right and window transfer.
func (a *App) closeTab(id string) {
	if v := a.rallyViews[id]; v != nil && v.Detail != nil && v.Detail.dirty() {
		a.confirm("Close unsaved work item?", "Discard the unsaved changes in this tab?", func() { a.closeTabNow(id) })
		return
	}
	a.closeTabNow(id)
}
func (a *App) closeTabNow(id string) {
	for _, t := range a.state.Tabs {
		if t.ID == id {
			if v := a.rallyViews[id]; v != nil {
				if v.cancel != nil {
					v.cancel()
				}
				v.Generation++
				v.Closed = true
				delete(a.rallyViews, id)
			}
			delete(a.files, id)
			if t.Kind == workspace.Chat {
				if c := a.state.Chats[t.Target]; c != nil {
					if v := a.chats[c.ID]; v != nil {
						c.Draft = text(v.Editor)
						c.DraftAttachments = v.Attachments
						a.rememberDraft(c.ID)
					}
					if c.Ephemeral {
						if c.Busy() && c.TurnID != "" {
							a.rpc("turn/interrupt", map[string]any{"threadId": c.ID, "turnId": c.TurnID}, nil)
						}
						delete(a.state.Chats, c.ID)
						delete(a.chats, c.ID)
					} else if !c.Busy() {
						c.ReleaseTranscript()
						delete(a.chats, c.ID)
					}
				}
			}
			break
		}
	}
	a.state.Close(id)
}
func (a *App) maintain() {
	if time.Since(a.lastMaintenance) < 10*time.Second {
		return
	}
	a.lastMaintenance = time.Now()
	// ReadMemStats and scavenging run off the renderer. The result is applied at a
	// frame boundary, with one pressure cycle at a time.
	client := a.client
	a.work(func() {
		memory := platform.ResidentMemory()
		var all struct {
			Bytes uint64
			Count int
		}
		if client != nil {
			ctx, cancel := context.WithTimeout(a.ctx, 3*time.Second)
			_ = client.Call(ctx, "fastrock/memory", map[string]any{"bytes": memory}, &all)
			cancel()
		}
		budget := int64(256<<20) / int64(max(1, all.Count))
		debug.SetMemoryLimit(budget)
		a.post(func() {
			a.memoryBytes = all.Bytes
			if a.memoryBytes == 0 {
				a.memoryBytes = memory
			}
			if memory < uint64(budget)*3/4 && all.Bytes < 384<<20 {
				return
			}
			active := a.state.Current()
			for id, v := range a.rallyViews {
				if active != nil && id == active.ID {
					continue
				}
				if v.Loading || v.Detail != nil && v.Detail.dirty() {
					continue
				}
				v.Items = nil
				v.cards = nil
				v.filterCache = nil
				v.filterSearch = nil
				v.boardGroups = nil
				v.boardPrepared = false
				v.Evicted = true
			}
			for id, f := range a.files {
				if active != nil && id == active.ID || f.Virtual {
					continue
				}
				f.Editor = nil
				f.Diff = nil
			}
			for id, c := range a.state.Chats {
				if c.Ephemeral || c.Busy() || active != nil && active.Target == id {
					continue
				}
				if v := a.chats[id]; v != nil {
					c.Draft = text(v.Editor)
					c.DraftAttachments = v.Attachments
					a.rememberDraft(id)
				}
				c.ReleaseTranscript()
				delete(a.chats, id)
			}
			if a.rallyClient != nil {
				a.rallyClient.PurgeCache()
			}
			a.work(debug.FreeOSMemory)
		})
	})
}

func (a *App) closeTabs(ids []string) {
	closeAll := func() {
		for _, id := range ids {
			a.closeTabNow(id)
		}
	}
	for _, id := range ids {
		if v := a.rallyViews[id]; v != nil && v.Detail != nil && v.Detail.dirty() {
			a.confirm("Close unsaved work items?", "Discard unsaved changes in the selected tabs?", closeAll)
			return
		}
	}
	closeAll()
}
