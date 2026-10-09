//go:build !windows

package codex

import (
	"context"
	"github.com/allquixotic/fastrock/internal/platform"
	"os/exec"
	"syscall"
)

func command(ctx context.Context, binary string, args ...string) *exec.Cmd {
	c := platform.CommandContext(ctx, binary, args...)
	c.Env = codexChildEnv()
	c.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	c.Cancel = func() error {
		if c.Process == nil {
			return nil
		}
		return syscall.Kill(-c.Process.Pid, syscall.SIGKILL)
	}
	return c
}

func containProcess(c *exec.Cmd) (func(), error) { return func() {}, nil }
