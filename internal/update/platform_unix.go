//go:build !windows

package update

import (
	"fmt"
	"github.com/allquixotic/fastrock/internal/platform"
	"os/exec"
	"runtime"
	"syscall"
	"time"
)

func detach(cmd *exec.Cmd)    { cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true} }
func detachApp(cmd *exec.Cmd) { detach(cmd) }
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
	// Require our Developer ID identity, not merely a valid/ad-hoc code seal.
	const requirement = `anchor apple generic and identifier "com.allquixotic.fastrock" and certificate leaf[subject.OU] = "B6XDYNLMPU" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists`
	if output, e := platform.Command("/usr/bin/codesign", "--verify", "--deep", "--strict", "-R", "="+requirement, path).CombinedOutput(); e != nil {
		return fmt.Errorf("app signature verification failed: %s", output)
	}
	if output, e := platform.Command("/usr/bin/xcrun", "stapler", "validate", path).CombinedOutput(); e != nil {
		return fmt.Errorf("app notarization ticket verification failed: %s", output)
	}
	if output, e := platform.Command("/usr/sbin/spctl", "--assess", "--type", "execute", path).CombinedOutput(); e != nil {
		return fmt.Errorf("Gatekeeper rejected update: %s", output)
	}
	return nil
}

// PrepareInstallation is deliberately a no-op on macOS: the user drags the DMG
// app to /Applications or ~/Applications. Never prompt for administrator rights.
func PrepareInstallation(exe string) (bool, error) { return false, nil }
