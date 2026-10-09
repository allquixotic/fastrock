//go:build !windows

package codex

import (
	"context"
	"github.com/allquixotic/fastrock/internal/platform"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const pathMarker = "__FASTROCK_PATH__"

var cachedLoginPath = sync.OnceValue(loginShellPath)
var childPath atomic.Value

// lookPath finds Codex. Apps opened from Finder, the Dock or a desktop launcher
// do not inherit the PATH set in shell profiles, so on failure it adopts the
// login shell's PATH for the Codex child. Never change the GUI process's PATH.
func lookPath() (string, error) {
	binary, e := exec.LookPath("codex")
	if e == nil {
		return binary, nil
	}
	path := cachedLoginPath()
	if path == "" {
		return "", e
	}
	path = mergePath(path, os.Getenv("PATH"))
	for _, dir := range filepath.SplitList(path) {
		candidate := filepath.Join(dir, "codex")
		if info, err := os.Stat(candidate); err == nil && info.Mode().IsRegular() && info.Mode().Perm()&0111 != 0 {
			childPath.Store(path)
			return candidate, nil
		}
	}
	return "", e
}

func codexChildEnv() []string {
	env := platform.ChildEnv()
	if path, ok := childPath.Load().(string); ok {
		for i, item := range env {
			if strings.HasPrefix(item, "PATH=") {
				env[i] = "PATH=" + path
				return env
			}
		}
		env = append(env, "PATH="+path)
	}
	return env
}

func loginShellPath() string {
	shell := os.Getenv("SHELL")
	if !filepath.IsAbs(shell) {
		shell = "/bin/sh"
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	cmd := platform.CommandContext(ctx, shell, "-lc", `printf '`+pathMarker+`%s`+pathMarker+`' "$PATH"`)
	cmd.WaitDelay = time.Second
	out, e := cmd.Output()
	if e != nil {
		return ""
	}
	_, path, _ := strings.Cut(string(out), pathMarker)
	path, _, found := strings.Cut(path, pathMarker)
	if !found {
		return ""
	}
	return path
}

func mergePath(first, rest string) string {
	seen := map[string]bool{}
	var dirs []string
	for _, dir := range append(filepath.SplitList(first), filepath.SplitList(rest)...) {
		if dir != "" && !seen[dir] {
			seen[dir] = true
			dirs = append(dirs, dir)
		}
	}
	return strings.Join(dirs, string(os.PathListSeparator))
}
