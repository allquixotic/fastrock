package desktop

import (
	"golang.org/x/mobile/event/lifecycle"
	"testing"
	"time"
)

func TestClosedWindowStopsUpdater(t *testing.T) {
	w := &masterWindow{closing: true}
	done := make(chan struct{})
	go func() { w.updater(); close(done) }()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("closed window retained its renderer goroutine")
	}
}

func TestCloseRequestCanBeVetoed(t *testing.T) {
	called := false
	w := &masterWindow{onCloseRequested: func() bool { called = true; return false }}
	if !w.handleEventLocked(lifecycle.Event{To: lifecycle.StageDead}) || w.closing || !called {
		t.Fatal("close request bypassed the application guard")
	}
}
