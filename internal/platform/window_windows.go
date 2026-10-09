package platform

import (
	"image"
	"syscall"
	"unsafe"
)

func WindowSize() image.Point {
	var r struct{ Left, Top, Right, Bottom int32 }
	ok, _, _ := syscall.NewLazyDLL("user32.dll").NewProc("SystemParametersInfoW").Call(0x30, 0, uintptr(unsafe.Pointer(&r)), 0)
	if ok != 0 {
		return image.Pt(min(1360, max(900, int(r.Right-r.Left)-60)), min(800, max(560, int(r.Bottom-r.Top)-80)))
	}
	return image.Pt(1280, 720)
}
