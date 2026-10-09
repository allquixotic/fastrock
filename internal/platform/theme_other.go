//go:build !windows

package platform

import (
	"context"
	"runtime"
	"strings"
)

func SystemLightTheme(ctx context.Context) (bool, error) {
	if runtime.GOOS != "darwin" {
		return true, nil
	}
	// Reading the global domain succeeds even when AppleInterfaceStyle is
	// absent (the system's light appearance); a missing single key does not.
	data, err := CommandContext(ctx, "/usr/bin/defaults", "read", "-g").Output()
	if err != nil {
		return false, err
	}
	for line := range strings.SplitSeq(string(data), "\n") {
		key, value, ok := strings.Cut(line, "=")
		if ok && strings.TrimSpace(key) == "AppleInterfaceStyle" {
			return !strings.Contains(strings.ToLower(value), "dark"), nil
		}
	}
	return true, nil
}
