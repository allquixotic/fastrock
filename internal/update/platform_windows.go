package update

import (
	"errors"
	"fmt"
	"golang.org/x/sys/windows"
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
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: windows.CREATE_NEW_PROCESS_GROUP | windows.DETACHED_PROCESS}
}
func detachApp(cmd *exec.Cmd) {
	detach(cmd)
	// SW_HIDE also hides the GUI's first ShowWindow call, preventing presentation.
	cmd.SysProcAttr.HideWindow = false
}
func waitProcess(pid int, timeout time.Duration) error {
	h, e := windows.OpenProcess(windows.SYNCHRONIZE, false, uint32(pid))
	if e != nil {
		if errors.Is(e, windows.ERROR_INVALID_PARAMETER) {
			return nil
		}
		return e
	}
	defer windows.CloseHandle(h)
	result, e := windows.WaitForSingleObject(h, uint32(timeout.Milliseconds()))
	if result == windows.WAIT_OBJECT_0 {
		return nil
	}
	return fmt.Errorf("waiting for old process: result %d (%v)", result, e)
}

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
	folder, err := windows.KnownFolderPath(windows.FOLDERID_Programs, windows.KF_FLAG_DEFAULT)
	if err != nil {
		return fmt.Errorf("find user Start menu: %w", err)
	}
	return createShortcutAt(exe, folder)
}
func createShortcutAt(exe, folder string) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	// CoInitializeEx returns S_FALSE (1) when this thread already owns an
	// apartment. It is still a successful, balanced initialization.
	const sFalse syscall.Errno = 1
	if err := windows.CoInitializeEx(0, windows.COINIT_APARTMENTTHREADED); err != nil && err != sFalse {
		return fmt.Errorf("initialize shortcut COM: %w", err)
	}
	defer windows.CoUninitialize()
	// x/sys does not wrap CoCreateInstance; restrict its lookup to System32.
	ole := windows.NewLazySystemDLL("ole32.dll")
	clsid := guid{0x00021401, 0, 0, [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	iid := guid{0x000214f9, 0, 0, [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	persistIID := guid{0x0000010b, 0, 0, [8]byte{0xc0, 0, 0, 0, 0, 0, 0, 0x46}}
	var link, persist *comObject
	hr, _, _ := ole.NewProc("CoCreateInstance").Call(uintptr(unsafe.Pointer(&clsid)), 0, windows.CLSCTX_INPROC_SERVER, uintptr(unsafe.Pointer(&iid)), uintptr(unsafe.Pointer(&link)))
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
	path := filepath.Join(folder, "Fastrock.lnk")
	shortcut, _ := syscall.UTF16PtrFromString(path)
	e = callCOM(persist, 6, uintptr(unsafe.Pointer(shortcut)), 1)
	runtime.KeepAlive(target)
	runtime.KeepAlive(dir)
	runtime.KeepAlive(description)
	runtime.KeepAlive(shortcut)
	return e
}
