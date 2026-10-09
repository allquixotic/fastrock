package platform

import (
	"sync"
	"testing"
)

func TestV50ProcessMemoryConcurrent(t *testing.T) {
	var workers sync.WaitGroup
	for range 8 {
		workers.Go(func() {
			for range 4 {
				if n := ProcessMemoryBytes(); n == 0 {
					t.Error("process memory accounting returned zero")
				}
			}
		})
	}
	workers.Wait()
}
