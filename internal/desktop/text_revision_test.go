package desktop

import (
	"slices"
	"strings"
	"testing"
)

func TestV4TrackedEditorReportsEveryMutation(t *testing.T) {
	ed := &TextEditor{}
	ed.clearState(TextEditMultiLine)
	ed.TrackChanges()
	var replay []rune
	ed.OnChange = func(change TextChange) {
		replay = slices.Replace(replay, change.Start, change.Start+change.Removed, change.Inserted...)
		if !slices.Equal(replay, ed.Buffer) {
			t.Fatalf("change does not reconstruct buffer: %+v %q != %q", change, string(replay), string(ed.Buffer))
		}
	}
	ed.SetText("hello 界")
	ed.Cursor = len(ed.Buffer)
	ed.Text([]rune("!🚀"))
	ed.SelectStart, ed.SelectEnd = 0, 5
	ed.Paste("goodbye")
	ed.DoUndo()
	ed.DoRedo()
	ed.InsertMode = true
	ed.Cursor = 0
	ed.SelectStart, ed.SelectEnd = 0, 0
	ed.Text([]rune("G"))
	ed.Delete(1, 2)
	if got := ed.Snapshot(); got != string(replay) {
		t.Fatal(got, string(replay))
	}
	revision := ed.TextRevision()
	ed.SelectAll()
	if ed.TextRevision() != revision {
		t.Fatal("selection dirtied document")
	}
	if n := testing.AllocsPerRun(100, func() { _ = ed.Snapshot() }); n != 0 {
		t.Fatal("idle snapshot allocated", n)
	}
}

func BenchmarkV4TrackedLargeSnapshot(b *testing.B) {
	ed := &TextEditor{}
	ed.TrackChanges()
	ed.SetText(strings.Repeat("a", 4<<20))
	ed.Snapshot()
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		ed.Snapshot()
	}
}

func TestV23EditorInitializationPreservesRestoredCaret(t *testing.T) {
	ed := &TextEditor{Buffer: []rune("hello 世界"), Flags: EditBox, Cursor: 6, SelectStart: 2, SelectEnd: 6}
	ed.Scrollbar.Y = 50
	ed.init(&Window{})
	if ed.Cursor != 6 || ed.SelectStart != 2 || ed.SelectEnd != 6 || ed.Scrollbar.Y != 50 || !ed.Initialized {
		t.Fatalf("restored state reset during native initialization: %+v", ed)
	}
}
