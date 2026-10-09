// Copyright 2013 @atotto. All rights reserved.
// Use of this source code is governed by a BSD-style
// license that can be found in the LICENSE file.

package clipboard

import (
	"fmt"
	"os"
	"syscall"
	"unsafe"
)

const (
	cfUnicodetext = 13
	gmemMoveable  = 0x0002
)

var (
	user32           = syscall.MustLoadDLL("user32")
	openClipboard    = user32.MustFindProc("OpenClipboard")
	closeClipboard   = user32.MustFindProc("CloseClipboard")
	emptyClipboard   = user32.MustFindProc("EmptyClipboard")
	getClipboardData = user32.MustFindProc("GetClipboardData")
	setClipboardData = user32.MustFindProc("SetClipboardData")

	kernel32     = syscall.NewLazyDLL("kernel32")
	globalAlloc  = kernel32.NewProc("GlobalAlloc")
	globalFree   = kernel32.NewProc("GlobalFree")
	globalLock   = kernel32.NewProc("GlobalLock")
	globalUnlock = kernel32.NewProc("GlobalUnlock")
	globalSize   = kernel32.NewProc("GlobalSize")
)

func readAll() (string, error) {
	r, _, err := openClipboard.Call(0)
	if r == 0 {
		return "", err
	}
	defer closeClipboard.Call()

	h, _, err := getClipboardData.Call(cfUnicodetext)
	if h == 0 {
		return "", err
	}

	l, _, err := globalLock.Call(h)
	if l == 0 {
		return "", err
	}

	size, _, _ := globalSize.Call(h)
	if size > 32<<20 {
		globalUnlock.Call(h)
		return "", fmt.Errorf("clipboard text exceeds 32 MiB")
	}
	text := syscall.UTF16ToString(unsafe.Slice((*uint16)(unsafe.Pointer(l)), int(size/2)))
	globalUnlock.Call(h)

	return text, nil
}

func writeAll(text string) error {
	r, _, err := openClipboard.Call(0)
	if r == 0 {
		return err
	}
	defer closeClipboard.Call()

	r, _, err = emptyClipboard.Call(0)
	if r == 0 {
		return err
	}

	data := syscall.StringToUTF16(text)

	h, _, err := globalAlloc.Call(gmemMoveable, uintptr(len(data)*int(unsafe.Sizeof(data[0]))))
	if h == 0 {
		return err
	}

	l, _, err := globalLock.Call(h)
	if l == 0 {
		globalFree.Call(h)
		return err
	}

	copy(unsafe.Slice((*uint16)(unsafe.Pointer(l)), len(data)), data)
	globalUnlock.Call(h)

	r, _, err = setClipboardData.Call(cfUnicodetext, h)
	if r == 0 {
		globalFree.Call(h)
		return err
	}
	return nil
}

func Start() {
}

func Get() string {
	str, err := readAll()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return ""
	}
	return str
}

func GetPrimary() string {
	return ""
}

func Set(text string) {
	err := writeAll(text)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
	}
}
