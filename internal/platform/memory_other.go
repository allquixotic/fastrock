//go:build !windows && !darwin

package platform

import "runtime"

func ResidentMemory() uint64 {
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	return m.Sys - m.HeapReleased
}
