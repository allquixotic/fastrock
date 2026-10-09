package platform

import (
	"os"
	"runtime"
	"sync"
	"unsafe"

	"github.com/ebitengine/purego"
)

var usageOnce sync.Once
var readUsage func(int32, int32, unsafe.Pointer) int32

func ResidentMemory() uint64 {
	usageOnce.Do(func() {
		lib, err := purego.Dlopen("/usr/lib/libproc.dylib", purego.RTLD_LAZY|purego.RTLD_LOCAL)
		if err == nil {
			purego.RegisterLibFunc(&readUsage, lib, "proc_pid_rusage")
		}
	})
	if readUsage != nil {
		var usage struct {
			UUID                                                                                             [16]byte
			User, System, PackageWakeups, InterruptWakeups, Pageins, Wired, Resident, Footprint, Start, Exit uint64
		}
		if readUsage(int32(os.Getpid()), 0, unsafe.Pointer(&usage)) == 0 {
			return usage.Resident
		}
	}
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	return m.Sys - m.HeapReleased
}
