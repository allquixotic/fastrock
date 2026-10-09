package ui

import (
	"github.com/aarzilli/nucular/rect"
	"strings"
)

func (a *App) modalBounds(width, height int) rect.Rect {
	w, h := a.clientBounds.Dx(), a.clientBounds.Dy()
	if w <= 0 || h <= 0 {
		w, h = 800, 600
	}
	width = min(width, max(120, w-24))
	height = min(height, max(100, h-24))
	return rect.Rect{X: max(0, (w-width)/2), Y: max(0, (h-height)/3), W: width, H: height}
}
func dialogAction(title, fallback string) string {
	for _, verb := range []string{"Go", "Rename", "Delete", "Archive", "Restart", "Run", "Quit", "Close", "Stop", "Sign out", "Save", "Create", "Apply", "Discard"} {
		if strings.HasPrefix(title, verb) {
			return verb
		}
	}
	return fallback
}
