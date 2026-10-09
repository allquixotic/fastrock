package ui

import (
	"fmt"
	"image"
	"image/color"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/richtext"
	"golang.org/x/image/font/gofont/goitalic"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
)

type richEditor struct {
	doc                            *richtext.Document
	editor, source                 *nucular.TextEditor
	mode                           string
	content                        string
	offsets                        []int
	widths                         map[[2]int]int
	italic                         font.Face
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
	r := &richEditor{doc: richtext.Parse(html), source: textEditor(html, true), mode: "Edit"}
	r.editor = textEditor(string(r.doc.Text), true)
	r.editor.PaintText = r.paint
	r.cache()
	return r
}
func (r *richEditor) cache() {
	r.revision = r.doc.Revision
	r.content = string(r.doc.Text)
	r.offsets = r.offsets[:0]
	for i := range r.content {
		r.offsets = append(r.offsets, i)
	}
	r.offsets = append(r.offsets, len(r.content))
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
		f := r.doc.Marks[i]
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
func (r *richEditor) sync() {
	if r.mode == "HTML" {
		if value := text(r.source); value != r.doc.HTML() {
			r.doc = richtext.Parse(value)
			setText(r.editor, string(r.doc.Text))
			r.cache()
		}
	} else if r.doc.Sync(r.editor.Buffer) {
		r.cache()
	}
}
func (r *richEditor) html() string          { r.sync(); return r.doc.HTML() }
func (r *richEditor) selection() (int, int) { return r.editor.SelectStart, r.editor.SelectEnd }
func (r *richEditor) toggle(s richtext.Style) {
	r.doc.Toggle(r.selectionStart(), r.selectionEnd(), s)
	r.editor.Redraw = true
}
func (r *richEditor) selectionStart() int { return min(r.editor.SelectStart, r.editor.SelectEnd) }
func (r *richEditor) selectionEnd() int   { return max(r.editor.SelectStart, r.editor.SelectEnd) }
func (r *richEditor) undo(redo bool) {
	if r.doc.Undo(redo) {
		setText(r.editor, string(r.doc.Text))
		r.cache()
	}
}
func (a *App) richField(w *nucular.Window, name string, r *richEditor, height int) {
	if name == "AcceptanceCriteria" {
		name = "Acceptance criteria"
	}
	if r == nil {
		return
	}
	r.sync()
	if r.revision != r.doc.Revision {
		r.cache()
	}
	r.accent = a.p.Accent
	if r.fontSize != a.prefs.FontSize {
		r.fontSize = a.prefs.FontSize
		r.italic, _ = font.NewFace(goitalic.TTF, r.fontSize)
		clear(r.widths)
	}
	w.Row(28).Static(max(120, w.LayoutAvailableWidth()-196), 60, 70, 62)
	w.LabelColored(name, "LC", a.p.Text)
	for _, mode := range []string{"Edit", "Preview", "HTML"} {
		if flatRow(w, mode, "", r.mode == mode, color.RGBA{}, a.p) && r.mode != mode {
			if mode == "HTML" {
				setText(r.source, r.doc.HTML())
			}
			r.mode = mode
		}
	}
	if r.mode == "HTML" {
		w.Row(height).Dynamic(1)
		r.source.Edit(w)
		r.sync()
		return
	}
	if r.mode == "Edit" {
		w.Row(32).Static(30, 30, 30, 30, 36, 30, 30, 44, 30, 30, 116)
		f := r.doc.FormatAt(r.editor.Cursor)
		for _, t := range []struct {
			label string
			style richtext.Style
		}{{"B", richtext.Bold}, {"I", richtext.Italic}, {"U", richtext.Underline}, {"S", richtext.Strike}, {"</>", richtext.Code}} {
			if formatButton(w, t.label, "Toggle formatting", f.Style&t.style != 0, a.p) {
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
		if formatButton(w, "Link", "Edit selected link", f.Link != "", a.p) {
			start, end := r.selection()
			a.inputDialog("Link URL (select text first)", f.Link, func(link string) { r.doc.Link(start, end, link); r.editor.Redraw = true })
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
		r.editor.Flags |= nucular.EditReadOnly
	} else {
		r.editor.Flags &^= nucular.EditReadOnly
	}
	w.Row(height).Dynamic(1)
	r.links = r.links[:0]
	beforeEdit := r.doc.Revision
	r.editor.Edit(w)
	w.Master().Style().Edit = old
	if r.doc.Sync(r.editor.Buffer) {
		r.cache()
		r.editor.Redraw = true
	} else if r.doc.Revision == beforeEdit && (r.editor.Cursor != r.lastCursor || r.editor.SelectStart != r.lastStart || r.editor.SelectEnd != r.lastEnd) {
		r.doc.ClearPending()
	}
	r.lastCursor, r.lastStart, r.lastEnd = r.editor.Cursor, r.editor.SelectStart, r.editor.SelectEnd
	if r.mode == "Preview" {
		for _, link := range r.links {
			if w.Input().Mouse.Clicked(mouse.ButtonLeft, link.bounds) && (strings.HasPrefix(link.url, "https://") || strings.HasPrefix(link.url, "http://")) {
				a.openURL(link.url)
			}
		}
	}
}

// Formatted runs keep the native editor's advances, caret and hit-test coordinates.
// Bold is an overstrike; italic and decorations share those same advances.
func (r *richEditor) paint(out *command.Buffer, b rect.Rect, text []rune, start int, face font.Face, fg color.RGBA, selected bool) {
	if r.doc.Sync(r.editor.Buffer) {
		r.cache()
	}
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
		f := r.doc.Marks[i]
		j := i + 1
		for j < end && r.doc.Marks[j] == f {
			j++
		}
		s := r.content[r.offsets[i]:r.offsets[j]]
		k := [2]int{i, j}
		width, ok := r.widths[k]
		if !ok {
			width = nucular.FontWidth(face, s)
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
		fontFace := face
		if f.Style&richtext.Italic != 0 {
			fontFace = r.italic
		}
		out.DrawText(q, s, fontFace, c)
		if f.Style&richtext.Bold != 0 || f.Heading > 0 {
			q.X++
			out.DrawText(q, s, fontFace, c)
			q.X--
		}
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
