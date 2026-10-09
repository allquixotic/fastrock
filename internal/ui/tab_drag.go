package ui

import (
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
)

func revealTab(scroll, index, width, viewport int) int {
	left, right := index*width, (index+1)*width
	if left < scroll || width > viewport {
		return left
	}
	if right > scroll+viewport {
		return right - viewport
	}
	return scroll
}

func (a *App) scrollTabs(direction int, now time.Time) {
	double := direction == a.tabScrollDirection && now.Sub(a.tabScrollClicked) < 400*time.Millisecond
	if double {
		if direction < 0 {
			a.tabScroll = 0
		} else {
			a.tabScroll = a.tabScrollMax
		}
		a.tabScrollClicked = time.Time{}
	} else {
		a.tabScroll = min(a.tabScrollMax, max(0, a.tabScroll+direction*a.tabStep))
		a.tabScrollClicked = now
	}
	a.tabScrollDirection = direction
}

func (a *App) tabScrollButton(w *desktop.Window, direction int) {
	icon, tip := "tab-right", "Scroll tabs right; double-click for the last tab"
	enabled := a.tabScroll < a.tabScrollMax
	if direction < 0 {
		icon, tip = "tab-left", "Scroll tabs left; double-click for the first tab"
		enabled = a.tabScroll > 0
	}
	p := a.p
	if !enabled {
		p.Muted = p.Faint
	}
	b := w.WidgetBounds()
	if iconButton(w, icon, false, p) && enabled {
		a.scrollTabs(direction, time.Now())
	}
	if w.Input().Mouse.HoveringRect(b) {
		w.Tooltip(tip)
	}
}

// tabDropTarget uses the nearest insertion boundary, then accounts for removing
// the dragged tab. The returned marker uses the same screen coordinates as the
// tab rectangles, including horizontal scrolling and display scaling.
func tabDropTarget(x, first, stride, count, from int) (to, marker int) {
	if stride <= 0 || count <= 0 || from < 0 || from >= count {
		return from, first
	}
	slot := min(count, max(0, (x-first+stride/2)/stride))
	to = slot
	if slot > from {
		to--
	}
	return to, first + slot*stride
}
