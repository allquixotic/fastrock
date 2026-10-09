package platform

import (
	"context"
	"os"
	"os/exec"
	"strings"
)

// ChildEnv preserves Codex/provider configuration while excluding GUI-only
// credentials and automation capabilities from tools, openers and Git hooks.
func ChildEnv(drop ...string) []string {
	denied := map[string]bool{"FASTROCK_RALLY_TOKEN": true, "FASTROCK_AUTOMATION": true, "FASTROCK_BROKER": true, "FASTROCK_BROKER_TOKEN": true}
	for _, key := range drop {
		denied[strings.ToUpper(key)] = true
	}
	out := []string{}
	for _, entry := range os.Environ() {
		key, _, _ := strings.Cut(entry, "=")
		if !denied[strings.ToUpper(key)] {
			out = append(out, entry)
		}
	}
	return out
}
func Command(name string, args ...string) *exec.Cmd {
	c := exec.Command(name, args...)
	c.Env = ChildEnv()
	return c
}
func CommandContext(ctx context.Context, name string, args ...string) *exec.Cmd {
	c := exec.CommandContext(ctx, name, args...)
	c.Env = ChildEnv()
	return c
}
