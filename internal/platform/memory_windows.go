package platform

import (
	"syscall"
	"unsafe"
)

func ResidentMemory() uint64 {
	type counters struct {
		Size, Faults                                                                                        uint32
		Peak, Working, QuotaPeakPaged, QuotaPaged, QuotaPeakNonPaged, QuotaNonPaged, Pagefile, PeakPagefile uintptr
	}
	var c counters
	c.Size = uint32(unsafe.Sizeof(c))
	kernel := syscall.NewLazyDLL("kernel32.dll")
	process, _, _ := kernel.NewProc("GetCurrentProcess").Call()
	ok, _, _ := kernel.NewProc("K32GetProcessMemoryInfo").Call(process, uintptr(unsafe.Pointer(&c)), uintptr(c.Size))
	if ok == 0 {
		return 0
	}
	return uint64(c.Working)
}
