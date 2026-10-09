//go:build !windows && !darwin

package platform

import (
	"os/exec"
	"strings"
)

func ChoosePath(save, dir bool) (string, error) {
	args := []string{"--file-selection"}
	if save {
		args = append(args, "--save")
	}
	if dir {
		args = append(args, "--directory")
	}
	out, e := exec.Command("zenity", args...).Output()
	if e != nil {
		return "", nil
	}
	return strings.TrimSpace(string(out)), nil
}
