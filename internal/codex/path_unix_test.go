//go:build !windows

package codex

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLookPathKeepsLoginPathInChildEnvironment(t *testing.T) {
	dir := t.TempDir()
	bin := filepath.Join(dir, "bin")
	if e := os.Mkdir(bin, 0o755); e != nil {
		t.Fatal(e)
	}
	if e := os.WriteFile(filepath.Join(bin, "codex"), []byte("#!/bin/sh\n"), 0o755); e != nil {
		t.Fatal(e)
	}
	shell := filepath.Join(dir, "shell")
	script := "#!/bin/sh\necho 'profile banner'\nPATH=" + bin + ":/usr/bin\nexport PATH\nexec /bin/sh -c \"$2\"\n"
	if e := os.WriteFile(shell, []byte(script), 0o755); e != nil {
		t.Fatal(e)
	}
	t.Setenv("SHELL", shell)
	t.Setenv("PATH", "/usr/bin:/bin")
	binary, e := lookPath()
	if e != nil {
		t.Fatal(e)
	}
	if binary != filepath.Join(bin, "codex") {
		t.Fatalf("binary = %q", binary)
	}
	if got, want := os.Getenv("PATH"), "/usr/bin:/bin"; got != want {
		t.Fatalf("PATH = %q, want %q", got, want)
	}
	found := false
	for _, item := range codexChildEnv() {
		if item == "PATH="+bin+":/usr/bin:/bin" {
			found = true
		}
	}
	if !found {
		t.Fatal("Codex child did not receive the resolved PATH")
	}
}
