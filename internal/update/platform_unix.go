//go:build !windows

package update

import (
	"fmt"
	"os/exec"
	"runtime"
	"syscall"
	"time"
)

func detach(cmd *exec.Cmd) { cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true} }
func waitProcess(pid int, timeout time.Duration) error {
	until := time.Now().Add(timeout)
	for time.Now().Before(until) {
		e := syscall.Kill(pid, 0)
		if e == syscall.ESRCH {
			return nil
		}
		if e != nil {
			return e
		}
		time.Sleep(100 * time.Millisecond)
	}
	return fmt.Errorf("previous Fastrock process has not exited")
}
func verifyPlatform(path string) error {
	if runtime.GOOS != "darwin" {
		return nil
	}
	// Verify the bundle assembled by the release workflow, including its code seal.
	if output, e := exec.Command("/usr/bin/codesign", "--verify", "--deep", "--strict", path).CombinedOutput(); e != nil {
		return fmt.Errorf("app signature verification failed: %s", output)
	}
	return nil
}

// PrepareInstallation is deliberately a no-op on macOS: the user drags the DMG
// app to /Applications or ~/Applications. Never prompt for administrator rights.
func PrepareInstallation(exe string) (bool, error) { return false, nil }
