package codex

import (
	"context"
	"github.com/allquixotic/fastrock/internal/platform"
	"golang.org/x/sys/windows"
	"os/exec"
	"strconv"
	"strings"
	"syscall"
	"unsafe"
)

func command(ctx context.Context, binary string, args ...string) *exec.Cmd {
	var c *exec.Cmd
	if strings.HasSuffix(strings.ToLower(binary), ".cmd") || strings.HasSuffix(strings.ToLower(binary), ".bat") {
		quoted := []string{`"` + strings.ReplaceAll(binary, `"`, ``) + `"`}
		for _, a := range args {
			quoted = append(quoted, `"`+strings.ReplaceAll(a, `"`, ``)+`"`)
		}
		c = platform.CommandContext(ctx, "cmd.exe")
		c.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: windows.CREATE_NO_WINDOW, CmdLine: `cmd.exe /d /s /c "` + strings.Join(quoted, " ") + `"`}
	} else {
		c = platform.CommandContext(ctx, binary, args...)
		c.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: windows.CREATE_NO_WINDOW}
	}
	c.Cancel = func() error {
		if c.Process == nil {
			return nil
		}
		return platform.Command("taskkill.exe", "/PID", strconv.Itoa(c.Process.Pid), "/T", "/F").Run()
	}
	return c
}

// Keep descendants in a kill-on-close Job even if app-server exits before its
// MCP servers. taskkill remains the timeout fallback for the launching process.
func containProcess(c *exec.Cmd) (func(), error) {
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return nil, err
	}
	limits := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	limits.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	if _, err = windows.SetInformationJobObject(job, windows.JobObjectExtendedLimitInformation, uintptr(unsafe.Pointer(&limits)), uint32(unsafe.Sizeof(limits))); err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	process, err := windows.OpenProcess(windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE, false, uint32(c.Process.Pid))
	if err == nil {
		err = windows.AssignProcessToJobObject(job, process)
		windows.CloseHandle(process)
	}
	if err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	return func() { windows.CloseHandle(job) }, nil
}
