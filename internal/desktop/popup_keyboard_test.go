package desktop

import (
	"bytes"
	"golang.org/x/mobile/event/key"
	"testing"
)

func TestEscapeDismissesOnlyClosablePopups(t *testing.T) {
	for _, flags := range []WindowFlags{windowPopup | windowNonblock, windowPopup | WindowClosable, windowPopup} {
		root := &Window{}
		popup := &Window{flags: flags}
		closed := 0
		popup.OnClose(func() { closed++ })
		ctx := &context{Windows: []*Window{root, popup}}
		ctx.processKeyEvent(key.Event{Code: key.CodeEscape, Direction: key.DirPress}, &bytes.Buffer{})
		shouldClose := flags&(windowNonblock|WindowClosable) != 0
		if (len(ctx.Windows) == 1) != shouldClose {
			t.Fatalf("flags %v: %d windows", flags, len(ctx.Windows))
		}
		if shouldClose && len(ctx.Input.Keyboard.events) != 0 {
			t.Fatal("Escape leaked into underlying conversation")
		}
		if (closed == 1) != shouldClose {
			t.Fatal("close callback missed or ran for retained dialog")
		}
	}
}
