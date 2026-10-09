package ui

import (
	"image/color"
	"slices"
	"sort"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func agentStatusLabel(status string) string {
	switch status {
	case "starting", "pendingInit":
		return "Starting"
	case "running", "inProgress", "active":
		return "Running"
	case "idle", "waiting":
		return "Idle"
	case "completed", "complete":
		return "Completed"
	case "error", "errored", "failed", "notFound":
		return "Failed"
	case "interrupted", "shutdown", "stopped":
		return "Stopped"
	default:
		return "Unknown"
	}
}
func agentStatusColor(status string, p palette) color.RGBA {
	switch agentStatusLabel(status) {
	case "Running":
		return p.Accent
	case "Completed":
		return p.Success
	case "Failed":
		return p.Danger
	case "Starting":
		return p.Warning
	default:
		return p.Muted
	}
}
func agentDetail(agent workspace.Agent) string {
	var parts []string
	for _, value := range []string{agent.Role, agent.Path, strings.TrimSpace(strings.SplitN(agent.Message, "\n", 2)[0])} {
		if value != "" {
			parts = append(parts, value)
		}
	}
	if len(parts) == 0 {
		return cut(agent.ID, 40)
	}
	return cut(strings.Join(parts, " · "), 100)
}
func upsertAgent(c *workspace.Conversation, update workspace.Agent) {
	if update.ID == "" || update.ID == c.ID {
		return
	}
	// History preparation can share old session slices with the live view.
	c.Agents = slices.Clone(c.Agents)
	for i := range c.Agents {
		old := &c.Agents[i]
		if old.ID != update.ID {
			continue
		}
		if update.Name != "" {
			old.Name = update.Name
		}
		if update.Status != "" {
			old.Status = update.Status
		}
		if update.Role != "" {
			old.Role = update.Role
		}
		if update.Path != "" {
			old.Path = update.Path
		}
		if update.Message != "" {
			old.Message = update.Message
		}
		return
	}
	if len(c.Agents) < 64 {
		c.Agents = append(c.Agents, update)
	}
}
func updateAgents(c *workspace.Conversation, it map[string]any) {
	switch str(it, "type") {
	case "collabAgentToolCall":
		states, _ := it["agentsStates"].(map[string]any)
		ids := map[string]bool{}
		for id := range states {
			ids[id] = true
		}
		if receivers, ok := it["receiverThreadIds"].([]any); ok {
			for _, v := range receivers {
				if id, ok := v.(string); ok {
					ids[id] = true
				}
			}
		}
		ordered := make([]string, 0, len(ids))
		for id := range ids {
			ordered = append(ordered, id)
		}
		sort.Strings(ordered)
		for _, id := range ordered {
			state, _ := states[id].(map[string]any)
			status := str(state, "status")
			if status == "" && str(it, "tool") == "spawnAgent" {
				status = "running"
			}
			upsertAgent(c, workspace.Agent{ID: id, Name: str(state, "nickname"), Role: str(state, "role"), Status: status, Message: str(state, "message")})
		}
	case "subAgentActivity":
		status := map[string]string{"started": "running", "interacted": "running", "interrupted": "stopped", "completed": "completed"}[str(it, "kind")]
		upsertAgent(c, workspace.Agent{ID: str(it, "agentThreadId"), Path: str(it, "agentPath"), Status: status})
	}
}
func (a *App) drawAgents(w *desktop.Window, c *workspace.Conversation, depth int, seen map[string]bool) {
	if depth >= 4 || seen[c.ID] {
		return
	}
	seen[c.ID] = true
	for _, agent := range c.Agents {
		name := fallback(agent.Name, cut(agent.ID, 20))
		w.Row(28).Ratio(.08, .62, .30)
		w.LabelColored("●", "LC", agentStatusColor(agent.Status, a.p))
		if w.ButtonText(strings.Repeat("  ", depth) + name) {
			a.openInfoPeer(agent.ID, name, c.Cwd)
		}
		w.LabelColored(agentStatusLabel(agent.Status), "LC", a.p.Muted)
		muted(w, strings.Repeat("  ", depth)+agentDetail(agent), a.p)
		if child := a.state.Chats[agent.ID]; child != nil {
			a.drawAgents(w, child, depth+1, seen)
		}
	}
}
func (a *App) openInfoPeer(id, name, cwd string) {
	if id == "" {
		return
	}
	if a.state.Chats[id] == nil {
		a.state.Chats[id] = &workspace.Conversation{ID: id, Title: name, Cwd: cwd}
	}
	a.resumeThread(id)
}
