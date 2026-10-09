package platform

import (
	"fmt"
	"image"
	"image/png"
	"os"
	"syscall"
	"unsafe"
)

// Screenshot captures this process's visible window with Win32/GDI. It is used
// only by opt-in Windows validation; it needs no PowerShell or C# compiler.
func Screenshot(path string) error {
	user := syscall.NewLazyDLL("user32.dll")
	gdi := syscall.NewLazyDLL("gdi32.dll")
	var hwnd uintptr
	cb := syscall.NewCallback(func(h, unused uintptr) uintptr {
		var pid uint32
		user.NewProc("GetWindowThreadProcessId").Call(h, uintptr(unsafe.Pointer(&pid)))
		visible, _, _ := user.NewProc("IsWindowVisible").Call(h)
		if int(pid) == os.Getpid() && visible != 0 {
			hwnd = h
			return 0
		}
		return 1
	})
	user.NewProc("EnumWindows").Call(cb, 0)
	if hwnd == 0 {
		return fmt.Errorf("capture: no visible Fastrock window")
	}
	var bounds struct{ Left, Top, Right, Bottom int32 }
	ok, _, e := user.NewProc("GetWindowRect").Call(hwnd, uintptr(unsafe.Pointer(&bounds)))
	if ok == 0 {
		return fmt.Errorf("capture window bounds: %w", e)
	}
	width, height := int(bounds.Right-bounds.Left), int(bounds.Bottom-bounds.Top)
	if width <= 0 || height <= 0 || width*height > 32_000_000 {
		return fmt.Errorf("capture: invalid dimensions %dx%d", width, height)
	}
	dc, _, e := user.NewProc("GetDC").Call(0)
	if dc == 0 {
		return e
	}
	defer user.NewProc("ReleaseDC").Call(0, dc)
	memory, _, e := gdi.NewProc("CreateCompatibleDC").Call(dc)
	if memory == 0 {
		return e
	}
	defer gdi.NewProc("DeleteDC").Call(memory)
	info := struct {
		Size                   uint32
		Width, Height          int32
		Planes, Bits           uint16
		Compression, ImageSize uint32
		XPels, YPels           int32
		Used, Important        uint32
	}{Size: 40, Width: int32(width), Height: -int32(height), Planes: 1, Bits: 32}
	var pixels unsafe.Pointer
	bitmap, _, e := gdi.NewProc("CreateDIBSection").Call(dc, uintptr(unsafe.Pointer(&info)), 0, uintptr(unsafe.Pointer(&pixels)), 0, 0)
	if bitmap == 0 {
		return e
	}
	defer gdi.NewProc("DeleteObject").Call(bitmap)
	old, _, _ := gdi.NewProc("SelectObject").Call(memory, bitmap)
	defer gdi.NewProc("SelectObject").Call(memory, old)
	ok, _, e = gdi.NewProc("BitBlt").Call(memory, 0, 0, uintptr(width), uintptr(height), dc, uintptr(bounds.Left), uintptr(bounds.Top), 0x40cc0020)
	if ok == 0 {
		return e
	}
	data := unsafe.Slice((*byte)(pixels), width*height*4)
	img := image.NewRGBA(image.Rect(0, 0, width, height))
	for i := 0; i < len(data); i += 4 {
		img.Pix[i], img.Pix[i+1], img.Pix[i+2], img.Pix[i+3] = data[i+2], data[i+1], data[i], 255
	}
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return png.Encode(f, img)
}
