//go:build nucular_headless

package nucular

import "golang.org/x/exp/shiny/screen"

func runDriver(f func(screen.Screen)) { panic("native windows are disabled in headless test builds") }
