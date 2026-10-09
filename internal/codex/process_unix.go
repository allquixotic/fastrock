//go:build !windows

package codex

import (
	"context"
	"os/exec"
)

func command(ctx context.Context, binary string, args ...string) *exec.Cmd {
	return exec.CommandContext(ctx, binary, args...)
}
