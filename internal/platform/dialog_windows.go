package platform

import (
	"fmt"
	"runtime"
	"syscall"
	"unsafe"
)

type openFileName struct {
	Size                         uint32
	Owner, Instance              uintptr
	Filter, CustomFilter         *uint16
	MaxCustomFilter, FilterIndex uint32
	File                         *uint16
	MaxFile                      uint32
	FileTitle                    *uint16
	MaxFileTitle                 uint32
	InitialDir, Title            *uint16
	Flags                        uint32
	FileOffset, FileExtension    uint16
	DefaultExtension             *uint16
	CustomData                   uintptr
	Hook                         uintptr
	Template                     *uint16
	Reserved                     uintptr
	ReservedSize, FlagsEx        uint32
}

func ChoosePath(save, dir bool) (string, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	ole := syscall.NewLazyDLL("ole32.dll")
	hr, _, _ := ole.NewProc("CoInitializeEx").Call(0, 2)
	if hr == 0 || hr == 1 {
		defer ole.NewProc("CoUninitialize").Call()
	}
	buffer := make([]uint16, 32768)
	if dir {
		title, _ := syscall.UTF16PtrFromString("Choose project folder")
		info := struct {
			Owner, Root     uintptr
			Display, Title  *uint16
			Flags           uint32
			Callback, Param uintptr
			Image           int32
		}{Display: &buffer[0], Title: title, Flags: 0x41}
		shell := syscall.NewLazyDLL("shell32.dll")
		pidl, _, _ := shell.NewProc("SHBrowseForFolderW").Call(uintptr(unsafe.Pointer(&info)))
		if pidl == 0 {
			return "", nil
		}
		defer ole.NewProc("CoTaskMemFree").Call(pidl)
		ok, _, _ := shell.NewProc("SHGetPathFromIDListEx").Call(pidl, uintptr(unsafe.Pointer(&buffer[0])), uintptr(len(buffer)), 0)
		if ok == 0 {
			return "", fmt.Errorf("the selected item is not a filesystem folder")
		}
		return syscall.UTF16ToString(buffer), nil
	}
	filter := []uint16{'A', 'l', 'l', ' ', 'f', 'i', 'l', 'e', 's', 0, '*', '.', '*', 0, 0}
	info := openFileName{Filter: &filter[0], File: &buffer[0], MaxFile: uint32(len(buffer)), Flags: 0x80000 | 0x8 | 0x800}
	info.Size = uint32(unsafe.Sizeof(info))
	api := syscall.NewLazyDLL("comdlg32.dll")
	method := "GetOpenFileNameW"
	if save {
		method = "GetSaveFileNameW"
		info.Flags |= 0x2
	} else {
		info.Flags |= 0x1000
	}
	ok, _, _ := api.NewProc(method).Call(uintptr(unsafe.Pointer(&info)))
	runtime.KeepAlive(filter)
	runtime.KeepAlive(buffer)
	if ok == 0 {
		code, _, _ := api.NewProc("CommDlgExtendedError").Call()
		if code != 0 {
			return "", fmt.Errorf("file dialog failed: 0x%x", code)
		}
		return "", nil
	}
	return syscall.UTF16ToString(buffer), nil
}
