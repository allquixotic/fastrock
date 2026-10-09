package ui

import (
	"strings"

	"github.com/allquixotic/fastrock/internal/workspace"
)

var slashCommands = []string{"model", "permissions", "approvals", "approve", "plan", "review", "new", "clear", "resume", "fork", "rename", "compact", "recap", "side", "btw", "init", "diff", "mention", "copy", "export", "status", "usage", "settings", "config", "mcp", "skills", "plugins", "apps", "hooks", "experimental", "features", "memories", "theme", "debug-config", "archive", "ps", "stop", "worktree", "login", "logout", "quit", "exit"}

func (a *App) slash(c *workspace.Conversation, input string) bool {
	name, args, _ := strings.Cut(strings.TrimPrefix(input, "/"), " ")
	name = strings.ToLower(name)
	switch name {
	case "approvals":
		name = "permissions"
	case "btw":
		name = "side"
	case "exit":
		name = "quit"
	case "apps":
		name = "plugins"
	}
	args = strings.TrimSpace(args)
	if args != "" && !contains([]string{"plan", "rename", "review", "side", "new", "fork", "resume"}, name) {
		return false
	}
	switch name {
	case "plan":
		c.Plan = true
		if args != "" {
			a.startTurn(c, args, nil, "send")
		}
	case "new", "clear":
		a.newThread(c.Cwd)
	case "resume":
		a.chooseConversation(func(next *workspace.Conversation) { a.resumeThread(next.ID) })
	case "rename":
		if args != "" {
			name := args
			a.rpc("thread/name/set", map[string]any{"threadId": c.ID, "name": name}, nil)
		} else {
			a.chatAction(c.ID, name)
		}
	case "fork", "compact", "recap", "side", "init", "review", "export", "archive", "worktree":
		if args != "" {
			return false
		}
		a.chatAction(c.ID, name)
	case "diff":
		a.showDiff(c)
	case "mention":
		a.choosePath(false, false, func(path string) {
			if v := a.chats[c.ID]; v != nil {
				v.Editor.Paste("@\"" + strings.ReplaceAll(path, "\"", "\\\"") + "\" ")
			}
		})
	case "copy":
		found := false
		for i := len(c.Blocks) - 1; i >= 0; i-- {
			if c.Blocks[i].Role == "assistant" {
				a.copyText(c.Blocks[i].Text)
				found = true
				break
			}
		}
		if !found {
			a.toast = "No assistant reply to copy"
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
		a.settingsPage("Appearance")
	case "settings", "config":
		a.openSettings()
	case "status", "usage":
		a.prefs.Info = true
	case "ps":
		a.prefs.Info = true
		if a.infoViews == nil {
			a.infoViews = map[string]*conversationInfo{}
		}
		if a.infoViews[c.ID] == nil {
			a.infoViews[c.ID] = &conversationInfo{}
		}
		if a.infoCollapsed == nil {
			a.infoCollapsed = map[string]bool{}
		}
		a.infoCollapsed["terminals"] = false
		a.loadTerminals(c, a.infoViews[c.ID])
	case "stop":
		a.runAction("escape")
	case "approve":
		a.toast = "No review denial is selected. Inspect the denial before approving it."
	case "logout", "login":
		a.settingsPage("Account")
	case "quit":
		a.requestQuit()
	default:
		return false
	}
	return true
}
func (a *App) settingsPage(page string) {
	a.openSettings()
	a.leaveRawConfig(func() {
		s := a.settingsView
		s.LoadGeneration++
		s.Busy = false
		s.Page, s.Items = page, nil
		setText(s.Output, "")
		a.loadSettingsPage(page)
	})
}
