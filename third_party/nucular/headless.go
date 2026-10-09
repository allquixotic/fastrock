//go:build nucular_headless

package nucular

import (
	"github.com/aarzilli/nucular/rect"
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
func (h *HeadlessHarness) Frame(render bool) int {
	w := h.window
	w.ctx.Windows[0].Bounds = rect.FromRectangle(h.pixels.Bounds())
	w.ctx.Update()
	changed := w.drawChanged()
	if render && changed {
		w.ctx.Draw(h.pixels)
	}
	w.prevCmds = append(w.prevCmds[:0], w.ctx.cmds...)
	return len(w.ctx.cmds)
}
