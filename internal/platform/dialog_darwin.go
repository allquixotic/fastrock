package platform

import (
	"context"
	"fmt"
	"strings"
	"time"
)

func ChooseSaveText(name string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	out, err := CommandContext(ctx, "osascript", "-l", "JavaScript", "-e", textSaveScript(name)).Output()
	if err != nil {
		return "", fmt.Errorf("save dialog failed: %w", err)
	}
	return strings.TrimSuffix(string(out), "\n"), nil
}

func ChoosePath(save, dir bool) (string, error) {
	script := `POSIX path of (choose file with prompt "Select a file")`
	if save {
		script = `POSIX path of (choose file name with prompt "Save file")`
	}
	if dir {
		script = `POSIX path of (choose folder with prompt "Select project folder")`
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	out, e := CommandContext(ctx, "osascript", "-e", script).Output()
	if e != nil {
		return "", nil
	}
	return strings.TrimSpace(string(out)), nil
}
