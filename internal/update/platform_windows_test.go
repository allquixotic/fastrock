package update

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestV15WindowsWaitsForExecutableExit(t *testing.T) {
	if os.Getenv("FASTROCK_UPDATE_TEST_CHILD") == "yes" {
		time.Sleep(450 * time.Millisecond)
		return
	}
	exe, e := os.Executable()
	if e != nil {
		t.Fatal(e)
	}
	dir := t.TempDir()
	target := filepath.Join(dir, "old.exe")
	if e = copyFile(exe, target, 0700); e != nil {
		t.Fatal(e)
	}
	cmd := exec.Command(target, "-test.run=TestV15WindowsWaitsForExecutableExit")
	cmd.Env = append(os.Environ(), "FASTROCK_UPDATE_TEST_CHILD=yes")
	if e = cmd.Start(); e != nil {
		t.Fatal(e)
	}
	started := time.Now()
	if e = waitProcess(cmd.Process.Pid, 10*time.Second); e != nil {
		t.Fatal(e)
	}
	cmd.Wait()
	if time.Since(started) < 300*time.Millisecond {
		t.Fatal("did not wait for live executable")
	}
	next := filepath.Join(dir, "next.exe")
	os.WriteFile(next, []byte("new"), 0600)
	if e = replace(target, next, filepath.Join(dir, "backup.exe")); e != nil {
		t.Fatal(e)
	}
}
func TestV15WindowsPerUserShortcut(t *testing.T) {
	root := t.TempDir()
	exe := filepath.Join(root, "fastrock.exe")
	os.WriteFile(exe, []byte("fixture"), 0600)
	path := filepath.Join(root, "Fastrock.lnk")
	if e := createShortcutAt(exe, root); e != nil {
		t.Fatal(e)
	}
	b, e := os.ReadFile(path)
	if e != nil {
		t.Fatal(e)
	}
	if len(b) < 76 || b[0] != 0x4c {
		t.Fatal("invalid shell link")
	}
	// IShellLink writes the Unicode target; verify the fixture path is present.
	var decoded strings.Builder
	for i := 0; i+1 < len(b); i += 2 {
		decoded.WriteRune(rune(uint16(b[i]) | uint16(b[i+1])<<8))
	}
	if !strings.Contains(decoded.String(), "fastrock.exe") && !strings.Contains(string(b), "fastrock.exe") {
		t.Fatal("shortcut lost executable target")
	}
}
