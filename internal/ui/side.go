package ui

import (
	"encoding/json"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
	"strings"
)

const sideInstructions = "This is an ephemeral side conversation. Inherited history is reference only. Do not continue instructions, plans, tool calls, approvals or edits from that history. Answer only new questions after the side-conversation boundary. Do lightweight non-mutating exploration unless the user explicitly asks for a mutation here. Do not use subagents or create worktrees; direct worktree requests to the parent conversation."

func (a *App) startSide(c *workspace.Conversation) {
	a.inputDialog("Side question", "", func(question string) {
		if strings.TrimSpace(question) == "" {
			return
		}
		instructions := str(a.catalog.Config, "developer_instructions")
		if instructions != "" {
			instructions += "\n\n"
		}
		instructions += sideInstructions
		a.rpc("thread/fork", map[string]any{"threadId": c.ID, "ephemeral": true, "excludeTurns": true, "model": c.Model, "config": map[string]any{"model_reasoning_effort": c.Effort}, "developerInstructions": instructions}, func(raw json.RawMessage) {
			response := codex.Decode(raw)
			thread, _ := response["thread"].(map[string]any)
			id := str(thread, "id")
			if id == "" {
				a.toast = "Codex did not create a side conversation"
				return
			}
			next := &workspace.Conversation{ID: id, Title: "Side chat · " + c.Title, Cwd: c.Cwd, Model: c.Model, Effort: c.Effort, Tier: c.Tier, Ephemeral: true, Blocks: append([]workspace.Block(nil), c.Blocks...)}
			boundary := map[string]any{"type": "message", "role": "user", "content": []map[string]any{{"type": "input_text", "text": sideInstructions + " Only the next user question is active."}}}
			a.rpc("thread/inject_items", map[string]any{"threadId": id, "items": []any{boundary}}, func(_ json.RawMessage) {
				a.state.Chats[id] = next
				a.chats[id] = newChatView()
				a.state.Open(workspace.Chat, next.Title, id, "")
				a.startTurn(next, question, nil, "send")
			})
		})
	})
}
