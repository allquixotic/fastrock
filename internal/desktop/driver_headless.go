//go:build fltk_headless

package desktop

import "github.com/allquixotic/fastrock/internal/desktop/internal/windowing"

func runDriver(f func(windowing.Display)) {
	panic("native windows are disabled in headless test builds")
}
