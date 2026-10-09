package ui

import (
	"bytes"
	"image"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/workspace"
	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/extension"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
)

type transcriptLine struct {
	Start, End int
	Height     int
	Code       bool
	Runs       []transcriptRun
	Table      bool
}
type transcriptRun struct {
	Text     string
	Start    int
	X, Width int
	Format   richtext.Format
}
type transcriptLayout struct {
	Text                string
	Plain               string
	Width, Size, Height int
	Lines               []transcriptLine
	Pending             bool
}

// Byte offsets are always UTF-8 boundaries from the rendered runs. Keeping a
// range instead of another editor avoids copying every displayed message.
type transcriptSelection struct {
	BlockID     string
	Layout      *transcriptLayout
	Anchor, End int
	Dragging    bool
}

func (s transcriptSelection) bounds() (int, int) {
	return min(s.Anchor, s.End), max(s.Anchor, s.End)
}
func (s transcriptSelection) text(id string) string {
	if s.BlockID != id || s.Layout == nil {
		return ""
	}
	start, end := s.bounds()
	return s.Layout.Plain[max(0, min(start, len(s.Layout.Plain))):max(0, min(end, len(s.Layout.Plain)))]
}
func transcriptHit(line transcriptLine, x int, face font.Face) int {
	for _, run := range line.Runs {
		if x < run.X {
			return run.Start
		}
		if x <= run.X+run.Width {
			previous := 0
			for i, r := range run.Text {
				end := i + utf8.RuneLen(r)
				width := nucular.FontWidth(face, run.Text[:end])
				if x-run.X < (previous+width)/2 {
					return run.Start + i
				}
				previous = width
			}
			return run.Start + len(run.Text)
		}
	}
	return line.End
}
func (a *App) transcriptSelectionKeys(w *nucular.Window, v *chatView) {
	s := &v.RichSelection
	for event := range w.Input().Keyboard.Events() {
		if event.HandleKey(key.CodeC, platform.PrimaryModifier()) {
			if value := s.text(s.BlockID); value != "" {
				a.copyText(value)
			}
		} else if event.HandleKey(key.CodeA, platform.PrimaryModifier()) && s.Layout != nil {
			s.Anchor, s.End = 0, len(s.Layout.Plain)
		}
	}
}

func prepareTranscript(source string, width, size int) *transcriptLayout {
	f, _ := font.NewFace(uiRegular, size)
	defer f.Face.Close()
	lineHeight := nucular.FontHeight(f) + 7
	l := &transcriptLayout{Text: source, Width: width, Size: size}
	var html bytes.Buffer
	md := goldmark.New(goldmark.WithExtensions(extension.GFM))
	_ = md.Convert([]byte(source), &html)
	doc := richtext.Parse(html.String())
	// The formatted document is temporary. Cache compact runs rather than a
	// per-rune format array in every transcript layout.
	ordered := 0
	for start := 0; start < len(doc.Text); {
		end := start
		for end < len(doc.Text) && doc.Text[end] != '\n' {
			end++
		}
		value := string(doc.Text[start:end])
		format := richtext.Format{}
		if start < end {
			format = doc.Marks[start]
		}
		columns := strings.Split(value, "\t")
		if len(columns) > 1 && columns[len(columns)-1] == "" {
			columns = columns[:len(columns)-1]
		}
		table := len(columns) > 1 && format.Style&richtext.Code == 0
		if !table {
			columns = []string{value}
		}
		available := width - 8
		if format.List > 0 || format.Quote {
			available -= 18
		}
		cellWidth := max(30, available/len(columns))
		rows := []transcriptLine{}
		columnStart := start
		for col, cell := range columns {
			position := columnStart
			for rowIndex, row := range nucular.WrapText(f, cell, max(20, cellWidth-10)) {
				for len(rows) <= rowIndex {
					rows = append(rows, transcriptLine{Height: lineHeight, Code: format.Style&richtext.Code != 0, Table: table})
				}
				n := utf8.RuneCountInString(row)
				runes := doc.Text[position : position+n]
				x := col*cellWidth + 4
				for i := 0; i < n; {
					mark := doc.Marks[position+i]
					j := i + 1
					for j < n && doc.Marks[position+j] == mark {
						j++
					}
					s := string(runes[i:j])
					w := nucular.FontWidth(f, s)
					rows[rowIndex].Runs = append(rows[rowIndex].Runs, transcriptRun{Text: s, X: x, Width: w, Format: mark})
					x += w
					i = j
				}
				position += n
				for position < end && unicode.IsSpace(doc.Text[position]) && doc.Text[position] != '\t' {
					position++
				}
			}
			columnStart += utf8.RuneCountInString(cell) + 1
		}
		if len(rows) == 0 {
			rows = append(rows, transcriptLine{Height: 8})
		}
		if format.List > 0 || format.Quote {
			marker := "• "
			if format.List == 2 {
				ordered++
				marker = strconv.Itoa(ordered) + ". "
			}
			if format.Quote {
				marker = "│ "
			}
			for ri := range rows {
				for i := range rows[ri].Runs {
					rows[ri].Runs[i].X += 18
				}
			}
			rows[0].Runs = append([]transcriptRun{{Text: marker, Width: 18, Format: format}}, rows[0].Runs...)
		}
		if format.List != 2 {
			ordered = 0
		}
		for _, row := range rows {
			l.Lines = append(l.Lines, row)
			l.Height += row.Height
		}
		start = end + 1
	}
	if len(l.Lines) == 0 {
		l.Lines = []transcriptLine{{Height: 8}}
		l.Height = 8
	}
	var plain strings.Builder
	for i := range l.Lines {
		line := &l.Lines[i]
		line.Start = plain.Len()
		right := 0
		for j := range line.Runs {
			run := &line.Runs[j]
			if j > 0 && run.X > right+2 {
				plain.WriteByte('\t')
			}
			run.Start = plain.Len()
			plain.WriteString(run.Text)
			right = run.X + run.Width
		}
		line.End = plain.Len()
		plain.WriteByte('\n')
	}
	l.Plain = plain.String()
	return l
}

func (a *App) drawTranscriptLine(w *nucular.Window, v *chatView, id string, layout *transcriptLayout, line transcriptLine) bool {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	if line.Code {
		out.FillRect(b, 3, a.p.Sunken)
	}
	if line.Table {
		out.StrokeLine(image.Pt(b.X, b.Y+b.H-1), image.Pt(b.X+b.W, b.Y+b.H-1), 1, a.p.Border)
	}
	in := w.Input()
	selection := &v.RichSelection
	if in.Mouse.IsClickDownInRect(mouse.ButtonLeft, b, true) {
		at := transcriptHit(line, in.Mouse.Buttons[mouse.ButtonLeft].ClickedPos.X-b.X, w.Master().Style().Font)
		*selection = transcriptSelection{BlockID: id, Layout: layout, Anchor: at, End: at, Dragging: true}
		v.Editor.Active = false
		v.Follow = false
	}
	if selection.BlockID == id && selection.Layout == layout && selection.Dragging && in.Mouse.HoveringRect(b) {
		selection.End = transcriptHit(line, in.Mouse.Pos.X-b.X, w.Master().Style().Font)
	}
	linkMenu := false
	face := w.Master().Style().Font
	for _, run := range line.Runs {
		q := rect.Rect{X: b.X + run.X, Y: b.Y, W: run.Width + 2, H: b.H}
		fg := a.p.Text
		f := face
		if run.Format.Link != "" {
			fg = a.p.Accent
		}
		if run.Format.Style&richtext.Italic != 0 {
			f = a.italicFace
		}
		if selection.BlockID == id && selection.Layout == layout {
			start, end := selection.bounds()
			lo, hi := max(start, run.Start)-run.Start, min(end, run.Start+len(run.Text))-run.Start
			if lo < hi {
				left := nucular.FontWidth(face, run.Text[:lo])
				width := nucular.FontWidth(face, run.Text[lo:hi])
				out.FillRect(rect.Rect{X: q.X + left, Y: q.Y, W: width, H: q.H}, 2, a.p.Selected)
			}
		}
		out.DrawText(q, run.Text, f, fg)
		if run.Format.Style&richtext.Bold != 0 || run.Format.Heading > 0 {
			q.X++
			out.DrawText(q, run.Text, f, fg)
			q.X--
		}
		if run.Format.Style&richtext.Strike != 0 {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H/2), image.Pt(q.X+run.Width, q.Y+q.H/2), 1, fg)
		}
		if run.Format.Link != "" {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H-3), image.Pt(q.X+run.Width, q.Y+q.H-3), 1, fg)
			if in.Mouse.Clicked(mouse.ButtonLeft, q) && selection.Anchor == selection.End {
				a.openLink(run.Format.Link)
			}
			if menu := w.ContextualOpen(0, image.Pt(210, 80), q, nil); menu != nil {
				linkMenu = true
				if menu.MenuItem(label.T("Open link")) {
					a.openLink(run.Format.Link)
				}
				if menu.MenuItem(label.T("Copy link")) {
					a.copyText(run.Format.Link)
				}
			}
		}
	}
	return linkMenu
}
func (a *App) transcriptLayout(v *chatView, b workspace.Block, width int) *transcriptLayout {
	if v.Layouts == nil {
		v.Layouts = map[string]*transcriptLayout{}
	}
	source := cut(b.Text, 60000)
	size := a.prefs.FontSize
	old := v.Layouts[b.ID]
	if old != nil && old.Text == source && old.Width == width && old.Size == size {
		return old
	}
	if old != nil && old.Pending {
		return old
	}
	placeholder := &transcriptLayout{Text: source, Width: width, Size: size, Height: 48, Pending: true}
	if old != nil {
		placeholder.Height = old.Height
		placeholder.Lines = old.Lines
	}
	job := func() {
		result := prepareTranscript(source, width, size)
		a.post(func() {
			if v.Layouts[b.ID] == placeholder {
				v.Layouts[b.ID] = result
			}
		})
	}
	select {
	case a.layoutJobs <- job:
		v.Layouts[b.ID] = placeholder
	default:
		a.window.Changed()
		if old != nil {
			return old
		}
	}
	return placeholder
}
func (a *App) transcriptMenu(w *nucular.Window, c *workspace.Conversation, v *chatView, b workspace.Block) {
	if menu := w.ContextualOpen(0, image.Pt(230, 250), w.LastWidgetBounds, nil); menu != nil {
		value := b.Text
		if selection := v.RichSelection.text(b.ID); selection != "" {
			value = selection
		}
		if v.SelectID == b.ID && v.Selection != nil {
			start, end := v.Selection.SelectStart, v.Selection.SelectEnd
			if start > end {
				start, end = end, start
			}
			if start < end {
				value = string(v.Selection.Buffer[start:end])
			}
		}
		if menu.MenuItem(label.T("Select message text")) {
			v.SelectID = b.ID
			v.Selection = textEditor(b.Text, true)
			v.Selection.Flags |= nucular.EditReadOnly
		}
		if menu.MenuItem(label.T("Copy message / selection")) {
			a.copyText(value)
		}
		if menu.MenuItem(label.T("Quote in prompt")) {
			setText(v.Editor, text(v.Editor)+"\n> "+strings.ReplaceAll(value, "\n", "\n> ")+"\n")
		}
		if menu.MenuItem(label.T("Forward to conversation…")) {
			a.chooseConversation(func(target *workspace.Conversation) {
				a.resumeThread(target.ID)
				if cv := a.chats[target.ID]; cv != nil {
					setText(cv.Editor, value)
				}
			})
		}
		if menu.MenuItem(label.T("View as text")) {
			a.openText(b.Role+" · "+c.Title, b.Text)
		}
		if menu.MenuItem(label.T("Expand / collapse")) {
			v.Expanded[b.ID] = !v.Expanded[b.ID]
		}
		if b.Role == "changes" && menu.MenuItem(label.T("Open changes")) {
			a.showDiff(c)
		}
	}
}
func (a *App) chooseConversation(pick func(*workspace.Conversation)) {
	a.window.PopupOpen("Choose conversation", nucular.WindowTitle|nucular.WindowClosable, dialogBounds(), true, func(w *nucular.Window) {
		for _, c := range a.state.Sidebar("", false) {
			w.Row(30).Dynamic(1)
			if w.ButtonText(c.Title) {
				pick(c)
				w.Close()
			}
		}
	})
}

func dialogBounds() rect.Rect { return rect.Rect{X: 320, Y: 140, W: 600, H: 500} }
