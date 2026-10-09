package settings

import (
	"errors"
	"golang.org/x/sys/windows"
)

func syncDirectory(string) error { return nil } // File contents were flushed before atomic replacement.
func retryRename(err error) bool {
	return errors.Is(err, windows.ERROR_SHARING_VIOLATION) || errors.Is(err, windows.ERROR_ACCESS_DENIED)
}
