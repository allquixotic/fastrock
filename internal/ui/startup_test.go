//go:build fltk_headless

package ui

import (
	"context"
	"errors"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/allquixotic/fastrock/internal/settings"
)

func startupFixture(t *testing.T) *App {
	a := shellFixture(t)
	a.ctx, a.cancel = context.WithCancel(a.ctx)
	t.Cleanup(a.cancel)
	return a
}

func TestV73StartupWaitsForPresentationAndRetries(t *testing.T) {
	a := startupFixture(t)
	client := a.client
	a.client = nil
	var calls atomic.Int32
	started, unblock := make(chan struct{}, 2), make(chan struct{})
	a.connection.Connect = func(ctx context.Context) (Connection, error) {
		n := calls.Add(1)
		started <- struct{}{}
		if n == 1 {
			select {
			case <-unblock:
			case <-ctx.Done():
				return Connection{}, ctx.Err()
			}
			return Connection{}, errors.New("CLI temporarily blocked during update")
		}
		return Connection{Client: client, Address: "fixture", Token: "fixture"}, nil
	}
	a.reconnect()
	if calls.Load() != 0 || a.connecting {
		t.Fatal("Codex was invoked before a native frame was presented")
	}
	a.windowPresented()
	<-started
	a.reconnect()
	if calls.Load() != 1 {
		t.Fatal("overlapping startup attempts")
	}
	close(unblock)
	drain(t, a, func() bool { return !a.connecting })
	if a.fatal != "" || a.client != nil || !strings.Contains(a.serverError, "temporarily blocked") {
		t.Fatal("recoverable startup failure became fatal", a.serverError, a.fatal)
	}
	a.reconnect()
	drain(t, a, func() bool { return !a.connecting })
	if calls.Load() != 2 || a.client != client || a.serverError != "" || a.connection.Connect == nil {
		t.Fatal("retry did not reconnect", calls.Load(), a.serverError)
	}
	a.stopStartup()
}

func TestV73StartupCloseCancelsPendingWork(t *testing.T) {
	a := startupFixture(t)
	a.client = nil
	a.connection.Connect = func(ctx context.Context) (Connection, error) {
		<-ctx.Done()
		return Connection{}, ctx.Err()
	}
	a.windowPresented()
	a.cancel()
	a.stopStartup()
	if a.client != nil {
		t.Fatal("closing a window retained a startup connection")
	}
}

func TestV73StartupCloseDisposesUnclaimedClient(t *testing.T) {
	a := startupFixture(t)
	client := a.client
	a.client = nil
	a.connection.Connect = func(context.Context) (Connection, error) {
		return Connection{Client: client}, nil
	}
	a.windowPresented()
	<-a.connectionDone
	a.cancel()
	a.stopStartup()
	if err := client.Call(context.Background(), "fixture", nil, nil); err == nil {
		t.Fatal("unclaimed client was not closed", err)
	}
}

func TestV73StartupPanicRemainsRetryable(t *testing.T) {
	a := startupFixture(t)
	a.client = nil
	a.connection.Connect = func(context.Context) (Connection, error) { panic("fixture") }
	a.windowPresented()
	drain(t, a, func() bool { return !a.connecting })
	if a.fatal != "" || !strings.Contains(a.serverError, "fixture") {
		t.Fatal("startup panic left the window stuck", a.serverError)
	}
	a.stopStartup()
}

func TestV73StartupWaitsForSessionRestore(t *testing.T) {
	a := startupFixture(t)
	a.sessionLoading, a.restorePending = true, true
	a.restoreWhenReady()
	if !a.restorePending {
		t.Fatal("connection restored documents before the saved workspace arrived")
	}
	a.applySessions(loadedSessions{})
	if a.sessionLoading || a.restorePending {
		t.Fatal("completed session load did not release pending document restore")
	}
}

func TestV73StartupRetainsOfflinePreferences(t *testing.T) {
	a := startupFixture(t)
	client := a.client
	a.client = nil
	a.prefs = settings.Defaults()
	a.preferencesServer, a.preferencesQueued = a.prefs, a.prefs
	a.preferences = make(chan preferenceWrite, 1)
	a.prefs.Sidebar = false
	a.savePrefs()
	a.connection.Connect = func(context.Context) (Connection, error) {
		return Connection{Client: client}, nil
	}
	a.windowPresented()
	drain(t, a, func() bool { return !a.connecting })
	write := <-a.preferences
	if write.Client != client || write.Snapshot.Sidebar || string(write.Patch["sidebar"]) != "false" {
		t.Fatal("retry did not replace the offline preference write", write)
	}
	a.stopStartup()
}
