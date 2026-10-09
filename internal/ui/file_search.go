package ui

import (
	"context"
	"fmt"
	"image/color"
	"sort"
	"strconv"
	"unicode"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"golang.org/x/image/math/fixed"
	"golang.org/x/mobile/event/key"
)

type fileAnalysis struct {
	lines, matches []uint32
	bytes, runes   int
	matchLength    int
}

type fileSearch struct {
	editor     *desktop.TextEditor
	revision   uint64
	query      string
	generation uint64
	pending    bool
	cancel     context.CancelFunc
	result     *fileAnalysis
	moves      int
	err        string
	paint      *fileAnalysis
	palette    palette
	paintScale float64
	paintOpen  bool
}

func (v *fileView) resetFileSearch() {
	if v.Search.cancel != nil {
		v.Search.cancel()
	}
	generation := v.Search.generation + 1
	v.Search = fileSearch{generation: generation}
	if v.Editor != nil {
		v.Editor.PaintText, v.Editor.PaintTabText, v.Editor.PaintGutter, v.Editor.GutterWidth = nil, nil, nil, 0
	}
}

func foldFileRune(r rune) rune {
	minimum := r
	for n := unicode.SimpleFold(r); n != r; n = unicode.SimpleFold(n) {
		minimum = min(minimum, n)
	}
	return minimum
}

// KMP keeps repeated-character searches linear. Offsets use the native
// editor's rune coordinates, including Unicode simple-fold equivalents.
func analyzeFile(ctx context.Context, value, query string, lines []uint32) (*fileAnalysis, error) {
	r := &fileAnalysis{lines: lines, bytes: len(value)}
	indexLines := lines == nil
	if indexLines {
		r.lines = []uint32{0}
	}
	needle := []rune(query)
	for i := range needle {
		if i%4096 == 0 && ctx.Err() != nil {
			return nil, ctx.Err()
		}
		needle[i] = foldFileRune(needle[i])
	}
	r.matchLength = len(needle)
	prefix := make([]int, len(needle))
	for i, j := 1, 0; i < len(needle); i++ {
		if i%4096 == 0 && ctx.Err() != nil {
			return nil, ctx.Err()
		}
		for j > 0 && needle[i] != needle[j] {
			j = prefix[j-1]
		}
		if needle[i] == needle[j] {
			j++
		}
		prefix[i] = j
	}
	j := 0
	for _, ch := range value {
		if r.runes%4096 == 0 {
			if err := ctx.Err(); err != nil {
				return nil, err
			}
		}
		if indexLines && ch == '\n' {
			r.lines = append(r.lines, uint32(r.runes+1))
		}
		if len(needle) > 0 {
			folded := foldFileRune(ch)
			for j > 0 && folded != needle[j] {
				j = prefix[j-1]
			}
			if folded == needle[j] {
				j++
			}
			if j == len(needle) {
				r.matches = append(r.matches, uint32(r.runes-len(needle)+1))
				j = prefix[j-1]
			}
		}
		r.runes++
	}
	return r, ctx.Err()
}

func (a *App) prepareFileAnalysis(v *fileView) bool {
	if v.Closed || v.Editor == nil {
		return false
	}
	s, editor, query := &v.Search, v.Editor, text(v.Find)
	revision := editor.TextRevision()
	if s.editor == editor && s.revision == revision && s.query == query {
		return s.result != nil && !s.pending && s.err == ""
	}
	if s.cancel != nil {
		s.cancel()
	}
	var lines []uint32
	if s.editor == editor && s.revision == revision && s.result != nil {
		lines = s.result.lines
	}
	s.generation++
	generation := s.generation
	s.editor, s.revision, s.query = editor, revision, query
	s.pending, s.result, s.err, s.moves = true, nil, "", 0
	ctx := a.ctx
	if ctx == nil {
		ctx = context.Background()
	}
	ctx, cancel := context.WithCancel(ctx)
	s.cancel = cancel
	value := editor.Snapshot()
	a.work(func() {
		defer cancel()
		result, err := analyzeFile(ctx, value, query, lines)
		a.post(func() {
			if v.Closed || s.generation != generation || v.Editor != editor || editor.TextRevision() != revision || text(v.Find) != query {
				return
			}
			s.pending, s.cancel = false, nil
			if err != nil {
				s.err = err.Error()
				return
			}
			s.result = result
			if s.moves != 0 {
				selectFileMatch(v, s.moves)
			} else if query != "" && v.FindOpen && len(result.matches) > 0 {
				setFileMatch(v, 0)
			}
			s.moves = 0
		})
	}, func() {
		if s.generation == generation {
			s.pending, s.err = false, errWorkQueueFull.Error()
		}
		cancel()
	})
	return false
}

func (a *App) fileFind(v *fileView, back bool) {
	if text(v.Find) == "" || v.Editor == nil {
		return
	}
	direction := 1
	if back {
		direction = -1
	}
	if a.prepareFileAnalysis(v) {
		selectFileMatch(v, direction)
	} else {
		v.Search.moves += direction
	}
}

func setFileMatch(v *fileView, at int) {
	r := v.Search.result
	if r == nil || len(r.matches) == 0 {
		return
	}
	start := int(r.matches[at])
	v.Editor.SelectStart, v.Editor.SelectEnd = start, start+r.matchLength
	v.Editor.Cursor, v.Editor.CursorFollow = start+r.matchLength, true
}

func selectFileMatch(v *fileView, moves int) {
	r := v.Search.result
	if r == nil || len(r.matches) == 0 || moves == 0 {
		return
	}
	ed := v.Editor
	index := sort.Search(len(r.matches), func(i int) bool { return int(r.matches[i]) >= ed.SelectStart })
	selected := index < len(r.matches) && int(r.matches[index]) == ed.SelectStart && ed.SelectEnd == ed.SelectStart+r.matchLength
	if selected {
		index += moves
	} else {
		position := ed.Cursor
		if ed.SelectStart != ed.SelectEnd {
			position = max(ed.SelectStart, ed.SelectEnd)
			if moves < 0 {
				position = min(ed.SelectStart, ed.SelectEnd)
			}
		}
		index = sort.Search(len(r.matches), func(i int) bool { return int(r.matches[i]) >= position })
		if moves > 0 {
			index += moves - 1
		} else {
			index += moves
		}
	}
	index = (index%len(r.matches) + len(r.matches)) % len(r.matches)
	setFileMatch(v, index)
}

func (v *fileView) findLabel() string {
	s := &v.Search
	if s.pending {
		return "Searching…"
	}
	if s.err != "" {
		return "Search unavailable"
	}
	if s.query == "" || s.result == nil {
		return ""
	}
	if len(s.result.matches) == 0 {
		return "No results"
	}
	r := s.result
	i := sort.Search(len(r.matches), func(i int) bool { return int(r.matches[i]) >= v.Editor.SelectStart })
	current := 0
	if i < len(r.matches) && int(r.matches[i]) == v.Editor.SelectStart && v.Editor.SelectEnd == v.Editor.SelectStart+r.matchLength {
		current = i + 1
	}
	return fmt.Sprintf("%d of %d", current, len(r.matches))
}

func (v *fileView) fileStatus() string {
	r := v.Search.result
	if r == nil || v.Editor == nil {
		return "Indexing text…"
	}
	cursor := min(max(0, v.Editor.Cursor), r.runes)
	line := max(0, sort.Search(len(r.lines), func(i int) bool { return int(r.lines[i]) > cursor })-1)
	size := fmt.Sprintf("%d B", r.bytes)
	if r.bytes >= 1024 {
		size = fmt.Sprintf("%.1f KB", float64(r.bytes)/1024)
	}
	scope := ""
	if v.More || v.LimitReached {
		scope = " loaded"
	}
	return fmt.Sprintf("%s · %d lines%s · Ln %d, Col %d", size, len(r.lines), scope, line+1, cursor-int(r.lines[line])+1)
}

func (a *App) decorateFile(v *fileView, scale float64, face font.Face) {
	ed, s := v.Editor, &v.Search
	r := s.result
	if r == nil {
		ed.GutterWidth, ed.PaintGutter, ed.PaintText, ed.PaintTabText = 0, nil, nil, nil
		s.paint = nil
		return
	}
	ed.GutterWidth = desktop.FontWidth(face, strconv.Itoa(len(r.lines))) + int(14*scale)
	if s.paint == r && s.palette == a.p && s.paintScale == scale && s.paintOpen == v.FindOpen {
		return
	}
	s.paint, s.palette, s.paintScale, s.paintOpen = r, a.p, scale, v.FindOpen
	p := a.p
	ed.PaintGutter = func(out *command.Buffer, b rect.Rect, start int, f font.Face) {
		line := sort.Search(len(r.lines), func(i int) bool { return int(r.lines[i]) >= start })
		if line == len(r.lines) || int(r.lines[line]) != start {
			return
		}
		value := strconv.Itoa(line + 1)
		width := desktop.FontWidth(f, value)
		b.X, b.W = b.X+b.W-width-int(7*scale), width
		out.DrawText(b, value, f, p.Muted)
	}
	ed.PaintText, ed.PaintTabText = nil, nil
	if !v.FindOpen {
		return
	}
	paint := func(out *command.Buffer, b rect.Rect, runes []rune, start int, f font.Face, fg color.RGBA, selected, tab bool) {
		if !selected {
			paintFileMatches(out, b, runes, start, f, r, tab, p.WarningSoft)
		}
		out.DrawText(b, string(runes), f, fg)
	}
	ed.PaintText = func(out *command.Buffer, b rect.Rect, runes []rune, start int, f font.Face, fg color.RGBA, selected bool) {
		paint(out, b, runes, start, f, fg, selected, false)
	}
	ed.PaintTabText = func(out *command.Buffer, b rect.Rect, runes []rune, start int, f font.Face, fg color.RGBA, selected bool) {
		paint(out, b, runes, start, f, fg, selected, true)
	}
}

// Scan prefix advances once, stop at the visible edge, and coalesce adjacent
// highlights. Repeated matches on a long line must not measure every prefix or
// enqueue commands for the offscreen suffix. Coordinates match MeasureString.
func paintFileMatches(out *command.Buffer, b rect.Rect, runes []rune, start int, f font.Face, r *fileAnalysis, tab bool, tone color.RGBA) {
	if len(r.matches) == 0 || b.Y+b.H <= out.Clip.Y || b.Y >= out.Clip.Y+out.Clip.H {
		return
	}
	left, right := out.Clip.X, out.Clip.X+out.Clip.W
	match, from, to := -2, 0, 0
	flush := func() {
		if to > from {
			out.FillRect(rect.Rect{X: from, Y: b.Y, W: to - from, H: b.H}, 1, tone)
		}
		from, to = 0, 0
	}
	span := func(index, x, end int) {
		if end <= left || x >= right {
			return
		}
		if match == -2 {
			match = sort.Search(len(r.matches), func(i int) bool { return int(r.matches[i]) > index }) - 1
		} else {
			for match+1 < len(r.matches) && int(r.matches[match+1]) <= index {
				match++
			}
		}
		if match < 0 || int(r.matches[match])+r.matchLength <= index {
			flush()
			return
		}
		x, end = max(x, left), min(end, right)
		if to > from && x > to {
			flush()
		}
		if to <= from {
			from = x
		}
		to = max(to, end)
	}
	var advance fixed.Int26_6
	x := b.X
	for i, ch := range runes {
		if x >= right {
			break
		}
		if i > 0 {
			advance += f.Face.Kern(runes[i-1], ch)
		}
		width, _ := f.Face.GlyphAdvance(ch)
		advance += width
		end := b.X + advance.Ceil()
		span(start+i, x, end)
		x = end
	}
	if tab && x < right {
		span(start+len(runes), x, b.X+b.W)
	}
	flush()
}

func (a *App) fileSearchKey(v *fileView, event *desktop.KeyboardEvent, primary key.Modifiers) {
	if event.HandleKey(key.CodeF, primary) {
		v.FindOpen, v.FocusFind = true, true
	}
	if event.HandleKey(key.CodeF3, 0) {
		v.FindOpen = true
		a.fileFind(v, false)
	}
	if event.HandleKey(key.CodeF3, key.ModShift) {
		v.FindOpen = true
		a.fileFind(v, true)
	}
	if v.FindOpen {
		if event.HandleKey(key.CodeEscape, 0) {
			v.FindOpen, v.FocusFile = false, true
		}
		if v.Find != nil && v.Find.Active {
			if event.HandleKey(key.CodeReturnEnter, 0) || event.HandleKey(key.CodeKeypadEnter, 0) {
				a.fileFind(v, false)
			}
			if event.HandleKey(key.CodeReturnEnter, key.ModShift) || event.HandleKey(key.CodeKeypadEnter, key.ModShift) {
				a.fileFind(v, true)
			}
		}
	}
}
