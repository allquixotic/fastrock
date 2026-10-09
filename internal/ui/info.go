package ui

import (
	"fmt"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type conversationInfo struct {
	Goal                                                              map[string]any
	GoalLoading, GoalBusy, GoalAvailable                              bool
	GoalUnsupported                                                   bool
	GoalGeneration                                                    uint64
	GoalNote                                                          string
	Terminals                                                         []backgroundTerminal
	Loaded                                                            bool
	CrossTab                                                          bool
	TerminalsLoading, TerminalsDirty, TerminalsAvailable, StoppingAll bool
	TerminalGeneration                                                uint64
	TerminalEpoch                                                     uint64
	TerminalNote                                                      string
	TerminalUpdated                                                   time.Time
	Stopping                                                          map[string]bool
}

func (a *App) extraInfo(w *desktop.Window, c *workspace.Conversation) {
	if a.infoViews == nil {
		a.infoViews = map[string]*conversationInfo{}
	}
	v := a.infoViews[c.ID]
	if v == nil {
		v = &conversationInfo{}
		a.infoViews[c.ID] = v
	}
	if !v.Loaded {
		v.Loaded = true
		a.loadInfo(c, v)
	}
	if c.ContextWindow > 0 || c.Tokens > 0 {
		if a.infoSection(w, "context", "Context", contextLabel(c)) {
			if c.ContextWindow > 0 {
				muted(w, fmt.Sprintf("%d of %d tokens in context", c.ContextTokens, c.ContextWindow), a.p)
			}
			muted(w, fmt.Sprintf("Thread total %d · in %d · cached %d · out %d", c.Tokens, c.InputTokens, c.CachedTokens, c.OutputTokens), a.p)
		}
	}
	a.drawGoal(w, c, v)
	a.drawTerminals(w, c, v)
	if len(c.Agents) > 0 && a.infoSection(w, "agents", "Sub-agents", fmt.Sprintf("%d", len(c.Agents))) {
		a.drawAgents(w, c, 0, map[string]bool{})
	}
	status := "Messages allowed"
	if c.NoMessages {
		status = "Messages off"
	}
	if a.infoSection(w, "messages", "Other conversations", status) {
		w.Row(28).Dynamic(1)
		allowMessages := !c.NoMessages
		if w.CheckboxText("Let agents in other tabs message this thread", &allowMessages) {
			a.setMessagesAllowed(c, allowMessages)
		}
		a.drawMailbox(w, c)
		w.Row(28).Dynamic(1)
		if w.ButtonText("Choose conversation…") {
			a.chooseConversation(func(peer *workspace.Conversation) { a.resumeThread(peer.ID) })
		}
	}
}
func (a *App) loadInfo(c *workspace.Conversation, v *conversationInfo) {
	a.loadGoal(c, v)
	a.loadTerminals(c, v)
}
func (a *App) goalStatus(c *workspace.Conversation, v *conversationInfo, status string) {
	a.mutateGoal(c, v, "thread/goal/set", map[string]any{"threadId": c.ID, "origin": "user", "status": status})
}
