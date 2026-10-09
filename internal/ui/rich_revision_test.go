//go:build nucular_headless

package ui

import (
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/richtext"
)

func TestV4RichEditorAppliesNativeChangesOnce(t *testing.T) {
	r := newRichEditor("<p>Hello <b>世界</b></p>")
	r.ensureEditor()
	r.editor.Cursor = 5
	r.editor.SelectStart, r.editor.SelectEnd = 5, 5
	r.editor.Text([]rune(" new"))
	if len(r.changes) != 1 {
		t.Fatal("typing burst not coalesced", len(r.changes))
	}
	r.sync()
	if string(r.doc.Text) != "Hello new 世界" || r.doc.FormatAt(len(r.doc.Text)-1).Style&richtext.Bold == 0 {
		t.Fatal(r.doc.HTML())
	}
	revision := r.doc.Revision
	if n := testing.AllocsPerRun(100, func() { r.sync() }); n != 0 {
		t.Fatal("unchanged sync allocated", n)
	}
	if r.doc.Revision != revision {
		t.Fatal("unchanged sync mutated document")
	}
	// A formatting paint cache refresh must not erase an unconsumed text edit.
	r.doc.Paragraph(0, len(r.doc.Text), 1, 0, false)
	r.editor.Cursor = len(r.editor.Buffer)
	r.editor.SelectStart, r.editor.SelectEnd = r.editor.Cursor, r.editor.Cursor
	r.editor.Paste("!")
	r.cache()
	r.sync()
	if !strings.HasSuffix(string(r.doc.Text), "!") {
		t.Fatal("cache dropped native edit")
	}
}

func TestV4RichSourceAndUndoStaySynchronized(t *testing.T) {
	r := newRichEditor("<p>original</p>")
	r.ensureEditor()
	r.editor.Cursor = len(r.editor.Buffer)
	r.editor.Paste(" changed")
	r.sync()
	r.undo(false)
	if string(r.doc.Text) != "original" || text(r.editor) != "original" {
		t.Fatal(r.doc.HTML(), text(r.editor))
	}
	r.sync()
	if string(r.doc.Text) != "original" {
		t.Fatal("native replacement re-applied after undo")
	}
	r.mode = "HTML"
	r.source = textEditor("<p>source <em>edit</em></p>", true)
	// A newly attached source with revision zero must still be read.
	r.source.SetText("<p>source <em>edit</em></p>")
	r.sync()
	if string(r.doc.Text) != "source edit" {
		t.Fatal(r.doc.HTML())
	}
	r.mode = "Edit"
	r.sync()
	if text(r.editor) != "source edit" {
		t.Fatal(text(r.editor))
	}
}
