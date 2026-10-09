package platform

import (
	"bytes"
	"fmt"
	"image"
	"image/png"
	"runtime"
	"syscall"
	"unsafe"
)

// ClipboardPNG copies a bitmap from the Windows clipboard through system APIs.
// The clipboard is released before PNG encoding. No external helper is used.
func ClipboardPNG() ([]byte, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	user, gdi := syscall.NewLazyDLL("user32.dll"), syscall.NewLazyDLL("gdi32.dll")
	ok, _, _ := user.NewProc("IsClipboardFormatAvailable").Call(2)
	if ok == 0 {
		return nil, nil
	}
	ok, _, err := user.NewProc("OpenClipboard").Call(0)
	if ok == 0 {
		return nil, fmt.Errorf("open clipboard: %w", err)
	}
	clipboardOpen := true
	defer func() {
		if clipboardOpen {
			user.NewProc("CloseClipboard").Call()
		}
	}()
	bitmap, _, err := user.NewProc("GetClipboardData").Call(2)
	if bitmap == 0 {
		return nil, err
	}
	var object struct {
		Type, Width, Height, Stride int32
		Planes, Bits                uint16
		Pixels                      uintptr
	}
	ok, _, err = gdi.NewProc("GetObjectW").Call(bitmap, unsafe.Sizeof(object), uintptr(unsafe.Pointer(&object)))
	if ok == 0 {
		return nil, err
	}
	w, h := int(object.Width), int(object.Height)
	if w <= 0 || h <= 0 || w > 8192 || h > 8192 || w*h > 16_000_000 {
		return nil, fmt.Errorf("clipboard image exceeds 16 megapixels")
	}
	info := struct {
		Size                   uint32
		Width, Height          int32
		Planes, Bits           uint16
		Compression, ImageSize uint32
		XPels, YPels           int32
		Used, Important        uint32
	}{Size: 40, Width: int32(w), Height: -int32(h), Planes: 1, Bits: 32}
	data := make([]byte, w*h*4)
	dc, _, err := gdi.NewProc("CreateCompatibleDC").Call(0)
	if dc == 0 {
		return nil, err
	}
	defer gdi.NewProc("DeleteDC").Call(dc)
	ok, _, err = gdi.NewProc("GetDIBits").Call(dc, bitmap, 0, uintptr(h), uintptr(unsafe.Pointer(&data[0])), uintptr(unsafe.Pointer(&info)), 0)
	if ok == 0 {
		return nil, err
	}
	user.NewProc("CloseClipboard").Call()
	clipboardOpen = false
	for i := 0; i < len(data); i += 4 {
		data[i], data[i+2] = data[i+2], data[i]
		data[i+3] = 255
	}
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, &image.RGBA{Pix: data, Stride: w * 4, Rect: image.Rect(0, 0, w, h)}); err != nil {
		return nil, err
	}
	return encoded.Bytes(), nil
}
