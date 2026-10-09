package desktop

import (
	"github.com/allquixotic/fastrock/internal/desktop/internal/windowing"
	"golang.org/x/mobile/event/lifecycle"
	"testing"
	"time"
)

func TestV73PresentationCallbackRequiresNativeFrame(t *testing.T) {
	w := &masterWindow{masterWindowCommon: masterWindowCommon{ctx: &context{}}}
	calls := 0
	w.OnPresented(func() { calls++ })
	w.handleEventLocked(lifecycle.Event{To: lifecycle.StageFocused})
	if calls != 0 {
		t.Fatal("focus was mistaken for a displayed frame")
	}
	w.handleEventLocked(windowing.PresentedEvent{})
	w.handleEventLocked(windowing.PresentedEvent{})
	if calls != 1 {
		t.Fatal("presentation callback was not once-only", calls)
	}
	w = &masterWindow{closing: true}
	w.OnPresented(func() { calls++ })
	w.handleEventLocked(windowing.PresentedEvent{})
	if calls != 1 {
		t.Fatal("closing window invoked a startup callback")
	}
}

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
