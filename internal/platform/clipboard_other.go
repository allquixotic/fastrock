//go:build !darwin && !windows

package platform

func ClipboardPNG() ([]byte, error) { return nil, nil }
