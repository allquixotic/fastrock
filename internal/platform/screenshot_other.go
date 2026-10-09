//go:build !windows

package platform

import "errors"

func Screenshot(path string) error { return errors.New("GUI capture is supported only on Windows") }
