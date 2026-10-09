package update

import (
	"encoding/json"
	"errors"
	"fmt"
	"github.com/allquixotic/fastrock/internal/platform"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

type Job struct {
	ParentPID                            int
	Target, Staged, Stage, Version, GOOS string
}

func installTarget(exe string) string {
	if runtime.GOOS == "darwin" && strings.HasSuffix(exe, "/Contents/MacOS/fastrock") {
		return strings.TrimSuffix(exe, "/Contents/MacOS/fastrock")
	}
	return exe
}
func cleanEnvironment() []string { return platform.ChildEnv() }
func startApp(target string) error {
	exe := target
	if strings.HasSuffix(target, ".app") {
		exe = filepath.Join(target, "Contents", "MacOS", "fastrock")
	}
	cmd := exec.Command(exe)
	cmd.Env = cleanEnvironment()
	cmd.Dir = filepath.Dir(target)
	detach(cmd)
	if e := cmd.Start(); e != nil {
		return e
	}
	return cmd.Process.Release()
}

// LaunchHelper copies itself outside the installation being replaced. It is
// called only after the broker has observed the last window disconnect.
func LaunchHelper(jobFile string) error {
	exe, e := os.Executable()
	if e != nil {
		return e
	}
	helper := filepath.Join(filepath.Dir(jobFile), "update-helper")
	if runtime.GOOS == "windows" {
		helper += ".exe"
	}
	if e = copyFile(exe, helper, 0700); e != nil {
		return e
	}
	cmd := exec.Command(helper, "--apply-update", jobFile)
	cmd.Env = cleanEnvironment()
	detach(cmd)
	if e = cmd.Start(); e != nil {
		return e
	}
	return cmd.Process.Release()
}

// Apply runs in the isolated helper before settings, Codex, or UI initialization.
func Apply(jobFile string) error {
	data, e := os.ReadFile(jobFile)
	if e != nil {
		return e
	}
	if len(data) > 64<<10 {
		return errors.New("invalid update job")
	}
	var job Job
	if e = json.Unmarshal(data, &job); e != nil {
		return e
	}
	if job.ParentPID <= 0 || job.GOOS != runtime.GOOS || !IsRelease(job.Version) || !filepath.IsAbs(job.Target) || filepath.Clean(job.Stage) != filepath.Dir(jobFile) || filepath.Dir(filepath.Dir(job.Staged)) != job.Stage {
		return errors.New("invalid update job paths")
	}
	fail := func(err error) error {
		result, _ := json.Marshal(Status{State: "error", Version: job.Version, Message: "The update could not be installed: " + err.Error()})
		_ = os.WriteFile(filepath.Join(filepath.Dir(job.Stage), "last-update.json"), result, 0600)
		return err
	}
	if e = waitProcess(job.ParentPID, 10*time.Minute); e != nil {
		return fail(e)
	}
	if e = validatePayload(job.Staged, job.GOOS); e != nil {
		return fail(e)
	}
	// Test writability before moving the old installation. This also puts all
	// renames on one filesystem, even if the download cache is on another volume.
	local, e := os.MkdirTemp(filepath.Dir(job.Target), ".fastrock-update-")
	if e != nil {
		return fail(fmt.Errorf("installation is not writable; move Fastrock to a user-owned Applications folder: %w", e))
	}
	keepRecovery := false
	defer func() {
		if keepRecovery {
			return
		}
		// Keep the rollback copy if an external lock prevented recovery.
		if _, err := os.Stat(filepath.Join(local, "previous")); os.IsNotExist(err) {
			_ = os.RemoveAll(local)
		}
	}()
	next := filepath.Join(local, filepath.Base(job.Target))
	if e = copyTree(job.Staged, next); e != nil {
		return fail(e)
	}
	if e = verifyRelease(next, job.Version); e != nil {
		return fail(e)
	}
	backup := filepath.Join(local, "previous")
	if e = replace(job.Target, next, backup); e != nil {
		return fail(e)
	}
	if e = startApp(job.Target); e != nil {
		rollback := os.Rename(job.Target, next)
		if rollback == nil {
			rollback = os.Rename(backup, job.Target)
		}
		if rollback != nil { // Retain the backup if rollback fails for external reasons.
			keepRecovery = true
			_ = os.Rename(backup, job.Target+".recovery")
			return fail(fmt.Errorf("restart failed: %v; rollback failed: %v", e, rollback))
		}
		_ = startApp(job.Target)
		return fail(fmt.Errorf("restart failed; restored previous version: %w", e))
	}
	_ = os.Remove(filepath.Join(filepath.Dir(job.Stage), "last-update.json"))
	_ = os.WriteFile(filepath.Join(job.Stage, ".complete"), nil, 0600)
	// Windows cannot unlink its running helper; a later startup prunes this stage.
	_ = os.RemoveAll(backup)
	_ = os.RemoveAll(job.Stage)
	return nil
}
func replace(target, next, backup string) error {
	// A second independent instance may still hold the Windows image. Retrying
	// never deletes it, and leaves the old app untouched when the deadline expires.
	deadline := time.Now().Add(30 * time.Second)
	for {
		e := os.Rename(target, backup)
		if e == nil {
			break
		}
		if runtime.GOOS != "windows" || time.Now().After(deadline) {
			return e
		}
		time.Sleep(200 * time.Millisecond)
	}
	if e := os.Rename(next, target); e != nil {
		if rollback := os.Rename(backup, target); rollback != nil {
			return fmt.Errorf("install failed: %v; previous version remains at %s: %v", e, backup, rollback)
		}
		return e
	}
	return nil
}
func copyFile(src, dst string, mode os.FileMode) error {
	in, e := os.Open(src)
	if e != nil {
		return e
	}
	defer in.Close()
	out, e := os.OpenFile(dst, os.O_WRONLY|os.O_CREATE|os.O_EXCL, mode)
	if e != nil {
		return e
	}
	_, e = io.Copy(out, in)
	if e == nil {
		e = out.Sync()
	}
	ce := out.Close()
	if e != nil {
		return e
	}
	return ce
}
func copyTree(src, dst string) error {
	return filepath.WalkDir(src, func(path string, d os.DirEntry, e error) error {
		if e != nil {
			return e
		}
		rel, e := filepath.Rel(src, path)
		if e != nil {
			return e
		}
		target := filepath.Join(dst, rel)
		if d.Type()&os.ModeSymlink != 0 {
			return errors.New("symlink in staged update")
		}
		if d.IsDir() {
			return os.MkdirAll(target, 0700)
		}
		info, e := d.Info()
		if e != nil {
			return e
		}
		return copyFile(path, target, info.Mode().Perm())
	})
}
