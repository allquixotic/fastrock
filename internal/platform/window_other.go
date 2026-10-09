//go:build !windows

package platform

import "image"

func WindowSize() image.Point { return image.Pt(1280, 800) }
