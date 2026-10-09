package desktop

import (
	"fmt"
	"hash/fnv"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/desktop/internal/windowing"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	nstyle "github.com/allquixotic/fastrock/internal/desktop/style"
)

func (win *Window) nativeBounds(b rect.Rect) bool {
	return win.ctx.native && win.flags&windowEnabled != 0 &&
		b.W > 0 && b.H > 0 && b.Rectangle().Intersect(win.cmds.Clip.Rectangle()) == b.Rectangle()
}

func (win *Window) nativeButton(lbl label.Label, b rect.Rect, style *nstyle.Button, repeat bool) bool {
	if !win.nativeBounds(b) || lbl.Kind != label.TextLabel || repeat ||
		style.Draw.ButtonText != nil || style.DrawBegin != nil || style.DrawEnd != nil {
		return false
	}
	h := fnv.New64a()
	fmt.Fprintf(h, "%d:%d:%d:%d:%d:%s", win.idx, b.X, b.Y, b.W, b.H, lbl.Text)
	id := h.Sum64() | (1 << 63)
	win.ctx.nativeControls = append(win.ctx.nativeControls, nativeControl{win: win, control: windowing.Control{
		ID: id, Kind: windowing.ButtonControl, Bounds: b.Rectangle(), Text: lbl.Text,
		Foreground: style.TextNormal, Background: style.Normal.Data.Color,
		Selection: style.Active.Data.Color, FontSize: nativeFontSize(win),
		Border: style.BorderColor, BorderWidth: style.Border,
	}})
	clicked := win.ctx.nativeClicks[id]
	delete(win.ctx.nativeClicks, id)
	return clicked
}

func nativeFontSize(win *Window) int {
	return max(8, int(float64(FontHeight(win.ctx.Style.Font))*0.82+0.5))
}

type nativeControl struct {
	win     *Window
	control windowing.Control
}

func (edit *TextEditor) nativeInput(win *Window, b rect.Rect) bool {
	if !win.nativeBounds(b) || edit.Flags&(EditMultiline|EditReadOnly|EditNoCursor) != 0 ||
		edit.PasswordChar != 0 || edit.PaintText != nil || edit.PaintTabText != nil || edit.PaintGutter != nil ||
		edit.Filter != nil && fmt.Sprintf("%p", edit.Filter) != fmt.Sprintf("%p", FilterDefault) {
		return false
	}
	ctx := win.ctx
	if edit.nativeID == 0 {
		ctx.nextNativeID++
		edit.nativeID = ctx.nextNativeID
	}
	if ctx.nativeEditors == nil {
		ctx.nativeEditors = make(map[uint64]*TextEditor)
	}
	ctx.nativeEditors[edit.nativeID] = edit
	value := edit.Snapshot()
	style := ctx.Style.Edit
	ctx.nativeControls = append(ctx.nativeControls, nativeControl{win: win, control: windowing.Control{
		ID: edit.nativeID, Kind: windowing.InputControl, Bounds: b.Rectangle(), Text: value,
		Placeholder: edit.Placeholder, Foreground: style.TextNormal, Background: style.Normal.Data.Color,
		Selection: style.SelectedNormal, FontSize: nativeFontSize(win),
		Sequence: edit.nativeSequence, Cursor: runeByteOffset(value, edit.Cursor),
		Mark: runeByteOffset(value, edit.SelectStart), Focused: edit.Active,
	}})
	return true
}

func runeByteOffset(s string, n int) int {
	for i := range s {
		if n == 0 {
			return i
		}
		n--
	}
	return len(s)
}

func (ctx *context) nativeFocus(id uint64) {
	for _, item := range ctx.nativeControls {
		if item.control.ID != id {
			continue
		}
		ctx.Input.activateWindow = item.win
		if editor := ctx.nativeEditors[id]; editor != nil {
			ctx.Input.activateEditor = editor
		} else {
			// A non-editor target deactivates every canvas text editor.
			ctx.Input.activateEditor = id
		}
		return
	}
}

func (ctx *context) nativeEvent(e windowing.ControlEvent) {
	if e.Kind == windowing.ButtonControl {
		if ctx.nativeClicks == nil {
			ctx.nativeClicks = make(map[uint64]bool)
		}
		ctx.nativeClicks[e.ID] = true
		return
	}
	ed := ctx.nativeEditors[e.ID]
	if ed == nil || e.Sequence <= ed.nativeSequence {
		return
	}
	ed.nativeSequence = e.Sequence
	// A delayed native edit must never overwrite an app update or a restored draft.
	if ed.Snapshot() != e.Base {
		return
	}
	value := []rune(e.Text)
	if ed.Maxlen > 0 && len(value) > ed.Maxlen {
		return
	}
	if ed.Snapshot() != e.Text {
		ed.SetText(e.Text)
	}
	ed.Cursor = utf8.RuneCountInString(e.Text[:max(0, min(e.Cursor, len(e.Text)))])
	ed.SelectStart = utf8.RuneCountInString(e.Text[:max(0, min(e.Mark, len(e.Text)))])
	ed.SelectEnd = ed.Cursor
	ed.Active = e.Focused
	if e.Committed && ed.Flags&EditSigEnter != 0 {
		ed.nativeCommitted = true
		ed.Active = false
	}
}

func (ctx *context) publishControls() []windowing.Control {
	controls := make([]windowing.Control, 0, len(ctx.nativeControls))
	active := make(map[uint64]*TextEditor)
	// Under a popup, only its controls may intercept input above the canvas.
	top := ctx.Windows[0]
	for _, win := range ctx.Windows {
		if win.flags&windowTooltip == 0 {
			top = win
		}
	}
	for _, item := range ctx.nativeControls {
		win := item.win
		for win.parent != nil {
			win = win.parent
		}
		if top != ctx.Windows[0] && win != top {
			continue
		}
		covered := false
		for _, overlay := range ctx.Windows {
			if overlay.flags&windowTooltip != 0 && overlay.Bounds.Rectangle().Overlaps(item.control.Bounds) {
				covered = true
				break
			}
		}
		if covered {
			continue
		}
		controls = append(controls, item.control)
		if ed := ctx.nativeEditors[item.control.ID]; ed != nil {
			active[item.control.ID] = ed
		}
	}
	ctx.nativeEditors = active
	clear(ctx.nativeClicks)
	return controls
}
