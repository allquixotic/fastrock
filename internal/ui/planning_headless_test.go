//go:build fltk_headless

package ui

import (
	"fmt"
	"image"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
)

func TestV22PlanningAndTimelineDrawVisibleRowsOnly(t *testing.T) {
	a := &App{p: colors(false)}
	v := newRallyView(rally.FindPage("teamplan"))
	for i := range 100000 {
		ref := fmt.Sprintf("/iteration/%d", i)
		a.iterations = append(a.iterations, rally.Object{"_ref": ref, "Name": fmt.Sprintf("Iteration %d", i)})
		v.Items = append(v.Items, rally.Object{"_ref": fmt.Sprintf("/story/%d", i), "Name": "Story", "Iteration": map[string]any{"_ref": ref}, "PlanEstimate": float64(1)})
	}
	items := v.filtered()
	v.preparePlanning(items)
	for _, mode := range []string{"planning", "timeline"} {
		t.Run(mode, func(t *testing.T) {
			offset := 0
			h := desktop.NewHeadlessHarness(0, image.Pt(900, 600), func(w *desktop.Window) {
				w.Scrollbar.Y = offset
				if mode == "planning" {
					a.planning(w, v, items)
				} else {
					a.timeline(w, v, items)
				}
			})
			h.Master().SetStyle(makeStyle(a.p, 13))
			for _, position := range []int{0, 1000000, 3400000} {
				offset = position
				h.Frame(false)
				if n := h.Frame(true); n > 400 || n < 8 {
					t.Fatalf("%s generated %d commands at scroll %d", mode, n, position)
				}
			}
		})
	}
}
