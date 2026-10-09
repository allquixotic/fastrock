package ui

import "github.com/aarzilli/nucular"

// Estimate only the bounded visible prefix. Drafts can be arbitrarily longer
// than the growing editor; its own scrollbar handles text beyond this height.
func composerHeight(ed *nucular.TextEditor, width, fontSize int) int {
	columns := max(8, width/max(1, fontSize/2))
	lines, column := 1, 0
	for _, r := range ed.Buffer {
		if r == '\n' {
			lines++
			column = 0
		} else {
			column++
			if column >= columns {
				lines++
				column = 0
			}
		}
		if lines >= 10 {
			break
		}
	}
	return min(220, max(44, lines*(fontSize+7)+20))
}
