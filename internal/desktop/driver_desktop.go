//go:build !fltk_headless

package desktop

import "github.com/allquixotic/fastrock/internal/desktop/internal/fltkdriver"

var runDriver = fltkdriver.Main
