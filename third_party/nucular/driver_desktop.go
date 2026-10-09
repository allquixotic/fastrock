//go:build !nucular_headless

package nucular

import (
	"github.com/aarzilli/nucular/internal/ebitenscreen"
	"golang.org/x/exp/shiny/screen"
)

func runDriver(f func(screen.Screen)) { ebitenscreen.Main(f) }
