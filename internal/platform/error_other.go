//go:build !windows && !darwin

package platform

import (
	"fmt"
	"os"
)

func ShowError(message string) { fmt.Fprintln(os.Stderr, message) }
