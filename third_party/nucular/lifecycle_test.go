package nucular

import (
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
