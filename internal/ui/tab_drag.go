package ui

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
