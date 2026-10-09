package platform

import (
	"unsafe"

	"golang.org/x/sys/windows"
)

var memoryInfo = windows.NewLazySystemDLL("kernel32.dll").NewProc("K32GetProcessMemoryInfo")

// Matches PROCESS_MEMORY_COUNTERS_EX, including its final PrivateUsage field.
type memoryCounters struct {
	Size, Faults                                                                                                 uint32
	Peak, Working, QuotaPeakPaged, QuotaPaged, QuotaPeakNonPaged, QuotaNonPaged, Pagefile, PeakPagefile, Private uintptr
}

// ProcessMemoryBytes reports private committed bytes, not working-set size.
func ProcessMemoryBytes() uint64 {
	if err := memoryInfo.Find(); err != nil {
		return managedMemoryBytes()
	}
	var c memoryCounters
	c.Size = uint32(unsafe.Sizeof(c))
	ok, _, _ := memoryInfo.Call(uintptr(windows.CurrentProcess()), uintptr(unsafe.Pointer(&c)), uintptr(c.Size))
	if ok == 0 {
		return managedMemoryBytes()
	}
	return uint64(c.Private)
}
