package update

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"
	"unsafe"
)

func detach(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: 0x00000200 | 0x00000008}
}
func waitProcess(pid int, timeout time.Duration) error {
	k := syscall.NewLazyDLL("kernel32.dll")
	h, _, e := k.NewProc("OpenProcess").Call(0x00100000, 0, uintptr(pid))
	if h == 0 {
		if e == syscall.Errno(87) {
			return nil
		}
		return e
	}
	defer k.NewProc("CloseHandle").Call(h)
	result, _, e := k.NewProc("WaitForSingleObject").Call(h, uintptr(timeout.Milliseconds()))
	if result == 0 {
		return nil
	}
	return fmt.Errorf("waiting for old process: result %d (%v)", result, e)
}
func verifyPlatform(string) error { return nil }

// PrepareInstallation establishes a stable, writable per-user update target.
func PrepareInstallation(exe string) (bool, error) {
	base := os.Getenv("LOCALAPPDATA")
	if base == "" {
		return false, errors.New("LOCALAPPDATA is unavailable")
	}
	target := filepath.Join(base, "Programs", "Fastrock", "fastrock.exe")
	if e := os.MkdirAll(filepath.Dir(target), 0700); e != nil {
		return false, e
	}
	same := strings.EqualFold(filepath.Clean(exe), filepath.Clean(target))
	if !same {
		if _, e := os.Stat(target); errors.Is(e, os.ErrNotExist) {
			temp, e := os.MkdirTemp(filepath.Dir(target), ".install-")
			if e != nil {
				return false, e
			}
			defer os.RemoveAll(temp)
			file := filepath.Join(temp, "fastrock.exe")
			if e = copyFile(exe, file, 0700); e != nil {
				return false, e
			}
			if e = os.Rename(file, target); e != nil {
				return false, e
			}
		}
	}
	if e := createShortcut(target); e != nil {
		return false, e
	}
	if !same {
		return true, startApp(target)
	}
	return false, nil
}

type guid struct {
	A    uint32
	B, C uint16
	D    [8]byte
}
type comObject struct{ V *[22]uintptr }

func callCOM(obj *comObject, index int, args ...uintptr) error {
	params := append([]uintptr{uintptr(unsafe.Pointer(obj))}, args...)
	hr, _, _ := syscall.SyscallN(obj.V[index], params...)
	if int32(hr) < 0 {
		return fmt.Errorf("Windows shortcut HRESULT 0x%x", hr)
	}
	return nil
}
func createShortcut(exe string) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	ole := syscall.NewLazyDLL("ole32.dll")
	hr, _, _ := ole.NewProc("CoInitializeEx").Call(0, 2)
	if int32(hr) < 0 {
		return fmt.Errorf("initialize shortcut COM: 0x%x", hr)
	}
	defer ole.NewProc("CoUninitialize").Call()
	clsid := guid{0x00021401, 0, 0, [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	iid := guid{0x000214f9, 0, 0, [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	persistIID := guid{0x0000010b, 0, 0, [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	var link, persist *comObject
	hr, _, _ = ole.NewProc("CoCreateInstance").Call(uintptr(unsafe.Pointer(&clsid)), 0, 1, uintptr(unsafe.Pointer(&iid)), uintptr(unsafe.Pointer(&link)))
	if int32(hr) < 0 {
		return fmt.Errorf("create shortcut: 0x%x", hr)
	}
	defer callCOM(link, 2)
	target, e := syscall.UTF16PtrFromString(exe)
	if e != nil {
		return e
	}
	// IShellLinkW::SetPath, SetWorkingDirectory and SetDescription.
	if e = callCOM(link, 20, uintptr(unsafe.Pointer(target))); e != nil {
		return e
	}
	dir, _ := syscall.UTF16PtrFromString(filepath.Dir(exe))
	if e = callCOM(link, 9, uintptr(unsafe.Pointer(dir))); e != nil {
		return e
	}
	description, _ := syscall.UTF16PtrFromString("Fastrock — Codex and Rally")
	if e = callCOM(link, 7, uintptr(unsafe.Pointer(description))); e != nil {
		return e
	}
	if e = callCOM(link, 0, uintptr(unsafe.Pointer(&persistIID)), uintptr(unsafe.Pointer(&persist))); e != nil {
		return e
	}
	defer callCOM(persist, 2)
	// FOLDERID_Programs resolves redirected/OneDrive profiles correctly.
	programsID := guid{0xa77f5d77, 0x2e2b, 0x44c3, [8]byte{0xa6, 0xa2, 0xab, 0xa6, 0x01, 0x05, 0x4a, 0x51}}
	var folder *uint16
	hr, _, _ = syscall.NewLazyDLL("shell32.dll").NewProc("SHGetKnownFolderPath").Call(uintptr(unsafe.Pointer(&programsID)), 0, 0, uintptr(unsafe.Pointer(&folder)))
	if int32(hr) < 0 {
		return fmt.Errorf("find user Start menu: 0x%x", hr)
	}
	defer ole.NewProc("CoTaskMemFree").Call(uintptr(unsafe.Pointer(folder)))
	// The Windows API returns a null-terminated string allocated by COM.
	chars := unsafe.Slice(folder, 32768)
	end := 0
	for end < len(chars) && chars[end] != 0 {
		end++
	}
	path := filepath.Join(syscall.UTF16ToString(chars[:end]), "Fastrock.lnk")
	shortcut, _ := syscall.UTF16PtrFromString(path)
	e = callCOM(persist, 6, uintptr(unsafe.Pointer(shortcut)), 1)
	runtime.KeepAlive(target)
	runtime.KeepAlive(dir)
	runtime.KeepAlive(description)
	runtime.KeepAlive(shortcut)
	return e
}
