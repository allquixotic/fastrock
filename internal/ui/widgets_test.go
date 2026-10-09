package ui

import (
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop/rect"
)

func TestTabCloseIsInsideTabWithoutOverlappingTitle(t *testing.T) {
	for _, width := range []int{84, 150, 240} {
		b := rect.Rect{X: 10, Y: 2, W: width, H: 34}
		r := tabLayout(b)
		if r.Close.X < b.X || r.Close.X+r.Close.W > b.X+b.W || r.Close.Y < b.Y || r.Close.Y+r.Close.H > b.Y+b.H {
			t.Fatalf("close outside tab: %#v", r)
		}
		if r.Title.X+r.Title.W > r.Close.X {
			t.Fatalf("title overlaps close: %#v", r)
		}
	}
}
