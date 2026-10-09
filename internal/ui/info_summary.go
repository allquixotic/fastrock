package ui

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func threadSettings(response map[string]any) workspace.ThreadSettings {
	p := response["approvalPolicy"]
	approval, _ := p.(string)
	if granular, ok := p.(map[string]any); ok && granular["granular"] != nil {
		approval = "granular"
	}
	sandbox, _ := response["sandbox"].(map[string]any)
	if sandbox == nil {
		sandbox, _ = response["sandboxPolicy"].(map[string]any)
	}
	network, _ := sandbox["network_access"].(bool)
	if value, ok := sandbox["networkAccess"].(bool); ok {
		network = value
	}
	if sandbox["network_access"] == "enabled" || sandbox["networkAccess"] == "enabled" {
		network = true
	}
	roots := sandbox["writable_roots"]
	if roots == nil {
		roots = sandbox["writableRoots"]
	}
	if roots == nil {
		roots = []string{}
	}
	encoded, _ := json.Marshal(roots)
	return workspace.ThreadSettings{Current: approval != "" && str(sandbox, "type") != "", Provider: str(response, "modelProvider"), Approval: approval, Reviewer: str(response, "approvalsReviewer"), Sandbox: str(sandbox, "type"), Network: network, WritableRoots: string(encoded)}
}

func approvalSummary(s workspace.ThreadSettings) string {
	label := map[string]string{"untrusted": "Untrusted commands", "on-request": "On request", "on-failure": "On failure", "never": "Never ask", "granular": "Granular"}[s.Approval]
	if label == "" {
		return "—"
	}
	if s.Approval != "never" && (s.Reviewer == "auto_review" || s.Reviewer == "autoReview") {
		label += " · auto-review"
	}
	return label
}

func sandboxSummary(s workspace.ThreadSettings) string {
	label := map[string]string{"danger-full-access": "Full access", "read-only": "Read only", "external-sandbox": "External sandbox", "workspace-write": "Workspace write"}[s.Sandbox]
	if label == "" {
		return "—"
	}
	if s.Network && (s.Sandbox == "workspace-write" || s.Sandbox == "read-only") {
		label += " + network"
	}
	return label
}

func conversationSummary(c *workspace.Conversation) [][2]string {
	rows := [][2]string{{"Model", fallback(c.Model, "—")}, {"Effort", fallback(c.Effort, "Default")}, {"Provider", fallback(c.Settings.Provider, "—")}, {"Approval", approvalSummary(c.Settings)}, {"Sandbox", sandboxSummary(c.Settings)}}
	if c.Tier != "" {
		rows = append(rows, [2]string{"Speed", c.Tier})
	}
	if c.SideParentID != "" {
		rows = append([][2]string{{"Side chat", fmt.Sprintf("of “%s”, not saved", fallback(c.SideParentTitle, c.SideParentID))}}, rows...)
	}
	return rows
}

// Section state belongs to the panel, so changing conversations keeps the same
// disclosure choices. Callers omit genuinely empty optional sections.
func (a *App) infoSection(w *nucular.Window, key, name, summary string) bool {
	if a.infoCollapsed == nil {
		a.infoCollapsed = map[string]bool{}
	}
	if summary != "" {
		name += " · " + cut(strings.ReplaceAll(summary, "\n", " "), 48)
	}
	w.Row(28).Dynamic(1)
	if folderRow(w, name, !a.infoCollapsed[key], a.p) {
		a.infoCollapsed[key] = !a.infoCollapsed[key]
	}
	return !a.infoCollapsed[key]
}

func (a *App) drawInfoSummary(w *nucular.Window, c *workspace.Conversation) {
	if !a.infoSection(w, "session", "Session", agentStatusLabel(c.Status)) {
		return
	}
	for _, row := range conversationSummary(c) {
		w.Row(28).Ratio(.27, .73)
		w.LabelColored(row[0], "LC", a.p.Muted)
		w.LabelWrap(row[1])
	}
}
