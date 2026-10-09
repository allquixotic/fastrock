package ui

import (
	"strings"

	"github.com/allquixotic/fastrock/internal/workspace"
)

var slashCommands = []string{"model", "permissions", "approve", "plan", "review", "new", "resume", "fork", "rename", "compact", "recap", "side", "init", "diff", "mention", "copy", "export", "status", "settings", "mcp", "skills", "plugins", "hooks", "experimental", "memories", "theme", "debug-config", "archive", "ps", "stop", "worktree", "logout", "quit"}

func (a *App) slash(c *workspace.Conversation, input string) bool {
	name, args, _ := strings.Cut(strings.TrimPrefix(input, "/"), " ")
	switch name {
	case "plan":
		c.Plan = !c.Plan
		if args != "" {
			a.startTurn(c, args, nil, "send")
		}
	case "new", "clear":
		a.newThread(c.Cwd)
	case "resume":
		a.chooseConversation(func(next *workspace.Conversation) { a.resumeThread(next.ID) })
	case "fork", "rename", "compact", "recap", "side", "init", "review", "export", "archive", "worktree":
		a.chatAction(c.ID, name)
	case "diff":
		a.showDiff(c)
	case "mention":
		a.choosePath(false, false, func(path string) { setText(a.chats[c.ID].Editor, "@"+path+" ") })
	case "copy":
		for i := len(c.Blocks) - 1; i >= 0; i-- {
			if c.Blocks[i].Role == "assistant" {
				a.copyText(c.Blocks[i].Text)
				break
			}
		}
	case "model":
		a.settingsPage("Models")
	case "permissions", "debug-config":
		a.settingsPage("Codex configuration")
	case "mcp":
		a.settingsPage("MCP servers")
	case "skills":
		a.settingsPage("Skills")
	case "plugins":
		a.settingsPage("Plugins")
	case "hooks":
		a.settingsPage("Hooks")
	case "experimental", "features":
		a.settingsPage("Features")
	case "memories":
		a.settingsPage("Memories")
	case "theme":
		a.runAction("theme")
	case "settings", "config":
		a.openSettings()
	case "status", "usage":
		a.prefs.Info = true
	case "ps":
		a.inspectRPC("Background terminals", "thread/backgroundTerminals/list", map[string]any{"threadId": c.ID})
	case "stop":
		a.runAction("escape")
	case "approve":
		a.rpc("thread/approveGuardianDeniedAction", map[string]any{"threadId": c.ID}, nil)
	case "logout":
		a.rpc("account/logout", map[string]any{}, nil)
	case "quit":
		a.window.Close()
	default:
		return false
	}
	return true
}
func (a *App) settingsPage(page string) {
	a.openSettings()
	a.settingsView.Page = page
	a.loadSettingsPage(page)
}
