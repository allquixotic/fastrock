//go:build !windows

package settings

import "os"

func syncDirectory(path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return f.Sync()
}
func retryRename(error) bool { return false }
