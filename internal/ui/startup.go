package ui

import (
	"errors"
	"fmt"

	"github.com/allquixotic/fastrock/internal/desktop"
)

type connectionResult struct {
	connection Connection
	err        error
}

func (a *App) restoreWhenReady() {
	if a.restorePending && !a.sessionLoading && a.client != nil {
		a.restorePending = false
		a.restoreDocuments()
	}
}

func (a *App) windowPresented() {
	if a.presented {
		return
	}
	a.presented = true
	if a.client != nil {
		a.initializeConnection(a.connection)
	} else {
		a.reconnect()
	}
}

func (a *App) beginConnection() {
	if !a.presented || a.connecting || a.ctx.Err() != nil {
		return
	}
	a.connecting = true
	a.serverError = ""
	a.status = "Starting Codex…"
	done, results := make(chan struct{}), make(chan connectionResult, 1)
	a.connectionDone, a.connectionResults = done, results
	connect, ctx := a.connection.Connect, a.ctx
	go func() {
		result := connectionResult{}
		defer func() {
			if p := recover(); p != nil {
				result.err = fmt.Errorf("Codex startup failed: %v", p)
			}
			results <- result
			close(done)
			a.post(func() { a.finishConnection(results) })
		}()
		result.connection, result.err = connect(ctx)
		if result.err == nil && result.connection.Client == nil {
			result.err = errors.New("Codex startup returned no connection")
		}
	}()
}

func (a *App) finishConnection(results chan connectionResult) {
	select {
	case result := <-results:
		if a.ctx.Err() != nil || results != a.connectionResults {
			if result.connection.Client != nil {
				result.connection.Client.Close()
			}
			return
		}
		a.connecting = false
		if result.err != nil {
			if result.connection.Client != nil {
				result.connection.Client.Close()
			}
			a.serverError = result.err.Error()
			a.status = "Codex unavailable · Retry when ready"
			return
		}
		connect := a.connection.Connect
		reconnecting := a.connection.Client != nil
		a.connection = result.connection
		a.connection.Connect = connect
		a.client = result.connection.Client
		a.serverError, a.startedProvider = "", ""
		a.serverPaused, a.serverStarting = false, false
		a.serverGeneration++
		if reconnecting {
			a.resetInfoConnection()
			a.openThreads = nil
			a.publishThreads()
			for id, v := range a.chats {
				if chat := a.state.Chats[id]; chat != nil {
					chat.Draft = text(v.Editor)
					chat.DraftAttachments = v.Attachments
				}
				delete(a.chats, id)
			}
		}
		// Replace any queued offline write's nil client and carry local edits
		// into the connection without waiting for another settings change.
		a.preferencesQueued = a.preferencesServer
		a.savePrefs()
		a.initializeConnection(a.connection)
	default:
	}
}

// Cancel the window context first. Join startup before the primary process
// decides whether to keep its shared backend alive for remaining windows.
func (a *App) stopStartup() {
	if a.connectionDone == nil {
		return
	}
	<-a.connectionDone
	select {
	case result := <-a.connectionResults:
		if result.connection.Client != nil {
			result.connection.Client.Close()
		}
	default:
	}
}

func (a *App) drawConnectionStatus(w *desktop.Window) {
	if a.client != nil {
		return
	}
	if a.connecting || !a.presented {
		w.Row(28).Dynamic(1)
		muted(w, "Starting Codex… You can keep using Fastrock.", a.p)
		return
	}
	w.Row(48).Dynamic(1)
	w.LabelWrap("Codex unavailable: " + a.serverError)
	w.Row(28).Static(110, 130)
	if w.ButtonText("Retry Codex") {
		a.reconnect()
	}
	if w.ButtonText("Codex settings") {
		a.openSettings()
	}
}
