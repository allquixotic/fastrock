package ui

import (
	"bytes"
	"image"
	"sort"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/workspace"
	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/extension"
	"github.com/yuin/goldmark/renderer"
	gmtext "github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"
	"golang.org/x/image/math/fixed"
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
	Face     font.Face
	Advances []int
	Offsets  []int
}
type transcriptLayout struct {
	Cwd                 string
	Markup              string
	Text                string
	Plain               string
	Width, Size, Height int
	Lines               []transcriptLine
	Pending             bool
	Used                uint64
	LineOffsets         []int
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
			if len(run.Advances) == 0 {
				if run.Face.Face != nil {
					face = run.Face
				}
				measureTranscriptRun(&run, face)
			}
			i := sort.Search(len(run.Advances)-1, func(i int) bool { return x-run.X < (run.Advances[i]+run.Advances[i+1])/2 })
			return run.Start + run.Offsets[i]
		}
	}
	return line.End
}
func measureTranscriptRun(r *transcriptRun, f font.Face) {
	r.Advances = []int{0}
	r.Offsets = []int{0}
	var advance fixed.Int26_6
	previous := rune(-1)
	for i, ch := range r.Text {
		if previous >= 0 {
			advance += f.Face.Kern(previous, ch)
		}
		a, _ := f.Face.GlyphAdvance(ch)
		advance += a
		r.Advances = append(r.Advances, advance.Ceil())
		r.Offsets = append(r.Offsets, i+utf8.RuneLen(ch))
		previous = ch
	}
}

func borrowTranscriptFace(size int) (font.Face, func()) {
	return typeFace(size, regularFont), func() {}
}
func borrowLayoutFace(size int, mono bool) (font.Face, func()) {
	variant := regularFont
	if mono {
		variant = monoFont
	}
	return typeFace(size, variant), func() {}
}
func (a *App) transcriptSelectionKeys(w *desktop.Window, v *chatView) {
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
	return prepareMarkdownTranscript(source, width, size, true)
}
func prepareMarkdownTranscript(source string, width, size int, directives bool) *transcriptLayout {
	display := source
	if directives {
		display = visibleTranscriptMarkdown(source)
	}
	var html bytes.Buffer
	md := goldmark.New(goldmark.WithExtensions(extension.GFM), goldmark.WithRendererOptions(renderer.WithNodeRenderers(util.Prioritized(transcriptLinkRenderer{}, 500))))
	data := []byte(display)
	doc := md.Parser().Parse(gmtext.NewReader(data))
	addTranscriptAutolinks(doc, data)
	_ = md.Renderer().Render(&html, data, doc)
	return prepareDocumentTranscript(richtext.ParseWithLinks(html.String(), transcriptSafeLink), source, width, size)
}
func prepareDocumentTranscript(doc *richtext.Document, source string, width, size int) *transcriptLayout {
	f, release := borrowTranscriptFace(size)
	defer release()
	lineHeight := desktop.FontHeight(f) + 7
	l := &transcriptLayout{Text: source, Width: width, Size: size}
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
			format, _ = doc.RunAt(start)
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
			cellEnd := columnStart + utf8.RuneCountInString(cell)
			cellRows := wrapTranscriptCell(doc, columnStart, cellEnd, max(20, cellWidth-10), size)
			for rowIndex, row := range cellRows {
				for len(rows) <= rowIndex {
					rows = append(rows, transcriptLine{Height: lineHeight, Code: format.Style&richtext.Code != 0, Table: table})
				}
				rows[rowIndex].Height = max(rows[rowIndex].Height, row.Height)
				for _, run := range row.Runs {
					run.X += col * cellWidth
					rows[rowIndex].Runs = append(rows[rowIndex].Runs, run)
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
		l.LineOffsets = append(l.LineOffsets, 0)
		if i > 0 {
			l.LineOffsets[i] = l.LineOffsets[i-1] + l.Lines[i-1].Height
		}
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

func (a *App) drawTranscriptLine(w *desktop.Window, v *chatView, id string, layout *transcriptLayout, line transcriptLine) bool {
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
	if line.Code {
		face = a.monoFace
	}
	for _, run := range line.Runs {
		q := rect.Rect{X: b.X + run.X, Y: b.Y, W: run.Width + 2, H: b.H}
		fg := a.p.Text
		f := run.Face
		if f.Face == nil {
			f = face
		}
		if run.Format.Link != "" {
			fg = a.p.Accent
		}
		if run.Format.Style&richtext.Code != 0 && !line.Code {
			out.FillRect(q, 2, a.p.Sunken)
		}
		if selection.BlockID == id && selection.Layout == layout {
			start, end := selection.bounds()
			lo, hi := max(start, run.Start)-run.Start, min(end, run.Start+len(run.Text))-run.Start
			if lo < hi {
				left := transcriptAdvance(run, lo)
				width := transcriptAdvance(run, hi) - left
				out.FillRect(rect.Rect{X: q.X + left, Y: q.Y, W: width, H: q.H}, 2, a.p.Selected)
			}
		}
		out.DrawText(q, run.Text, f, fg)
		if run.Format.Style&richtext.Strike != 0 {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H/2), image.Pt(q.X+run.Width, q.Y+q.H/2), 1, fg)
		}
		if run.Format.Link != "" {
			out.StrokeLine(image.Pt(q.X, q.Y+q.H-3), image.Pt(q.X+run.Width, q.Y+q.H-3), 1, fg)
			if in.Mouse.HoveringRect(q) {
				w.Tooltip(run.Format.Link)
			}
			if in.Mouse.Clicked(mouse.ButtonLeft, q) && selection.Anchor == selection.End {
				a.openLinkAt(run.Format.Link, layout.Cwd)
			}
			if menu := w.ContextualOpen(0, image.Pt(210, 80), q, nil); menu != nil {
				linkMenu = true
				if menu.MenuItem(label.T("Open link")) {
					a.openLinkAt(run.Format.Link, layout.Cwd)
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
	source := b.Text
	if len(source) > 60000 {
		end := 60000
		for end > 0 && !utf8.RuneStart(source[end]) {
			end--
		}
		source = source[:end] + "\n[Display truncated; open message as text for the retained content.]"
	}
	size := a.prefs.FontSize
	cwd := fallback(v.Cwd, a.prefs.WorkingDirectory)
	markup := "markdown"
	if b.Role == "tool" || b.Role == "changes" || b.Role == "activity" || b.Kind == "crossTabMessage" {
		markup = "literal"
	} else if b.Role == "assistant" || b.Role == "recap" {
		markup = "directives"
	}
	old := v.Layouts[b.ID]
	v.LayoutClock++
	if old != nil {
		old.Used = v.LayoutClock
	}
	if old != nil && old.Text == source && old.Width == width && old.Size == size && old.Cwd == cwd && old.Markup == markup {
		return old
	}
	if old != nil && old.Pending {
		return old
	}
	placeholder := &transcriptLayout{Text: source, Cwd: cwd, Markup: markup, Width: width, Size: size, Height: 48, Pending: true}
	if old != nil && old.Cwd == cwd && old.Markup == markup {
		placeholder.Height = old.Height
		placeholder.Lines = old.Lines
	}
	job := func() {
		var result *transcriptLayout
		if markup == "literal" {
			result = prepareLiteralTranscript(source, width, size)
		} else {
			result = prepareMarkdownTranscript(source, width, size, markup == "directives")
		}
		result.Cwd = cwd
		result.Markup = markup
		a.post(func() {
			if v.Layouts[b.ID] == placeholder {
				result.Used = placeholder.Used
				v.Layouts[b.ID] = result
				v.boundLayouts(b.ID)
			}
		})
	}
	select {
	case a.layoutJobs <- job:
		placeholder.Used = v.LayoutClock
		v.Layouts[b.ID] = placeholder
		v.boundLayouts(b.ID)
	default:
		a.window.Changed()
		if old != nil {
			return old
		}
	}
	return placeholder
}
func (a *App) transcriptMenu(w *desktop.Window, c *workspace.Conversation, v *chatView, b workspace.Block) {
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
			v.Selection.Flags |= desktop.EditReadOnly
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
				target.Enqueue("Forwarded from "+c.Title+":\n\n"+value, nil)
				target.QueuePaused = true
				a.toast = "Forwarded message added to the paused queue"
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

// Tool output is literal text. In particular shell metacharacters and angle
// brackets must never be interpreted as Markdown or HTML.
func prepareLiteralTranscript(source string, width, size int) *transcriptLayout {
	f, release := borrowLayoutFace(size-1, true)
	defer release()
	plain := strings.ReplaceAll(source, "\t", "    ")
	l := &transcriptLayout{Text: source, Plain: plain, Width: width, Size: size}
	height := desktop.FontHeight(f) + 7
	start := 0
	appendLine := func(end int) {
		run := transcriptRun{Text: plain[start:end], Start: start, X: 4, Face: f, Format: richtext.Format{Style: richtext.Code}}
		measureTranscriptRun(&run, f)
		run.Width = run.Advances[len(run.Advances)-1]
		l.LineOffsets = append(l.LineOffsets, l.Height)
		l.Lines = append(l.Lines, transcriptLine{Start: start, End: end, Height: height, Code: true, Runs: []transcriptRun{run}})
		l.Height += height
	}
	lineWidth := 0
	for offset, r := range plain {
		if r == '\n' {
			appendLine(offset)
			start = offset + 1
			lineWidth = 0
			continue
		}
		advance, _ := f.Face.GlyphAdvance(r)
		if lineWidth+advance.Ceil() > max(20, width-8) && offset > start {
			appendLine(offset)
			start = offset
			lineWidth = 0
		}
		lineWidth += advance.Ceil()
	}
	appendLine(len(plain))
	return l
}

func transcriptAdvance(run transcriptRun, offset int) int {
	i := sort.SearchInts(run.Offsets, offset)
	if i < len(run.Advances) {
		return run.Advances[i]
	}
	return run.Width
}
