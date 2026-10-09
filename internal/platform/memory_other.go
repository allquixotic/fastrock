//go:build !windows && !darwin

package platform

// Other platforms report the Go runtime's retained allocation estimate.
func ProcessMemoryBytes() uint64 { return managedMemoryBytes() }
