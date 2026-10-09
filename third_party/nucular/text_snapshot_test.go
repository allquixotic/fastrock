package nucular

import (
	"bytes"
	"runtime"
	"testing"

	"github.com/aarzilli/nucular/font"
	"golang.org/x/mobile/event/key"
)

func TestSnapshotTracksDirectBufferEdits(t *testing.T) {
	e := TextEditor{Buffer: []rune("hello 🚀")}
	if e.Snapshot() != "hello 🚀" {
		t.Fatal(e.Snapshot())
	}
	e.Buffer[0] = 'H'
	if e.Snapshot() != "Hello 🚀" {
		t.Fatal(e.Snapshot())
	}
	if n := testing.AllocsPerRun(100, func() { _ = e.Snapshot() }); n != 0 {
		t.Fatalf("%g idle allocations", n)
	}
}

func TestNativeSelectAllShortcut(t *testing.T) {
	ed := TextEditor{Buffer: []rune("Select all\nincluding this line"), Cursor: 4}
	mod := key.ModControl
	if runtime.GOOS == "darwin" {
		mod = key.ModMeta
	}
	e := KeyboardEvent{kind: keyboardEventKey, key: key.Event{Code: key.CodeA, Modifiers: mod}}
	ed.key(&e, font.Face{}, 20, 200)
	if ed.SelectStart != 0 || ed.SelectEnd != len(ed.Buffer) {
		t.Fatalf("selection %d:%d", ed.SelectStart, ed.SelectEnd)
	}
}

func TestNativeKeyAndTextEventsDoNotInsertNUL(t *testing.T) {
	ctx := &context{}
	var text bytes.Buffer
	ctx.processKeyEvent(key.Event{Code: key.CodeA, Direction: key.DirPress}, &text)
	ctx.processKeyEvent(key.Event{Code: key.CodeUnknown, Rune: 'A', Direction: key.DirPress}, &text)
	if text.String() != "A" {
		t.Fatalf("keyboard generated %q", text.String())
	}
}
