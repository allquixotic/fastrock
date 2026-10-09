//go:build fltk_headless

package desktop

import (
	"bytes"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"golang.org/x/mobile/event/key"
	"image"
)

// HeadlessHarness exercises layout and the software renderer without starting a
// window, clipboard, display driver or background updater. Only test builds
// include it; Frame must be called serially by the test.
type HeadlessHarness struct {
	window *masterWindow
	pixels *image.RGBA
}

func NewHeadlessHarness(flags WindowFlags, size image.Point, fn UpdateFn) *HeadlessHarness {
	w := &masterWindow{}
	w.masterWindowCommonInit(&context{}, flags, fn, w)
	return &HeadlessHarness{window: w, pixels: image.NewRGBA(image.Rectangle{Max: size})}
}
func (h *HeadlessHarness) Master() MasterWindow { return h.window }

// Key injects a native-style key press without a display or window system.
func (h *HeadlessHarness) Key(code key.Code, modifiers key.Modifiers) {
	h.KeyRune(code, modifiers, 0)
}

// KeyRune also injects the text produced by a printable native key event.
func (h *HeadlessHarness) KeyRune(code key.Code, modifiers key.Modifiers, r rune) {
	var text bytes.Buffer
	h.window.ctx.processKeyEvent(key.Event{Code: code, Modifiers: modifiers, Rune: r, Direction: key.DirPress}, &text)
	h.window.ctx.Input.Keyboard.addText(text.String())
}
func (h *HeadlessHarness) Frame(render bool) int {
	w := h.window
	w.ctx.Windows[0].Bounds = rect.FromRectangle(h.pixels.Bounds())
	w.ctx.Update()
	changed := w.drawChanged()
	if render && changed {
		w.ctx.DrawDamage(h.pixels, w.frameDamage)
	}
	w.prevCmds = append(w.prevCmds[:0], w.ctx.cmds...)
	return len(w.ctx.cmds)
}

// Commands returns a snapshot of the completed frame, including modal windows.
// Like Frame, it must be called serially or while holding the master lock.
func (h *HeadlessHarness) Commands() []command.Command {
	return append([]command.Command(nil), h.window.ctx.cmds...)
}
