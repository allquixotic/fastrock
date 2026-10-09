package codex

import "os/exec"

// lookPath relies on the PATH Windows assigns to the desktop login.
func lookPath() (string, error) { return exec.LookPath("codex") }
