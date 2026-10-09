package ui

import (
	"encoding/json"
	"fmt"
	"slices"

	"github.com/allquixotic/fastrock/internal/workspace"
)

type windowReturn struct {
	destination string
	ticket      string
	offering    bool
}

type returnWindow struct{ ID, Name string }

// Prefer the main window; stable ordering keeps the choice predictable when
// only pop-outs remain. Never create a new window as part of closing one.
func returnDestination(windows []returnWindow) string {
	slices.SortFunc(windows, func(a, b returnWindow) int {
		if (a.Name == "Fastrock") != (b.Name == "Fastrock") {
			if a.Name == "Fastrock" {
				return -1
			}
			return 1
		}
		if a.ID < b.ID {
			return -1
		}
		if a.ID > b.ID {
			return 1
		}
		return 0
	})
	if len(windows) == 0 {
		return ""
	}
	return windows[0].ID
}

// Called only after the usual unsaved-work/running-turn close guards. Returns
// false while live documents are being acknowledged by another window.
func (a *App) beginWindowClose() bool {
	for id := range a.rallyViews {
		if a.pendingRallyWrite(id) {
			a.toast = "Wait for the Rally update to finish before closing"
			return false
		}
	}
	if a.connection.Ticket == "" || len(a.state.Tabs) == 0 || a.client == nil {
		return true
	}
	state := &windowReturn{}
	a.windowReturn = state
	a.toast = "Returning documents to another window before closing…"
	a.rpcResult("fastrock/windows", nil, func(raw json.RawMessage) {
		if a.windowReturn != state {
			return
		}
		var result struct{ Windows []returnWindow }
		if err := json.Unmarshal(raw, &result); err != nil {
			a.failWindowReturn(err)
			return
		}
		state.destination = returnDestination(result.Windows)
		if state.destination == "" {
			a.windowReturn = nil
			a.closeWindowNow()
			return
		}
		a.returnNextDocument()
	}, func(err error) { a.failWindowReturn(err) })
	return false
}

func (a *App) returnNextDocument() {
	state := a.windowReturn
	if state == nil || state.ticket != "" || state.offering {
		return
	}
	if len(a.state.Tabs) == 0 {
		a.windowReturn = nil
		a.closeWindowNow()
		return
	}
	tab := a.state.Tabs[0]
	if a.pendingRallyWrite(tab.ID) {
		a.failWindowReturn(fmt.Errorf("wait for the Rally update to finish before closing"))
		return
	}
	payload := a.tabSnapshot(tab)
	state.offering = true
	if a.popping == nil {
		a.popping = map[string]bool{}
	}
	a.popping[tab.ID] = true
	a.rpcResult("fastrock/offer", map[string]any{"data": payload}, func(raw json.RawMessage) {
		if a.windowReturn != state {
			return
		}
		state.offering = false
		var offer struct{ Ticket string }
		if json.Unmarshal(raw, &offer) != nil || offer.Ticket == "" {
			delete(a.popping, tab.ID)
			a.failWindowReturn(fmt.Errorf("invalid transfer ticket"))
			return
		}
		state.ticket = offer.Ticket
		if a.transfers == nil {
			a.transfers = map[string]string{}
		}
		a.transfers[offer.Ticket] = tab.ID
		a.watchTransfer(tab.ID, offer.Ticket)
		a.rpcResult("fastrock/move", map[string]string{"ticket": offer.Ticket, "window": state.destination}, nil, func(err error) { a.abortTransfer(tab.ID, offer.Ticket, err) })
	}, func(err error) {
		delete(a.popping, tab.ID)
		a.failWindowReturn(err)
	})
}

func (a *App) failWindowReturn(err error) {
	a.windowReturn = nil
	a.report(fmt.Errorf("could not return all documents; this window is still open: %w", err))
}
func (a *App) closeWindowNow() {
	a.allowWindowClose = true
	if a.window != nil {
		a.window.Close()
	}
}

func (a *App) closeTransferredTab(id string) {
	thread := ""
	for _, tab := range a.state.Tabs {
		if tab.ID == id && tab.Kind == workspace.Chat {
			thread = tab.Target
			break
		}
	}
	if thread != "" {
		if a.detached == nil {
			a.detached = map[string]bool{}
		}
		a.detached[thread] = true
	}
	a.closeTabNow(id)
	if thread != "" {
		a.releaseTransferredConversation(thread)
	}
}
func (a *App) releaseTransferredConversation(id string) {
	if c := a.state.Chats[id]; c != nil {
		c.Status, c.TurnID = "idle", ""
		c.Draft, c.EditQueue = "", ""
		c.DraftAttachments, c.Queue, c.Outbox = nil, nil, nil
		c.QueueDraft = workspace.Draft{}
		c.ReleaseTranscript()
	}
	delete(a.draftChats, id)
	delete(a.chats, id)
	delete(a.mailbox, id)
	a.invalidateSidebar(id)
}
