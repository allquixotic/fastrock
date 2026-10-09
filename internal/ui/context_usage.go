package ui

import (
	"fmt"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func contextLabel(c *workspace.Conversation) string {
	if c.ContextWindow > 0 {
		left := max(0, min(100, 100-c.ContextTokens*100/c.ContextWindow))
		return fmt.Sprintf("%s · %d%% context left", c.Model, left)
	}
	return fmt.Sprintf("%s · %d total tokens", c.Model, c.Tokens)
}
