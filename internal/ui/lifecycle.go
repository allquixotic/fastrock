package ui

import (
	"context"
	"encoding/json"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/richtext"
	"runtime"
	"runtime/debug"
	"time"

	"github.com/allquixotic/fastrock/internal/workspace"
)

// Drop only reconstructible state. Drafts and pending edits stay owned by their
// tabs. This also runs for close-other/close-right and window transfer.
func (a *App) closeTab(id string) {
	if a.pendingRallyWrite(id) {
		a.toast = "Wait for the Rally update to finish before closing"
		return
	}
	if a.windowReturn != nil || a.popping[id] || a.transferPending && a.incomingTab == id {
		a.toast = "Wait for the document transfer to finish before closing"
		return
	}
	for _, tab := range a.state.Tabs {
		if tab.ID == id && tab.Kind == workspace.Settings && a.settingsView.rawDirty() {
			a.leaveRawConfig(func() { a.closeTabNow(id) })
			return
		}
	}
	for _, t := range a.state.Tabs {
		if t.ID == id && t.Kind == workspace.Chat {
			for _, request := range a.approvals {
				if request.ThreadID == t.Target {
					a.confirm("Close waiting conversation?", "A request in this tab is waiting for your answer. Closing will stop the turn and dismiss its requests.", func() { a.stopTurnsThen([]string{id}, false, func() { a.closeTabNow(id) }) })
					return
				}
			}
			if c := a.state.Chats[t.Target]; c != nil && c.Busy() {
				a.confirm("Close running thread?", "Stop the running turn and close this tab?", func() { a.stopAndClose(id, c) })
				return
			}
		}
	}
	if v := a.rallyViews[id]; v != nil && v.Detail != nil && v.Detail.dirty() {
		a.leaveDetail(v, func() { a.closeTabNow(id) })
		return
	}
	a.closeTabNow(id)
}
func (a *App) closeTabNow(id string) {
	delete(a.documentCheckpoints, id)
	for _, t := range a.state.Tabs {
		if t.ID == id {
			if v := a.rallyViews[id]; v != nil {
				if m := v.SavedViewManager; m != nil && m.cancel != nil {
					m.cancel()
				}
				v.FilterDraft.closePicker()
				if v.searchTimer != nil {
					v.searchTimer.Stop()
				}
				if v.ExportCancel != nil {
					v.ExportCancel()
				}
				a.disposeDetail(v.Detail)
				if v.cancel != nil {
					v.cancel()
				}
				v.Generation++
				v.Closed = true
				delete(a.rallyViews, id)
			}
			if f := a.files[id]; f != nil {
				f.dispose()
			}
			delete(a.files, id)
			if t.Kind == workspace.Chat {
				a.cancelRecap(t.Target)
				a.declineConsents(t.Target, "The target conversation was closed")
				delete(a.infoViews, t.Target)
				a.rejectClosingApprovals(t.Target)
				if view := a.chats[t.Target]; view != nil && view.SuggestCancel != nil {
					view.SuggestCancel()
				}
				if a.client != nil && !a.detached[t.Target] {
					client, thread := a.client, t.Target
					a.work(func() {
						ctx, cancel := context.WithTimeout(a.ctx, 5*time.Second)
						defer cancel()
						_ = client.Call(ctx, "thread/unsubscribe", map[string]any{"threadId": thread}, nil)
					})
				}
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
						a.invalidateSidebar(c.ID)
						delete(a.chats, c.ID)
					} else {
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
	if a.store != nil && time.Since(a.lastAttachmentCollection) > time.Hour {
		a.lastAttachmentCollection = time.Now()
		refs, root := a.attachmentReferences(), a.store.Dir
		a.work(func() { collectAttachments(root, refs) })
	}
	// ReadMemStats and scavenging run off the renderer. The result is applied at a
	// frame boundary, with one pressure cycle at a time.
	client := a.client
	a.work(func() {
		memory := platform.ProcessMemoryBytes()
		var all struct {
			Bytes uint64
			Count int
		}
		if client != nil {
			ctx, cancel := context.WithTimeout(a.ctx, 3*time.Second)
			_ = client.Call(ctx, "fastrock/memory", map[string]any{"bytes": memory}, &all)
			cancel()
		}
		var stats runtime.MemStats
		runtime.ReadMemStats(&stats)
		richtext.SetHistoryBudget((8 << 20) / max(1, all.Count))
		budget := max(int64(96<<20), int64(256<<20)/int64(max(1, all.Count)))
		debug.SetMemoryLimit(max(budget, int64(stats.HeapAlloc)*5/4))
		a.post(func() {
			a.memoryBytes = all.Bytes
			if a.memoryBytes == 0 {
				a.memoryBytes = memory
			}
			if stats.HeapAlloc < uint64(budget)*3/4 {
				return
			}
			active := a.state.Current()
			for id, v := range a.rallyViews {
				if active != nil && id == active.ID {
					continue
				}
				if v.Loading || len(v.PendingCards) > 0 || v.Detail != nil && v.Detail.dirty() {
					continue
				}
				v.Items = nil
				v.filterSource = nil
				v.cardSource = nil
				if v.Detail != nil {
					v.Detail.Items = nil
					v.Detail.collectionLayout = nil
					v.Detail.collectionLayoutPending = nil
					v.Detail.taskLayout = nil
				}
				v.cards = nil
				v.filterCache = nil
				v.groupCounts = nil
				v.filterSearch = nil
				v.tableLinks = nil
				v.boardGroups = nil
				v.boardPrepared = false
				v.planningSummary = planningSummary{}
				v.planningPrepared = false
				v.Evicted = true
			}
			for id, f := range a.files {
				if active != nil && id == active.ID || f.Virtual {
					continue
				}
				f.evictEditor()
				f.DiffGeneration++
				f.Diff, f.DiffLayout = nil, nil
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
	for _, id := range ids {
		if a.pendingRallyWrite(id) {
			a.toast = "Wait for the Rally update to finish before closing"
			return
		}
		if a.windowReturn != nil || a.popping[id] || a.transferPending && a.incomingTab == id {
			a.toast = "Wait for the document transfer to finish before closing"
			return
		}
	}
	for _, id := range ids {
		for _, tab := range a.state.Tabs {
			if tab.ID == id && tab.Kind == workspace.Settings && a.settingsView.rawDirty() {
				a.leaveRawConfig(func() { a.closeTabs(ids) })
				return
			}
		}
	}
	ids = append([]string(nil), ids...)
	dirty, busy := false, false
	for _, id := range ids {
		if v := a.rallyViews[id]; v != nil && v.Detail != nil && v.Detail.dirty() {
			dirty = true
		}
		for _, tab := range a.state.Tabs {
			if tab.ID == id && tab.Kind == workspace.Chat {
				if c := a.state.Chats[tab.Target]; c != nil && c.Busy() {
					busy = true
				}
			}
		}
	}
	closeAll := func() {
		a.stopTurnsThen(ids, false, func() {
			for _, id := range ids {
				a.closeTabNow(id)
			}
		})
	}
	switch {
	case dirty && busy:
		a.confirm("Close selected tabs?", "Stop running turns and discard unsaved Rally changes in the selected tabs?", closeAll)
	case dirty:
		a.confirm("Close unsaved work items?", "Discard unsaved Rally changes in the selected tabs?", closeAll)
	case busy:
		a.confirm("Close running threads?", "Stop running turns and close the selected tabs?", closeAll)
	default:
		closeAll()
	}
}

func (a *App) canCloseWindow() bool {
	if a.sessionLoading {
		a.toast = "Wait for workspace restoration to finish before closing"
		return false
	}
	if a.allowWindowClose {
		return true
	}
	if a.windowReturn != nil || a.transferPending || len(a.popping) > 0 {
		a.toast = "Wait for the document transfer to finish before closing"
		return false
	}
	if a.settingsView.rawDirty() {
		a.leaveRawConfig(a.requestQuit)
		return false
	}
	var ids []string
	busy := a.assistant != nil && a.assistant.Busy
	dirty := false
	for _, tab := range a.state.Tabs {
		ids = append(ids, tab.ID)
		if v := a.rallyViews[tab.ID]; v != nil && v.Detail != nil && v.Detail.dirty() {
			dirty = true
		}
		if tab.Kind == workspace.Chat {
			if c := a.state.Chats[tab.Target]; c != nil && c.Busy() {
				busy = true
			}
		}
	}
	if !busy && !dirty {
		return a.beginWindowClose()
	}
	message := "Stop the running turns in this window and quit? Unsent drafts will be saved."
	if dirty {
		message = "Discard unsaved Rally changes and quit? Running turns in this window will be stopped. Unsent chat drafts will be saved."
	}
	title := "Quit Fastrock?"
	if a.connection.Ticket != "" {
		title = "Close window?"
		message = "Close this window? Running turns will be stopped. Drafts and unsaved edits will return to another open window or be saved for recovery."
	}
	a.confirm(title, message, func() {
		a.stopTurnsThen(ids, true, func() {
			if a.beginWindowClose() {
				a.closeWindowNow()
			}
		})
	})
	return false
}
func (a *App) requestQuit() {
	if a.canCloseWindow() {
		a.window.Close()
	}
}

func (a *App) stopTurnsThen(ids []string, includeAssistant bool, done func()) {
	type turn struct{ thread, id string }
	var turns []turn
	seen := map[string]bool{}
	for _, id := range ids {
		for _, tab := range a.state.Tabs {
			if tab.ID != id || tab.Kind != workspace.Chat {
				continue
			}
			c := a.state.Chats[tab.Target]
			if c == nil || !c.Busy() || seen[c.ID] {
				continue
			}
			if c.TurnID == "" {
				a.toast = "Wait for the starting turn, then close it"
				return
			}
			seen[c.ID] = true
			c.QueuePaused = true
			turns = append(turns, turn{c.ID, c.TurnID})
		}
	}
	if includeAssistant && a.assistant != nil && a.assistant.Busy {
		if a.assistant.TurnID == "" {
			a.toast = "Wait for the Rally assistant to start, then quit"
			return
		}
		turns = append(turns, turn{a.assistant.ThreadID, a.assistant.TurnID})
	}
	if len(turns) == 0 {
		done()
		return
	}
	client := a.client
	if client == nil {
		a.toast = "Codex is disconnected; could not stop the running turns"
		return
	}
	a.controlWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var err error
		for _, turn := range turns {
			if err = client.Call(ctx, "turn/interrupt", map[string]string{"threadId": turn.thread, "turnId": turn.id}, nil); err != nil {
				break
			}
		}
		a.post(func() {
			if err != nil {
				a.report(err)
				return
			}
			if a.client != client {
				a.toast = "Codex reconnected; review the current turns before closing"
				return
			}
			for _, turn := range turns {
				if c := a.state.Chats[turn.thread]; c != nil && c.Busy() && c.TurnID != turn.id {
					a.toast = "A new turn started; close again after reviewing it"
					return
				}
			}
			done()
		})
	})
}

func (a *App) stopAndClose(id string, c *workspace.Conversation) {
	if c.TurnID == "" {
		a.toast = "Wait for the starting turn, then close it"
		return
	}
	a.rpcResult("turn/interrupt", map[string]any{"threadId": c.ID, "turnId": c.TurnID}, func(_ json.RawMessage) { c.QueuePaused = true; a.closeTabNow(id) }, nil)
}
func (a *App) closeChatTabs(thread string) {
	var ids []string
	for _, t := range a.state.Tabs {
		if t.Kind == workspace.Chat && t.Target == thread {
			ids = append(ids, t.ID)
		}
	}
	for _, id := range ids {
		a.closeTab(id)
	}
}
