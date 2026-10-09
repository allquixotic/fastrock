package ui

import (
	"fmt"
	"image"
	"image/color"
	"net/url"
	"slices"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/richtext"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
)

type richEditor struct {
	presentation                   richPresentation
	editorRevision, sourceRevision uint64
	changes                        []desktop.TextChange
	doc                            *richtext.Document
	editor, source                 *desktop.TextEditor
	mode                           string
	content                        string
	offsets                        []int
	widths                         map[[2]int]int
	fontSize                       int
	accent                         color.RGBA
	markers                        map[int]string
	revision                       uint64
	links                          []richLink
	lastCursor, lastStart, lastEnd int
}

type richLink struct {
	bounds rect.Rect
	url    string
}

func newRichEditor(html string) *richEditor {
	r := &richEditor{doc: richtext.Parse(html), mode: "Edit"}
	if len(r.doc.Unsupported) > 0 {
		r.mode = "HTML"
	}
	return r
}

// Unopened fields retain only the document and original HTML, without native
// edit buffers, UTF-8 paint copies or per-rune byte-offset tables.
func (r *richEditor) ensureEditor() {
	if r.editor != nil {
		return
	}
	r.editor = textEditor(string(r.doc.Text), true)
	r.editor.OnChange = r.changed
	r.editor.PaintText = r.paint
	r.editor.MeasureText = r.measure
	r.presentation.Position.apply(r.editor)
	if r.mode == "HTML" && r.source == nil {
		r.source = textEditor(r.doc.HTML(), true)
		r.presentation.Source.apply(r.source)
	}
	r.synchronized()
	r.cache()
}
func (r *richEditor) cache() {
	r.revision = r.doc.Revision
	r.content = string(r.doc.Text)
	r.offsets = r.offsets[:0]
	for i := range r.content {
		r.offsets = append(r.offsets, i)
	}
	r.offsets = append(r.offsets, len(r.content))
	if r.editor != nil && r.fontSize > 0 {
		r.editor.MinRowHeight = desktop.FontHeight(typeFace(r.fontSize, regularFont))
		for i := 0; i < len(r.doc.Text); {
			format, next := r.doc.RunAt(i)
			r.editor.MinRowHeight = max(r.editor.MinRowHeight, desktop.FontHeight(drawFace(r.fontSize, format)))
			i = next
		}
	}
	clear(r.widths)
	if r.widths == nil {
		r.widths = make(map[[2]int]int)
	}
	if r.markers == nil {
		r.markers = make(map[int]string)
	} else {
		clear(r.markers)
	}
	count := 0
	for i, ch := range r.doc.Text {
		if i > 0 && r.doc.Text[i-1] != '\n' {
			continue
		}
		if ch == '\n' {
			continue
		}
		f, _ := r.doc.RunAt(i)
		if f.List != 2 {
			count = 0
		}
		switch {
		case f.List == 1:
			r.markers[i] = "•"
		case f.List == 2:
			count++
			r.markers[i] = fmt.Sprintf("%d.", count)
		case f.Quote:
			r.markers[i] = "│"
		}
	}
}
func (r *richEditor) synchronized() {
	r.changes = nil
	if r.editor != nil {
		r.editorRevision = r.editor.TextRevision()
	}
	if r.source != nil {
		r.sourceRevision = r.source.TextRevision()
	}
}
func (r *richEditor) sync() {
	if r.editor == nil {
		return
	}
	if r.mode == "HTML" {
		if r.source == nil || r.sourceRevision == r.source.TextRevision() {
			return
		}
		if value := text(r.source); value != r.doc.HTML() {
			r.doc = richtext.Parse(value)
			setText(r.editor, string(r.doc.Text))
			r.synchronized()
			r.cache()
		}
		r.sourceRevision = r.source.TextRevision()
	} else if r.editorRevision != r.editor.TextRevision() {
		changed := false
		for _, edit := range r.changes {
			changed = r.doc.ApplyEdit(edit.Start, edit.Removed, edit.Inserted) || changed
		}
		if len(r.changes) == 0 {
			changed = r.doc.Sync(r.editor.Buffer)
		}
		if changed {
			r.synchronized()
			r.cache()
		} else {
			r.synchronized()
		}
	}
}

func (r *richEditor) changed(edit desktop.TextChange) {
	// Keyboard text arrives rune by rune; merge an adjacent typing burst before
	// updating the rich document so its unchanged tail moves only once.
	if n := len(r.changes); n > 0 && edit.Removed == 0 {
		last := &r.changes[n-1]
		if last.Removed == 0 && last.Start+len(last.Inserted) == edit.Start {
			last.Inserted = append(last.Inserted, edit.Inserted...)
			return
		}
	}
	edit.Inserted = slices.Clone(edit.Inserted)
	r.changes = append(r.changes, edit)
}
func (r *richEditor) html() string { r.sync(); return r.doc.HTML() }
func (r *richEditor) toggle(s richtext.Style) {
	r.ensureEditor()
	r.doc.Toggle(r.selectionStart(), r.selectionEnd(), s)
	r.editor.Redraw = true
}
func (r *richEditor) selectionStart() int { return min(r.editor.SelectStart, r.editor.SelectEnd) }
func (r *richEditor) selectionEnd() int   { return max(r.editor.SelectStart, r.editor.SelectEnd) }
func (r *richEditor) undo(redo bool) {
	r.ensureEditor()
	r.sync()
	if r.doc.Undo(redo) {
		setText(r.editor, string(r.doc.Text))
		r.synchronized()
		r.cache()
	}
}
func (a *App) richField(w *desktop.Window, name string, r *richEditor, height int) {
	if name == "AcceptanceCriteria" {
		name = "Acceptance criteria"
	}
	if r == nil {
		return
	}
	r.ensureEditor()
	r.sync()
	if len(r.doc.Unsupported) > 0 {
		w.Row(48).Dynamic(1)
		w.LabelWrap("This field contains unsupported HTML (" + strings.Join(r.doc.Unsupported, ", ") + "). HTML mode preserves it. Formatted edits may lose those structures.")
	}
	if r.revision != r.doc.Revision {
		r.cache()
	}
	r.accent = a.p.Accent
	if r.fontSize != a.prefs.FontSize {
		r.fontSize = a.prefs.FontSize
		r.cache()
	}
	scale := w.Master().Style().Scaling
	modeWidths := []int{}
	remaining := w.LayoutAvailableWidth() - 3*w.WindowStyle().Spacing.X
	for _, mode := range []string{"Edit", "Preview", "HTML"} {
		width := desktop.FontWidth(w.Master().Style().Font, mode) + int(24*scale)
		modeWidths = append(modeWidths, width)
		remaining -= width
	}
	w.RowScaled(int(28*scale)).StaticScaled(max(int(40*scale), remaining), modeWidths[0], modeWidths[1], modeWidths[2])
	w.LabelColored(name, "LC", a.p.Text)
	for _, mode := range []string{"Edit", "Preview", "HTML"} {
		if flatRow(w, mode, "", r.mode == mode, color.RGBA{}, a.p) && r.mode != mode {
			if mode == "HTML" {
				if r.source == nil {
					r.source = textEditor("", true)
				}
				setText(r.source, r.doc.HTML())
			}
			r.mode = mode
		}
	}
	if r.mode == "HTML" {
		w.Row(height).Dynamic(1)
		codeEditor(w, r.source)
		r.sync()
		return
	}
	if r.mode == "Edit" {
		compact := w.LayoutAvailableWidth() < int(480*w.Master().Style().Scaling)
		if compact {
			w.Row(32).Static(30, 30, 30, 30, 36, 30, 30)
		} else {
			w.Row(32).Static(30, 30, 30, 30, 36, 30, 30, 44, 30, 30, 116)
		}
		f := r.doc.FormatAt(r.editor.Cursor)
		for _, t := range []struct {
			label string
			style richtext.Style
		}{{"B", richtext.Bold}, {"I", richtext.Italic}, {"U", richtext.Underline}, {"S", richtext.Strike}, {"</>", richtext.Code}} {
			if formatButton(w, t.label, map[string]string{"B": "Bold (Ctrl/Cmd+B)", "I": "Italic (Ctrl/Cmd+I)", "U": "Underline (Ctrl/Cmd+U)", "S": "Strikethrough", "</>": "Code"}[t.label], f.Style&t.style != 0, a.p) {
				r.toggle(t.style)
			}
		}
		if formatButton(w, "•", "Bulleted list", f.List == 1, a.p) {
			l := uint8(1)
			if f.List == 1 {
				l = 0
			}
			r.doc.Paragraph(r.selectionStart(), r.selectionEnd(), 0, l, false)
		}
		if formatButton(w, "1.", "Numbered list", f.List == 2, a.p) {
			l := uint8(2)
			if f.List == 2 {
				l = 0
			}
			r.doc.Paragraph(r.selectionStart(), r.selectionEnd(), 0, l, false)
		}
		if compact {
			w.Row(32).Static(44, 30, 30, 116)
		}
		if formatButton(w, "Link", "Edit selected link", f.Link != "", a.p) {
			a.editRichLink(r)
		}
		if formatButton(w, "undo", "Undo", false, a.p) {
			r.undo(false)
		}
		if formatButton(w, "redo", "Redo", false, a.p) {
			r.undo(true)
		}
		headings := []string{"Paragraph", "Heading 1", "Heading 2", "Heading 3", "Quote"}
		idx := int(f.Heading)
		if f.Quote {
			idx = 4
		}
		idx = min(idx, 4)
		if next := w.ComboSimple(headings, idx, 28); next != idx {
			h := uint8(next)
			if next == 4 {
				h = 0
			}
			r.doc.Paragraph(r.selectionStart(), r.selectionEnd(), h, 0, next == 4)
		}
		if r.editor.Active {
			for e := range w.Input().Keyboard.Events() {
				switch {
				case e.HandleKeyModmask(key.CodeK, key.ModControl|key.ModMeta):
					a.editRichLink(r)
				case e.HandleKeyModmask(key.CodeB, key.ModControl|key.ModMeta):
					r.toggle(richtext.Bold)
				case e.HandleKeyModmask(key.CodeI, key.ModControl|key.ModMeta):
					r.toggle(richtext.Italic)
				case e.HandleKeyModmask(key.CodeU, key.ModControl|key.ModMeta):
					r.toggle(richtext.Underline)
				case e.HandleKeyModmask(key.CodeZ, key.ModControl|key.ModMeta):
					r.undo(e.Key().Modifiers&key.ModShift != 0)
				case e.HandleKeyModmask(key.CodeY, key.ModControl|key.ModMeta):
					r.undo(true)
				}
			}
		}
	}
	old := w.Master().Style().Edit
	es := old
	es.Padding.X = 24
	es.Padding.Y = 12
	w.Master().Style().Edit = es
	if r.mode == "Preview" {
		r.editor.Flags |= desktop.EditReadOnly
	} else {
		r.editor.Flags &^= desktop.EditReadOnly
	}
	w.Row(height).Dynamic(1)
	r.links = r.links[:0]
	beforeEdit := r.doc.Revision
	r.editor.Edit(w)
	w.Master().Style().Edit = old
	r.sync()
	if r.doc.Revision != beforeEdit {
		r.editor.Redraw = true
	} else if r.doc.Revision == beforeEdit && (r.editor.Cursor != r.lastCursor || r.editor.SelectStart != r.lastStart || r.editor.SelectEnd != r.lastEnd) {
		r.doc.ClearPending()
	}
	r.lastCursor, r.lastStart, r.lastEnd = r.editor.Cursor, r.editor.SelectStart, r.editor.SelectEnd
	if r.mode == "Preview" {
		for _, link := range r.links {
			if w.Input().Mouse.Clicked(mouse.ButtonLeft, link.bounds) && validRichLink(link.url) {
				a.openURL(link.url)
			}
		}
	}
}

func validRichLink(value string) bool {
	if value == "" {
		return true
	} // Removing a link preserves the selected text.
	u, err := url.Parse(value)
	if err != nil || u.User != nil {
		return false
	}
	switch u.Scheme {
	case "http", "https":
		return u.Host != ""
	case "mailto":
		return u.Opaque != ""
	}
	return false
}

func (a *App) editRichLink(r *richEditor) {
	start, end := r.selectionStart(), r.selectionEnd()
	if start == end {
		a.toast = "Select the text to link or unlink first"
		return
	}
	a.inputDialog("Link URL (empty removes link)", r.doc.FormatAt(start).Link, func(value string) {
		value = strings.TrimSpace(value)
		if !validRichLink(value) {
			a.toast = "Use an http, https or mailto URL"
			return
		}
		r.doc.Link(start, end, value)
		r.editor.Redraw = true
	})
}

// Formatting shares faces with native measurement, caret and hit testing.
func (r *richEditor) paint(out *command.Buffer, b rect.Rect, text []rune, start int, face font.Face, fg color.RGBA, selected bool) {
	r.sync()
	if r.revision != r.doc.Revision {
		r.cache()
	}
	end := min(start+len(text), len(r.doc.Text))
	if start >= end {
		return
	}
	if start == 0 || r.doc.Text[start-1] == '\n' {
		label := r.markers[start]
		if label != "" {
			clip := out.Clip
			expanded := clip
			expanded.X -= 18
			expanded.W += 18
			out.PushScissor(expanded)
			q := b
			q.X -= 17
			q.W = 16
			out.DrawText(q, label, face, fg)
			out.PushScissor(clip)
		}
	}
	for i := start; i < end; {
		f, next := r.doc.RunAt(i)
		j := min(end, next)
		s := r.content[r.offsets[i]:r.offsets[j]]
		k := [2]int{i, j}
		width, ok := r.widths[k]
		if !ok {
			width = desktop.FontWidth(r.face(f), s)
			r.widths[k] = width
		}
		q := b
		q.W = width + 3
		if f.Link != "" {
			r.links = append(r.links, richLink{q, f.Link})
		}
		c := fg
		if f.Link != "" && !selected {
			c = r.accent
		}
		fontFace := r.face(f)
		out.DrawText(q, s, fontFace, c)
		if f.Style&richtext.Underline != 0 || f.Link != "" {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H-2), image.Pt(q.X+width, q.Y+q.H-2), 1, c)
		}
		if f.Style&richtext.Strike != 0 {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H/2), image.Pt(q.X+width, q.Y+q.H/2), 1, c)
		}
		if f.Style&richtext.Code != 0 {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H-1), image.Pt(q.X+width, q.Y+q.H-1), 1, r.accent)
		}
		b.X += width
		i = j
	}
}

func plainHTML(s string) string { return strings.TrimSpace(string(richtext.Parse(s).Text)) }

func (r *richEditor) face(format richtext.Format) font.Face {
	size := r.fontSize
	if size <= 0 {
		size = 13
	}
	return drawFace(size, format)
}
func (r *richEditor) measure(text []rune, start int, base font.Face) int {
	r.sync()
	if r.revision != r.doc.Revision {
		r.cache()
	}
	end := min(start+len(text), len(r.doc.Text))
	width := 0
	for i := start; i < end; {
		format, next := r.doc.RunAt(i)
		j := min(end, next)
		width += desktop.FontWidth(r.face(format), r.content[r.offsets[i]:r.offsets[j]])
		i = j
	}
	return width
}
