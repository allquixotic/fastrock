package ui

import (
	"fmt"
	"image"
	"image/color"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/label"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/mouse"
)

type diffLayout struct {
	offsets                          []int
	scale                            float64
	spacing, rowHeight, headerHeight int
}

func (a *App) openDiff(title, source, cwd string) {
	id := a.state.Open(workspace.File, title, "text:"+title, "")
	v := &fileView{Path: title, BasePath: cwd, Find: textEditor("", false), Virtual: true}
	a.files[id] = v
	a.prepareDiff(v, source)
}
func (a *App) prepareDiff(v *fileView, source string) {
	v.DiffGeneration++
	generation := v.DiffGeneration
	v.Loading = true
	previous := v.DiffSource
	if len(v.Diff) == 0 && v.DiffSummary == "" {
		v.DiffSource = source
	}
	a.work(func() {
		truncated := len(source) >= maxFileBytes
		source = boundedDiffText(source, maxFileBytes)
		files, rowLimit := parseDiff(source)
		added, removed, columns := 0, 0, 0
		for _, f := range files {
			added += f.Added
			removed += f.Removed
			for _, l := range f.Rows {
				columns = max(columns, l.Columns)
			}
		}
		a.post(func() {
			if v.DiffGeneration != generation {
				return
			}
			collapsed := map[string]bool{}
			for _, f := range v.Diff {
				collapsed[f.displayPath()] = f.Collapsed
			}
			for _, path := range v.DiffCollapsed {
				collapsed[path] = true
			}
			for i := range files {
				files[i].Collapsed = collapsed[files[i].displayPath()]
			}
			v.DiffCollapsed = nil
			if previous != source && !v.DiffRestoreScroll {
				v.DiffSelection = diffSelection{}
			}
			v.Diff, v.DiffSource, v.DiffLayout = files, source, nil
			v.DiffSummary = fmt.Sprintf("%d files changed · +%d −%d", len(files), added, removed)
			v.DiffTruncated, v.DiffColumns = truncated || rowLimit, columns
			v.resetFileSearch()
			v.Editor = nil // Parsed rows reference source bytes; no full or per-file rune copies.
			v.Loading = false
		})
	}, func() { v.Loading = false; v.Error = errWorkQueueFull.Error() })
}
func boundedDiffText(source string, limit int) string {
	if len(source) < limit {
		return source
	}
	source = source[:limit]
	// Do not pretend an incomplete final patch line is a complete change.
	if end := strings.LastIndexByte(source, '\n'); end >= 0 {
		return source[:end+1]
	}
	return ""
}
func (v *fileView) prepareDiffLayout(scale float64, spacing, rowHeight int) *diffLayout {
	if c := v.DiffLayout; c != nil && c.scale == scale && c.spacing == spacing && c.rowHeight == rowHeight {
		return c
	}
	c := &diffLayout{offsets: make([]int, len(v.Diff)+1), scale: scale, spacing: spacing, rowHeight: rowHeight, headerHeight: int(28 * scale)}
	for i, f := range v.Diff {
		height := c.headerHeight + spacing
		if !f.Collapsed {
			height += len(f.Rows) * (rowHeight + spacing)
		}
		c.offsets[i+1] = c.offsets[i] + height
	}
	v.DiffLayout = c
	return c
}
func (v *fileView) invalidateDiffLayout() { v.DiffLayout = nil }
func (v *fileView) jumpDiff(file, row int) {
	if file >= 0 && file < len(v.Diff) {
		v.Diff[file].Collapsed = false
		v.invalidateDiffLayout()
		v.DiffJumpFile, v.DiffJumpRow, v.DiffJump = file, row, true
	}
}
func (v *fileView) nextDiffAnchor(fileOnly, back bool) {
	layout := v.DiffLayout
	if layout == nil {
		return
	}
	current := v.DiffScroll.Y
	targetFile, targetRow := -1, -1
	for i, f := range v.Diff {
		if fileOnly {
			if (!back && layout.offsets[i] > current+2) || (back && layout.offsets[i] < current-2) {
				targetFile, targetRow = i, -1
				if !back {
					break
				}
			}
			continue
		}
		for j, l := range f.Rows {
			if l.Kind != diffHunk {
				continue
			}
			at := layout.offsets[i] + layout.headerHeight + layout.spacing + j*(layout.rowHeight+layout.spacing)
			if f.Collapsed {
				at = layout.offsets[i]
			}
			if (!back && at > current+2) || (back && at < current-2) {
				targetFile, targetRow = i, j
				if !back {
					break
				}
			}
		}
		if targetFile >= 0 && !back {
			break
		}
	}
	if targetFile >= 0 {
		v.jumpDiff(targetFile, targetRow)
	}
}
func (a *App) drawDiff(w *nucular.Window, v *fileView) {
	columns := 4
	if v.BasePath != "" {
		columns++
	}
	w.Row(28).Dynamic(columns)
	if w.ButtonText("Expand all") {
		for i := range v.Diff {
			v.Diff[i].Collapsed = false
		}
		v.invalidateDiffLayout()
	}
	if w.ButtonText("Collapse all") {
		for i := range v.Diff {
			v.Diff[i].Collapsed = true
		}
		v.invalidateDiffLayout()
	}
	if w.ButtonText("Copy diff") {
		a.copyText(v.DiffSource)
	}
	if w.ButtonText("View as text") {
		a.openText(v.Path, v.DiffSource)
		return
	}
	if v.BasePath != "" && w.ButtonText("Open folder") {
		a.openPath(v.BasePath, false)
	}
	muted(w, v.DiffSummary, a.p)
	w.Row(28).Dynamic(4)
	if w.ButtonText("Previous file") {
		v.nextDiffAnchor(true, true)
	}
	if w.ButtonText("Next file") {
		v.nextDiffAnchor(true, false)
	}
	if w.ButtonText("Previous hunk") {
		v.nextDiffAnchor(false, true)
	}
	if w.ButtonText("Next hunk") {
		v.nextDiffAnchor(false, false)
	}
	if v.DiffTruncated {
		w.Row(48).Dynamic(1)
		w.LabelWrap("Diff display limit reached. Copy the captured diff or inspect the full change in an external Git viewer.")
	}
	if v.DiffSelection.Focus {
		for event := range w.Input().Keyboard.Events() {
			if event.HandleKey(key.CodeC, platform.PrimaryModifier()) {
				a.copyText(v.diffSelectedText())
			} else if event.HandleKey(key.CodeA, platform.PrimaryModifier()) {
				v.DiffSelection.Anchor = 0
				v.DiffSelection.End = len(v.DiffSource)
			} else if event.HandleKey(key.CodeEscape, 0) {
				v.DiffSelection = diffSelection{}
			}
		}
	}
	if !w.Input().Mouse.Down(mouse.ButtonLeft) {
		v.DiffSelection.Dragging = false
	}
	w.Row(max(100, w.LayoutAvailableHeight()-4)).Dynamic(1)
	if body := w.GroupBegin("diff-rows", 0); body != nil {
		face := a.monoFace
		if face == (font.Face{}) {
			face = w.Master().Style().Font
		}
		spacing := body.WindowStyle().Spacing.Y
		scale := w.Master().Style().Scaling
		layout := v.prepareDiffLayout(scale, spacing, nucular.FontHeight(face)+int(6*scale))
		if v.DiffRestoreScroll {
			body.Scrollbar = v.DiffScroll
			v.DiffRestoreScroll = false
		}
		if v.DiffJump {
			y := layout.offsets[v.DiffJumpFile]
			if v.DiffJumpRow >= 0 {
				y += layout.headerHeight + spacing + v.DiffJumpRow*(layout.rowHeight+spacing)
			}
			body.Scrollbar.Y = y
			v.DiffJump = false
		}
		top := body.LayoutNextRowY()
		first := sort.Search(len(v.Diff), func(i int) bool { return top+layout.offsets[i+1] >= body.Bounds.Y })
		last := sort.Search(len(v.Diff), func(i int) bool { return top+layout.offsets[i] > body.Bounds.Y+body.Bounds.H })
		last = min(len(v.Diff), last+1)
		skipDiffPixels(body, layout.offsets[first], spacing)
		cell := max(1, nucular.FontWidth(face, "M"))
		width := max(body.LayoutAvailableWidth(), (v.DiffColumns+18)*cell)
		for i := first; i < last; i++ {
			f := &v.Diff[i]
			fileTop := top + layout.offsets[i]
			if fileTop+layout.headerHeight >= body.Bounds.Y && fileTop <= body.Bounds.Y+body.Bounds.H {
				body.RowScaled(layout.headerHeight).Ratio(.54, .20, .16, .10)
				if body.ButtonText(f.displayPath()) {
					f.Collapsed = !f.Collapsed
					v.invalidateDiffLayout()
				}
				a.diffStatusBadge(body, f)
				a.diffCountLabel(body, f)
				if f.Path != "" {
					if body.ButtonText("Open") {
						path := f.Path
						if !filepath.IsAbs(path) {
							path = filepath.Join(v.BasePath, path)
						}
						a.openFile(path)
						for _, l := range f.Rows {
							if l.New > 0 {
								a.goToFileLine(a.files[a.state.Active], l.New, 1)
								if a.files[a.state.Active].Loading {
									a.files[a.state.Active].PendingLine = l.New
								}
								break
							}
						}
					}
				} else {
					body.Spacing(1)
				}
			} else {
				skipDiffPixels(body, layout.headerHeight+spacing, spacing)
			}
			if f.Collapsed {
				continue
			}
			start := fileTop + layout.headerHeight + spacing
			stride := layout.rowHeight + spacing
			lo, hi := sidebarVisible(start, body.Bounds.Y, body.Bounds.Y+body.Bounds.H, stride, len(f.Rows))
			sidebarSkip(body, lo, stride, spacing)
			for j := lo; j < hi; j++ {
				body.RowScaled(layout.rowHeight).Static(width)
				a.drawDiffLine(body, v, f, &f.Rows[j], face, cell)
			}
			sidebarSkip(body, len(f.Rows)-hi, stride, spacing)
		}
		skipDiffPixels(body, layout.offsets[len(v.Diff)]-layout.offsets[last], spacing)
		v.DiffScroll = body.Scrollbar
		body.GroupEnd()
	}
}
func skipDiffPixels(w *nucular.Window, pixels, spacing int) {
	if pixels > 0 {
		w.RowScaled(pixels - spacing).Dynamic(1)
		w.Spacing(1)
	}
}
func (v *fileView) diffSelectedText() string {
	lo, hi := min(v.DiffSelection.Anchor, v.DiffSelection.End), max(v.DiffSelection.Anchor, v.DiffSelection.End)
	lo, hi = max(0, min(lo, len(v.DiffSource))), max(0, min(hi, len(v.DiffSource)))
	for lo > 0 && lo < len(v.DiffSource) && !utf8.RuneStart(v.DiffSource[lo]) {
		lo--
	}
	for hi > 0 && hi < len(v.DiffSource) && !utf8.RuneStart(v.DiffSource[hi]) {
		hi--
	}
	return v.DiffSource[lo:hi]
}
func diffByteColumn(source string, l *diffLine, at int) int {
	col, start := 0, 0
	i := sort.Search(len(l.Index), func(i int) bool { return l.Index[i].Byte > at })
	if i > 0 {
		col, start = l.Index[i-1].Column, l.Index[i-1].Byte
	}
	for _, r := range source[l.Start+start : l.Start+at] {
		if r == '\t' {
			col += 4
		} else {
			col++
		}
	}
	return col
}
func (a *App) drawDiffLine(w *nucular.Window, v *fileView, f *diffFile, l *diffLine, face font.Face, cell int) {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return
	}
	fg, bg, marker := a.p.Text, a.p.Window, ""
	switch l.Kind {
	case diffAdded:
		fg, bg, marker = a.p.DiffAddFG, a.p.DiffAddBG, "+"
	case diffRemoved:
		fg, bg, marker = a.p.DiffDelFG, a.p.DiffDelBG, "−"
	case diffHunk:
		fg, bg = a.p.DiffHunkFG, a.p.AccentSoft
	case diffMeta, diffNoNewline, diffBinary:
		fg = a.p.Muted
	}
	out.FillRect(b, 0, bg)
	x := b.X + 17*cell
	if l.Kind == diffHunk || l.Kind == diffMeta || l.Kind == diffNoNewline || l.Kind == diffBinary {
		x = b.X + cell
	}
	numbers := ""
	if l.Old > 0 {
		numbers = fmt.Sprintf("%6d", l.Old)
	} else {
		numbers = "      "
	}
	if l.New > 0 {
		numbers += fmt.Sprintf(" %6d", l.New)
	} else {
		numbers += "       "
	}
	numbers += "  " + marker
	if l.Kind == diffAdded || l.Kind == diffRemoved || l.Kind == diffContext {
		out.DrawText(rect.Rect{X: b.X, Y: b.Y, W: 16 * cell, H: b.H}, numbers, face, a.p.Faint)
	}
	clip := w.Bounds
	firstCol := max(0, (clip.X-x)/cell-1)
	lastCol := max(firstCol, (clip.X+clip.W-x)/cell+2)
	lo, hi := diffColumnByte(v.DiffSource, l, firstCol), diffColumnByte(v.DiffSource, l, lastCol)
	column := diffByteColumn(v.DiffSource, l, lo)
	for _, r := range l.Emphasis {
		left, right := max(lo, r.Start), min(hi, r.End)
		if left < right {
			c := diffByteColumn(v.DiffSource, l, left)
			end := diffByteColumn(v.DiffSource, l, right)
			out.FillRect(rect.Rect{X: x + c*cell, Y: b.Y, W: (end - c) * cell, H: b.H}, 0, blendDiff(bg, fg))
		}
	}
	sel := &v.DiffSelection
	hit := func(px int) int { return l.Start + diffColumnByte(v.DiffSource, l, max(0, (px-x+cell/2)/cell)) }
	if w.Input().Mouse.IsClickDownInRect(mouse.ButtonLeft, b, true) {
		at := hit(w.Input().Mouse.Buttons[mouse.ButtonLeft].ClickedPos.X)
		*sel = diffSelection{Anchor: at, End: at, Dragging: true, Focus: true}
	}
	if sel.Dragging && w.Input().Mouse.HoveringRect(b) {
		sel.End = hit(w.Input().Mouse.Pos.X)
	}
	left, right := max(l.Start+lo, min(sel.Anchor, sel.End)), min(l.Start+hi, max(sel.Anchor, sel.End))
	if left < right {
		c := diffByteColumn(v.DiffSource, l, left-l.Start)
		end := diffByteColumn(v.DiffSource, l, right-l.Start)
		out.FillRect(rect.Rect{X: x + c*cell, Y: b.Y, W: (end - c) * cell, H: b.H}, 0, a.p.Selected)
	}
	value := strings.ReplaceAll(v.DiffSource[l.Start+lo:l.Start+hi], "\t", "    ")
	out.DrawText(rect.Rect{X: x + column*cell, Y: b.Y, W: max(cell, (lastCol-column)*cell), H: b.H}, value, face, fg)
	if menu := w.ContextualOpen(0, image.Pt(210, 100), b, nil); menu != nil {
		if menu.MenuItem(label.T("Copy selection")) {
			a.copyText(v.diffSelectedText())
		}
		if menu.MenuItem(label.T("Copy line")) {
			a.copyText(v.DiffSource[l.Start:l.End])
		}
		if menu.MenuItem(label.T("Copy file diff")) {
			a.copyText(f.Text)
		}
	}
}

func blendDiff(bg, fg color.RGBA) color.RGBA {
	return color.RGBA{uint8((int(bg.R)*3 + int(fg.R)) / 4), uint8((int(bg.G)*3 + int(fg.G)) / 4), uint8((int(bg.B)*3 + int(fg.B)) / 4), 255}
}

func (a *App) diffStatusBadge(w *nucular.Window, f *diffFile) {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return
	}
	text := f.Status
	if f.Binary {
		if text == "modified" {
			text = "binary"
		} else {
			text += " · binary"
		}
	}
	fg, bg := a.p.Muted, a.p.Alt
	if f.Status == "added" {
		fg, bg = a.p.Success, a.p.SuccessSoft
	} else if f.Status == "deleted" {
		fg, bg = a.p.Danger, a.p.DangerSoft
	}
	font := w.Master().Style().Font
	b.Y += 3
	b.H = max(1, b.H-6)
	b.W = min(b.W, nucular.FontWidth(font, text)+12)
	out.FillRect(b, 4, bg)
	b.X += 6
	b.W = max(0, b.W-12)
	out.DrawText(b, text, font, fg)
	if w.Input().Mouse.HoveringRect(b) {
		w.Tooltip(text)
	}
}
func (a *App) diffCountLabel(w *nucular.Window, f *diffFile) {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return
	}
	font := w.Master().Style().Font
	left := b
	left.W = b.W / 2
	out.DrawText(left, fmt.Sprintf("+%d", f.Added), font, a.p.DiffAddFG)
	b.X += left.W
	b.W -= left.W
	out.DrawText(b, fmt.Sprintf("−%d", f.Removed), font, a.p.DiffDelFG)
}
