package platform

import (
	"testing"
	"unsafe"
)

// The extra field must be inside the structure size sent to Windows; using
// PROCESS_MEMORY_COUNTERS without EX cannot return private commit.
func TestV50WindowsMemoryCountersLayout(t *testing.T) {
	var c memoryCounters
	word := unsafe.Sizeof(uintptr(0))
	if unsafe.Sizeof(c) != 8+9*word || unsafe.Offsetof(c.Private) != 8+8*word {
		t.Fatal("PROCESS_MEMORY_COUNTERS_EX ABI mismatch", unsafe.Sizeof(c), unsafe.Offsetof(c.Private))
	}
}
