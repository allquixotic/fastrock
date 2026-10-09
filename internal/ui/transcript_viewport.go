package ui

import "sort"

func visibleTranscriptLines(layout *transcriptLayout, top, clipTop, clipBottom int, scale float64, spacing int) (first, last, leading, trailing int) {
	n := len(layout.Lines)
	if len(layout.LineOffsets) != n || n == 0 {
		return 0, n, 0, 0
	}
	lineTop := func(i int) int {
		if i == n {
			return int(float64(layout.Height)*scale) + n*spacing
		}
		return int(float64(layout.LineOffsets[i])*scale) + i*spacing
	}
	first = sort.Search(n, func(i int) bool { return top+lineTop(i+1) > clipTop })
	last = sort.Search(n, func(i int) bool { return top+lineTop(i) >= clipBottom })
	last = max(first, last)
	leading = lineTop(first)
	trailing = lineTop(n) - lineTop(last)
	return
}
