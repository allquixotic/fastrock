package ui

import (
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
)

type killableProcess interface{ Kill() error }

// A process may start after cancellation has already been acknowledged. Keep
// that decision until attach, but relinquish all control after commit.
type transferChild struct {
	mu             sync.Mutex
	process        killableProcess
	resolved, stop bool
}

func (c *transferChild) attach(p killableProcess) {
	c.mu.Lock()
	if !c.resolved {
		c.process = p
	}
	stop := c.stop
	c.mu.Unlock()
	if stop {
		_ = p.Kill()
	}
}
func (c *transferChild) exited() {
	c.mu.Lock()
	c.process = nil
	c.mu.Unlock()
}
func (c *transferChild) resolve(stop bool) {
	c.mu.Lock()
	if c.resolved {
		c.mu.Unlock()
		return
	}
	c.resolved, c.stop = true, stop
	p := c.process
	c.process = nil
	c.mu.Unlock()
	if stop && p != nil {
		go func() { _ = p.Kill() }()
	}
}

func (a *App) finishTransfer(ticket string, committed bool) {
	id := a.transfers[ticket]
	if timer := a.transferTimers[ticket]; timer != nil {
		timer.Stop()
	}
	delete(a.transferTimers, ticket)
	delete(a.transferCancelling, ticket)
	if child := a.transferChildren[ticket]; child != nil {
		child.resolve(!committed)
	}
	delete(a.transferChildren, ticket)
	delete(a.transfers, ticket)
	delete(a.popping, id)
	if committed && id != "" {
		a.closeTransferredTab(id)
	}
	if state := a.windowReturn; state != nil && state.ticket == ticket {
		state.ticket = ""
		if committed {
			a.returnNextDocument()
		} else {
			a.failWindowReturn(fmt.Errorf("destination cancelled the transfer"))
		}
	}
}

func (a *App) stopTransferTimers() {
	for ticket, timer := range a.transferTimers {
		timer.Stop()
		delete(a.transferTimers, ticket)
	}
}

// Freeze the source until the broker resolves cancellation against commit.
// A lost reply is not permission to kill a potentially committed window.
func (a *App) abortTransfer(id, ticket string, reason error) {
	if ticket == "" {
		delete(a.popping, id)
		a.report(reason)
		return
	}
	if a.transfers[ticket] != id || a.transferCancelling[ticket] {
		return
	}
	if a.transferCancelling == nil {
		a.transferCancelling = map[string]bool{}
	}
	a.transferCancelling[ticket] = true
	a.rpcResult("fastrock/cancel", map[string]string{"ticket": ticket}, func(raw json.RawMessage) {
		if a.transfers[ticket] != id {
			return
		}
		var result codex.TransferResult
		if json.Unmarshal(raw, &result) != nil || (result.State != codex.TransferCancelled && result.State != codex.TransferCommitted) {
			a.unconfirmedTransfer(id, ticket, fmt.Errorf("invalid cancellation response"))
			return
		}
		a.finishTransfer(ticket, result.State == codex.TransferCommitted)
		if result.State == codex.TransferCancelled {
			a.report(reason)
		}
	}, func(err error) { a.unconfirmedTransfer(id, ticket, err) })
}
func (a *App) unconfirmedTransfer(id, ticket string, err error) {
	if a.transfers[ticket] != id {
		return
	}
	delete(a.transferCancelling, ticket)
	if state := a.windowReturn; state != nil && state.ticket == ticket {
		a.windowReturn = nil
	}
	a.report(fmt.Errorf("window transfer outcome is unconfirmed; source retained and destination left running: %w", err))
	if a.client != nil {
		a.watchTransferAfter(id, ticket, 5*time.Second)
	}
}
func (a *App) watchTransfer(id, ticket string) { a.watchTransferAfter(id, ticket, 30*time.Second) }
func (a *App) watchTransferAfter(id, ticket string, delay time.Duration) {
	if a.transferTimers == nil {
		a.transferTimers = map[string]*time.Timer{}
	}
	if timer := a.transferTimers[ticket]; timer != nil {
		timer.Stop()
	}
	a.transferTimers[ticket] = time.AfterFunc(delay, func() {
		a.post(func() {
			if a.transfers[ticket] == id {
				a.abortTransfer(id, ticket, fmt.Errorf("destination window did not accept the tab; the source is still here"))
			}
		})
	})
}

func (a *App) finishIncoming() {
	a.transferPending = false
	a.incomingTicket, a.incomingTab, a.readyTicket = "", "", ""
	buffer := a.transferBuffer
	a.transferBuffer = nil
	for _, messages := range buffer {
		for _, message := range messages {
			a.event(message)
		}
	}
}
func (a *App) resolveIncoming(ticket string, reason error) {
	a.rpcResult("fastrock/cancel", map[string]string{"ticket": ticket}, func(raw json.RawMessage) {
		if a.incomingTicket != ticket {
			return
		}
		var result codex.TransferResult
		if json.Unmarshal(raw, &result) == nil {
			switch result.State {
			case codex.TransferCommitted:
				a.finishIncoming()
				return
			case codex.TransferCancelled:
				a.cancelIncoming()
				a.report(reason)
				return
			}
		}
		a.report(fmt.Errorf("window transfer outcome is unconfirmed; document retained: %w", reason))
	}, func(err error) {
		if a.incomingTicket == ticket {
			a.report(fmt.Errorf("window transfer outcome is unconfirmed; document retained: %w", err))
		}
	})
}

// Only a cancelled, empty newly spawned window may close automatically. On a
// broker disconnect an installed document stays available for recovery.
func (a *App) closeAbandonedPopout() {
	if a.connection.Ticket != "" && len(a.state.Tabs) == 0 && a.window != nil {
		a.allowWindowClose = true
		a.window.Close()
	}
}
