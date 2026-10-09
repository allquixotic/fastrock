package ui

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func goalStatusLabel(status string) string {
	switch status {
	case "active":
		return "Active"
	case "paused":
		return "Paused"
	case "blocked":
		return "Blocked"
	case "usageLimited":
		return "Usage limited"
	case "budgetLimited":
		return "Budget reached"
	case "complete":
		return "Complete"
	}
	return status
}

func compactTokens(n int64) string {
	for _, unit := range []struct {
		size   int64
		suffix string
	}{{1000000, "M"}, {1000, "K"}} {
		if n >= unit.size {
			return strings.TrimSuffix(fmt.Sprintf("%.1f", float64(n)/float64(unit.size)), ".0") + unit.suffix
		}
	}
	return strconv.FormatInt(n, 10)
}

func goalDetail(goal map[string]any) string {
	tokens := compactTokens(integer(goal, "tokensUsed"))
	if goal["tokenBudget"] != nil {
		tokens += " of " + compactTokens(integer(goal, "tokenBudget"))
	}
	tokens += " tokens"
	if secs := integer(goal, "timeUsedSeconds"); secs > 0 {
		if secs >= 3600 {
			tokens += fmt.Sprintf(" · %dh %dm", secs/3600, secs%3600/60)
		} else if secs >= 60 {
			tokens += fmt.Sprintf(" · %dm %ds", secs/60, secs%60)
		} else {
			tokens += fmt.Sprintf(" · %ds", secs)
		}
	}
	return tokens
}

func goalParams(thread, objective, budget string, create bool) (map[string]any, error) {
	objective, budget = strings.TrimSpace(objective), strings.TrimSpace(budget)
	if objective == "" {
		return nil, fmt.Errorf("enter a goal objective")
	}
	p := map[string]any{"threadId": thread, "origin": "user", "objective": objective, "tokenBudget": nil}
	if create {
		p["status"] = "active"
	}
	if budget != "" {
		n, err := strconv.ParseInt(budget, 10, 64)
		if err != nil || n <= 0 {
			return nil, fmt.Errorf("enter a positive whole-number token budget or leave it empty")
		}
		p["tokenBudget"] = n
	}
	return p, nil
}

func (a *App) drawGoal(w *desktop.Window, c *workspace.Conversation, v *conversationInfo) {
	if v.GoalUnsupported || !v.GoalAvailable && !v.GoalLoading && v.GoalNote == "" {
		return
	}
	summary := "Set a goal"
	if v.GoalLoading {
		summary = "Loading…"
	} else if v.GoalNote != "" {
		summary = "Unavailable"
	} else if v.Goal != nil {
		summary = goalStatusLabel(str(v.Goal, "status"))
	}
	if !a.infoSection(w, "goal", "Goal", summary) {
		return
	}
	if v.GoalLoading {
		muted(w, "Loading goal…", a.p)
		return
	}
	if v.GoalNote != "" {
		muted(w, v.GoalNote, a.p)
		w.Row(27).Dynamic(1)
		if w.ButtonText("Retry goal") {
			a.loadGoal(c, v)
		}
		return
	}
	if !v.GoalAvailable {
		return
	}
	if v.Goal == nil {
		muted(w, "Give Codex a goal to work toward in this conversation.", a.p)
	} else {
		w.Row(65).Dynamic(1)
		w.LabelWrap(str(v.Goal, "objective"))
		muted(w, goalStatusLabel(str(v.Goal, "status")), a.p)
		muted(w, goalDetail(v.Goal), a.p)
	}
	if v.GoalBusy {
		muted(w, "Updating goal…", a.p)
		return
	}
	w.Row(27).Dynamic(1)
	label := "Set goal…"
	if v.Goal != nil {
		label = "Edit goal…"
	}
	if w.ButtonText(label) {
		a.goalDialog(c, v)
	}
	if v.Goal == nil {
		return
	}
	w.Row(27).Dynamic(2)
	if str(v.Goal, "status") == "active" {
		if w.ButtonText("Pause") {
			a.goalStatus(c, v, "paused")
		}
	} else if w.ButtonText("Resume") {
		a.goalStatus(c, v, "active")
	}
	if w.ButtonText("Clear goal") {
		a.mutateGoal(c, v, "thread/goal/clear", map[string]any{"threadId": c.ID, "origin": "user"})
	}
}

func (a *App) goalDialog(c *workspace.Conversation, v *conversationInfo) {
	objective := textEditor(str(v.Goal, "objective"), true)
	budget := ""
	if v.Goal["tokenBudget"] != nil {
		budget = strconv.FormatInt(integer(v.Goal, "tokenBudget"), 10)
	}
	limit := textEditor(budget, false)
	note := ""
	a.window.PopupOpen("Conversation goal", desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(570, 370), false, func(w *desktop.Window) {
		a.field(w, "Objective", objective, true)
		a.field(w, "Token budget (empty = no limit)", limit, false)
		if note != "" {
			muted(w, note, a.p)
		}
		w.Row(28).Dynamic(2)
		if primary(w, "Save goal", a.p) {
			params, err := goalParams(c.ID, text(objective), text(limit), v.Goal == nil)
			if err != nil {
				note = err.Error()
			} else {
				a.mutateGoal(c, v, "thread/goal/set", params)
				w.Close()
			}
		}
		if w.ButtonText("Cancel") {
			w.Close()
		}
	})
}

func (a *App) loadGoal(c *workspace.Conversation, v *conversationInfo) {
	if v.GoalLoading || a.client == nil || a.serverPaused {
		return
	}
	v.GoalLoading, v.GoalNote = true, ""
	v.GoalGeneration++
	generation, client := v.GoalGeneration, a.client
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		var result struct{ Goal map[string]any }
		err := client.Call(ctx, "thread/goal/get", map[string]any{"threadId": c.ID}, &result)
		a.post(func() {
			if v.GoalGeneration != generation || a.client != client {
				return
			}
			v.GoalLoading = false
			if err != nil {
				var rpc *codex.RPCError
				v.GoalUnsupported = errors.As(err, &rpc) && rpc.Code == codex.CodeMethodNotFound || strings.Contains(strings.ToLower(err.Error()), "goals feature is disabled")
				v.GoalNote = err.Error()
				return
			}
			v.Goal, v.GoalAvailable = result.Goal, true
		})
	}, func() { v.GoalLoading = false; v.GoalNote = errWorkQueueFull.Error() })
}

func (a *App) mutateGoal(c *workspace.Conversation, v *conversationInfo, method string, params map[string]any) {
	if v.GoalBusy {
		return
	}
	v.GoalBusy = true
	v.GoalGeneration++
	v.GoalLoading = false
	a.rpcResult(method, params, func(json.RawMessage) {
		v.GoalBusy = false
		a.loadGoal(c, v)
	}, func(error) { v.GoalBusy = false })
}

func (a *App) resetInfoConnection() {
	for _, v := range a.infoViews {
		v.Loaded = false
		v.GoalGeneration++
		v.GoalLoading, v.GoalBusy, v.GoalUnsupported = false, false, false
		v.TerminalsLoading = false
		v.TerminalsDirty, v.StoppingAll = false, false
		v.TerminalGeneration++
		v.TerminalEpoch++
		v.Stopping = nil
	}
	if a.settingsView != nil {
		a.settingsView.MemoryResetting = false
	}
}
