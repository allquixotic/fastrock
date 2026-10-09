package desktop

// TextChange describes one mutation in rune coordinates. Inserted is only
// borrowed for the duration of OnChange; consumers retaining it must copy it.
type TextChange struct {
	Start, Removed int
	Inserted       []rune
}

// TrackChanges enables constant-time unchanged snapshots. Once enabled, update
// text through SetText, Text, Paste, Delete or undo/redo, rather than writing
// Buffer directly. Cursor/selection changes do not advance the text revision.
func (ed *TextEditor) TrackChanges()        { ed.trackedText = true; ed.snapshotValid = false }
func (ed *TextEditor) TextRevision() uint64 { return ed.textRevision }

// SetText replaces document content without changing selection or undo state.
func (ed *TextEditor) SetText(value string) {
	old := len(ed.Buffer)
	ed.Buffer = []rune(value)
	ed.changedText(0, old, ed.Buffer)
}

func (ed *TextEditor) changedText(start, removed int, inserted []rune) {
	if removed == 0 && len(inserted) == 0 {
		return
	}
	ed.textRevision++
	ed.snapshotValid = false
	if ed.OnChange != nil {
		ed.OnChange(TextChange{start, removed, inserted})
	}
}
