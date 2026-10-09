package platform

import "runtime"

// Fall back only when native process accounting is unavailable. This estimate
// covers Go-managed memory and is not a native private-memory measurement.
func managedMemoryBytes() uint64 {
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	return m.Sys - m.HeapReleased
}
