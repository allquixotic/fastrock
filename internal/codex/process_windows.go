package codex

import (
	"context"
	"os/exec"
	"strings"
	"syscall"
)

func command(ctx context.Context, binary string, args ...string) *exec.Cmd {
	var c *exec.Cmd
	if strings.HasSuffix(strings.ToLower(binary), ".cmd") || strings.HasSuffix(strings.ToLower(binary), ".bat") {
		quoted := []string{`"` + strings.ReplaceAll(binary, `"`, ``) + `"`}
		for _, a := range args {
			quoted = append(quoted, `"`+strings.ReplaceAll(a, `"`, ``)+`"`)
		}
		c = exec.CommandContext(ctx, "cmd.exe")
		c.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: 0x08000000, CmdLine: `cmd.exe /d /s /c "` + strings.Join(quoted, " ") + `"`}
	} else {
		c = exec.CommandContext(ctx, binary, args...)
		c.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: 0x08000000}
	}
	return c
}
