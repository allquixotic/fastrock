//go:build !windows && !darwin

package platform

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"time"
)

func ChooseSaveText(name string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	out, err := CommandContext(ctx, "zenity", textSaveArgs(name)...).Output()
	if err != nil {
		var exit *exec.ExitError
		if errors.As(err, &exit) && exit.ExitCode() == 1 && ctx.Err() == nil {
			return "", nil // Cancel.
		}
		return "", fmt.Errorf("save dialog failed: %w", err)
	}
	return strings.TrimSuffix(string(out), "\n"), nil
}

func ChoosePath(save, dir bool) (string, error) {
	args := []string{"--file-selection"}
	if save {
		args = append(args, "--save")
	}
	if dir {
		args = append(args, "--directory")
	}
	out, e := Command("zenity", args...).Output()
	if e != nil {
		return "", nil
	}
	return strings.TrimSpace(string(out)), nil
}
