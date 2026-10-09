//go:build fltk_headless

package desktop

import (
	"image"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop/internal/windowing"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"golang.org/x/mobile/event/key"
)

func TestNativeFieldChangesAndDelayedEdits(t *testing.T) {
	ed := &TextEditor{Flags: EditField, Buffer: []rune("before"), Maxlen: 30}
	h := NewHeadlessHarness(0, image.Pt(500, 300), func(w *Window) { w.Row(32).Dynamic(1); ed.Edit(w) })
	ctx := h.window.ctx
	ctx.native = true
	h.Frame(false)
	h.Frame(false)
	controls := ctx.publishControls()
	if len(controls) != 1 || controls[0].Kind != windowing.InputControl {
		t.Fatalf("plain field was not native: %+v", controls)
	}
	ctx.nativeEvent(windowing.ControlEvent{ID: ed.nativeID, Kind: windowing.InputControl, Sequence: 1, Base: "before", Text: "café", Cursor: 5, Mark: 3, Focused: true})
	if ed.Snapshot() != "café" || ed.Cursor != 4 || ed.SelectStart != 3 {
		t.Fatalf("UTF-8 edit/selection lost: %+v", ed)
	}
	ed.SetText("restored draft")
	ctx.nativeEvent(windowing.ControlEvent{ID: ed.nativeID, Kind: windowing.InputControl, Sequence: 2, Base: "café", Text: "late typing"})
	if ed.Snapshot() != "restored draft" {
		t.Fatal("delayed input overwrote restored draft")
	}
	ctx.nativeEvent(windowing.ControlEvent{ID: ed.nativeID, Kind: windowing.InputControl, Sequence: 1, Base: "restored draft", Text: "replayed"})
	if ed.Snapshot() != "restored draft" {
		t.Fatal("replayed input overwrote draft")
	}
}

func TestCustomEditorsRemainCanvasAndNativeButtonClicksOnce(t *testing.T) {
	ed := &TextEditor{Flags: EditBox}
	clicks := 0
	h := NewHeadlessHarness(0, image.Pt(500, 300), func(w *Window) {
		w.Row(30).Dynamic(1)
		if w.Button(label.T("Save"), false) {
			clicks++
			// Application actions can force a second layout pass in this frame.
			w.ctx.trashFrame = true
		}
		w.Row(120).Dynamic(1)
		ed.Edit(w)
	})
	ctx := h.window.ctx
	ctx.native = true
	h.Frame(false)
	h.Frame(false)
	controls := ctx.publishControls()
	if len(controls) != 1 || controls[0].Kind != windowing.ButtonControl {
		t.Fatalf("custom editor projected as plain input: %+v", controls)
	}
	ctx.nativeEvent(windowing.ControlEvent{ID: controls[0].ID, Kind: windowing.ButtonControl})
	h.Frame(false)
	ctx.publishControls()
	h.Frame(false)
	if clicks != 1 {
		t.Fatalf("native button dispatched %d times", clicks)
	}
}

func TestNativeControlsStayBelowTooltips(t *testing.T) {
	root := &Window{}
	tooltip := &Window{flags: windowTooltip, Bounds: rect.Rect{X: 10, Y: 10, W: 100, H: 50}}
	ctx := &context{Windows: []*Window{root, tooltip}}
	ctx.nativeControls = []nativeControl{
		{win: root, control: windowing.Control{ID: 1, Bounds: image.Rect(20, 20, 80, 40)}},
		{win: root, control: windowing.Control{ID: 2, Bounds: image.Rect(200, 20, 260, 40)}},
	}
	controls := ctx.publishControls()
	if len(controls) != 1 || controls[0].ID != 2 {
		t.Fatalf("tooltip must cover intersecting controls and leave others available: %+v", controls)
	}
}

func TestTooltipDoesNotRemoveItsNativeTrigger(t *testing.T) {
	h := NewHeadlessHarness(0, image.Pt(500, 300), func(w *Window) {
		w.Row(30).Static(150)
		w.ButtonText("Hide navigation")
		b := w.LastWidgetBounds
		w.Input().Mouse.Pos = image.Pt(b.X+b.W/2, b.Y+b.H/2)
		w.Tooltip("Show or hide navigation")
	})
	ctx := h.window.ctx
	ctx.native = true
	h.Frame(false)
	h.Frame(false)
	controls := ctx.publishControls()
	if len(controls) != 1 || controls[0].Text != "Hide navigation" {
		t.Fatal("hover tooltip removed its native button", controls)
	}
}

func TestNativeFocusKeepsEditingKeysOutOfCanvasEditor(t *testing.T) {
	field := &TextEditor{Flags: EditField}
	canvas := &TextEditor{Flags: EditBox, Buffer: []rune("keep this"), Active: true, Cursor: 9}
	h := NewHeadlessHarness(0, image.Pt(500, 300), func(w *Window) {
		w.Row(30).Dynamic(1)
		field.Edit(w)
		w.Row(120).Dynamic(1)
		canvas.Edit(w)
		w.Row(30).Dynamic(1)
		w.ButtonText("Done")
	})
	ctx := h.window.ctx
	ctx.native = true
	h.Frame(false)
	h.Frame(false)
	controls := ctx.publishControls()
	ctx.nativeFocus(field.nativeID)
	h.Frame(false)
	if canvas.Active || !field.Active {
		t.Fatal("native focus did not leave the canvas editor")
	}
	h.Key(key.CodeDeleteBackspace, 0)
	h.Frame(false)
	if canvas.Snapshot() != "keep this" {
		t.Fatal("native editing key changed the canvas editor")
	}
	ctx.nativeFocus(controls[1].ID)
	h.Frame(false)
	if field.Active || canvas.Active {
		t.Fatal("native button focus left an editor active")
	}
}
