package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/assistant"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

type assistantView struct {
	ThreadID, TurnID, Model, Effort, Tier, Status string
	History                                       workspace.Conversation
	Layout                                        *chatView
	Preview                                       []string
	Selected                                      []bool
	Outcomes                                      []string
	Editor                                        *desktop.TextEditor
	Busy, Visible                                 bool
	PlanApplied                                   bool
	PlanScope                                     rally.Query
	View                                          *rallyView
	Scope                                         rally.Query
	Plan                                          *assistant.Plan
}

func (a *App) openAssistant(v *rallyView) {
	if v != nil && v.Spec.ID == "customviews" {
		a.toast = "Open a Rally work-item page to use the assistant"
		return
	}
	if v == nil || v.Closed || a.rallyClient == nil || a.client == nil {
		a.toast = "Connect Rally and Codex before opening the Rally assistant"
		return
	}
	q := a.rallyQuery(v)
	if a.assistant != nil && (a.assistant.Scope.Workspace != q.Workspace || a.assistant.Scope.Project != q.Project || a.assistant.Scope.Children != q.Children || a.assistant.Scope.Parents != q.Parents) {
		if a.assistant.Busy {
			a.toast = "Stop the current Rally assistant before changing its scope"
			return
		}
		a.confirm("Start a new Rally conversation?", "The selected scope has changed. Start a new assistant conversation?", func() { a.assistant = nil; a.openAssistant(v) })
		return
	}
	if a.assistant == nil {
		a.assistant = &assistantView{Editor: textEditor("", true), Scope: a.rallyQuery(v)}
	}
	s := a.assistant
	s.View = v
	s.Visible = true
	if a.window != nil {
		a.window.Changed()
	}
}
func (a *App) drawAssistant(w *desktop.Window) {
	s := a.assistant
	if s == nil {
		return
	}
	w.Row(28).Ratio(.82, .18)
	w.Label("Rally assistant", "LC")
	if tooltipButton(w, "×", "Hide Rally assistant") {
		s.Visible = false
		return
	}
	if s.Plan != nil && len(s.Selected) != len(s.Plan.Changes) {
		s.Selected = make([]bool, len(s.Plan.Changes))
		s.Outcomes = make([]string, len(s.Plan.Changes))
		for i := range s.Selected {
			s.Selected[i] = true
			s.Outcomes[i] = "Pending"
		}
	}
	muted(w, "Uses your installed Codex and its configured model provider.", a.p)
	w.Row(24).Dynamic(3)
	w.Label("Model", "LC")
	w.Label("Effort", "LC")
	w.Label("Speed", "LC")
	w.Row(28).Dynamic(3)
	a.modelPickers(w, &s.Model, &s.Effort, &s.Tier, s.Busy)
	if a.activeApprovalFor(s.ThreadID) {
		a.drawApprovalFor(w, s.ThreadID)
	}
	scale := w.Master().Style().Scaling
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	reserved := int(120*scale) + 4*spacing
	if s.Plan != nil {
		reserved += int(float64(titleHeight(w)+168)*scale) + 3*spacing
	}
	h := max(int(60*scale), w.LayoutAvailableHeight()-reserved)
	w.RowScaled(h).Dynamic(1)
	if body := w.GroupBegin("rally-assistant-transcript", desktop.WindowNoHScrollbar); body != nil {
		if len(s.History.Blocks) == 0 {
			muted(body, "Try: Show blocked stories, group by owner, and explain the risks.", a.p)
		} else {
			a.drawAssistantHistory(body, s)
		}
		body.GroupEnd()
	}
	if s.Plan != nil {
		title(w, s.Plan.Summary, a.p)
		w.Row(140).Dynamic(1)
		if preview := w.GroupBegin("change-preview", desktop.WindowNoHScrollbar); preview != nil {
			for i, line := range s.Preview {
				preview.Row(26).Dynamic(1)
				if i < len(s.Selected) && !s.Busy && !s.PlanApplied {
					preview.CheckboxText(fmt.Sprintf("Change %d", i+1), &s.Selected[i])
				} else {
					preview.Label(fmt.Sprintf("Change %d · %s", i+1, s.Outcomes[i]), "LC")
				}
				lines := desktop.WrapText(preview.Master().Style().Font, line, max(100, preview.LayoutAvailableWidth()-16))
				preview.Row(min(240, max(44, len(lines)*(a.prefs.FontSize+7)))).Dynamic(1)
				preview.LabelWrap(line)
				preview.Row(26).Static(150)
				if preview.ButtonText("View full change") {
					a.openText(fmt.Sprintf("Proposed change %d", i+1), line)
				}
			}
			preview.GroupEnd()
		}
		w.Row(28).Dynamic(2)
		count := 0
		for _, on := range s.Selected {
			if on {
				count++
			}
		}
		if primary(w, fmt.Sprintf("Apply %d selected", count), a.p) && count > 0 && !s.Busy && !s.PlanApplied {
			if a.rallyClient == nil {
				s.Status = "Connect to Rally before applying changes"
				return
			}
			if a.rallyQuery(s.View) != s.PlanScope {
				s.Status = "Scope changed; request a new proposal before applying"
				return
			}
			p := *s.Plan
			p.Changes = nil
			var selected []int
			for i, change := range s.Plan.Changes {
				if s.Selected[i] {
					p.Changes = append(p.Changes, change)
					selected = append(selected, i)
				} else {
					s.Outcomes[i] = "Not selected"
				}
			}
			s.Busy = true
			c := a.rallyClient
			view := s.View
			s.PlanApplied = true
			a.writeWork(func() {
				ctx, cancel := context.WithTimeout(a.ctx, 2*time.Minute)
				defer cancel()
				n, e := assistant.Apply(ctx, c, p)
				a.post(func() {
					s.Busy = false
					s.Status = fmt.Sprintf("Applied %d of %d changes", n, len(p.Changes))
					if e != nil {
						s.Status += ". " + e.Error() + ". Refresh and request a new plan before retrying."
					}
					for i, index := range selected {
						s.Outcomes[index] = "Not attempted"
						if i < n {
							s.Outcomes[index] = "Applied"
						} else if i == n && e != nil {
							s.Outcomes[index] = "Failed: " + e.Error()
						}
					}
					s.History.Append(workspace.NewID("notice"), "notice", "notice", s.Status)
					a.refreshRallyItems(view)
				})
			}, func() { s.Busy = false; s.PlanApplied = false; s.Status = errWorkQueueFull.Error() })
		}
		if w.ButtonText("Discard") {
			s.Plan = nil
		}
	}
	w.Row(62).Dynamic(1)
	s.Editor.Edit(w)
	if s.Editor.Active && !s.Busy {
		for e := range w.Input().Keyboard.Events() {
			if e.HandleKeyModmask(key.CodeReturnEnter, key.ModControl|key.ModMeta) {
				a.sendAssistant()
			}
		}
	}
	w.Row(28).Dynamic(1)
	if !s.Busy && w.ButtonText("New conversation") {
		a.confirm("New Rally conversation?", "Start a new conversation in this scope? The current transcript will close.", func() { a.assistant = nil; a.openAssistant(s.View) })
	}
	w.Row(30).Ratio(.75, .25)
	w.LabelColored(s.Status, "LC", a.p.Muted)
	if s.Busy {
		if w.ButtonText("Stop") && s.TurnID != "" {
			a.rpc("turn/interrupt", map[string]any{"threadId": s.ThreadID, "turnId": s.TurnID}, nil)
		}
	} else if enabledButton(w, "Ask AI", a.rallyClient != nil && a.client != nil, true, a.p) {
		a.sendAssistant()
	}
}
func (a *App) sendAssistant() {
	s := a.assistant
	if s == nil || s.Busy {
		return
	}
	prompt := strings.TrimSpace(text(s.Editor))
	if prompt == "" {
		return
	}
	if a.rallyClient == nil {
		a.toast = "Connect Rally first"
		return
	}
	if a.client == nil {
		a.toast = "Codex is disconnected"
		return
	}
	if warning := a.catalog.SpeedWarning(fallback(s.Model, a.catalog.DefaultModel()), fallback(s.Tier, "default")); warning != "" {
		s.Status = warning
		return
	}
	s.Scope = a.rallyQuery(s.View)
	s.Busy = true
	s.History.Append(workspace.NewID("user"), "userMessage", "you", prompt)
	setText(s.Editor, "")
	s.Status = "Thinking…"
	startTurn := func() {
		params := map[string]any{"threadId": s.ThreadID, "input": codex.TextInput(prompt)}
		setModelParams(params, s.Model, s.Effort, s.Tier)
		a.rpcResult("turn/start", params, func(raw json.RawMessage) {
			r := codex.Decode(raw)
			t, _ := r["turn"].(map[string]any)
			if s.Busy {
				s.TurnID = str(t, "id")
			}
		}, func(err error) { s.Busy = false; s.Status = err.Error(); setText(s.Editor, prompt) })
	}
	if s.ThreadID != "" {
		startTurn()
		return
	}
	s.Scope = a.rallyQuery(s.View)

	instructions := assistant.Instructions + "\nSelected workspace: " + s.Scope.Workspace + "\nSelected project: " + s.Scope.Project
	params := map[string]any{"ephemeral": true, "cwd": a.prefs.WorkingDirectory, "developerInstructions": instructions, "dynamicTools": assistant.Specs(), "sandbox": "read-only"}
	setModelParams(params, s.Model, "", s.Tier)
	a.rpcResult("thread/start", params, func(raw json.RawMessage) {
		r := codex.Decode(raw)
		t, _ := r["thread"].(map[string]any)
		s.ThreadID = str(t, "id")
		if s.ThreadID == "" {
			s.Busy = false
			s.Status = "Codex did not create the assistant thread"
			return
		}
		startTurn()
	}, func(err error) { s.Busy = false; s.Status = err.Error(); setText(s.Editor, prompt) })
}
func (a *App) assistantEvent(m codex.Message, p map[string]any) {
	s := a.assistant
	switch m.Method {
	case "item/agentMessage/delta":
		s.History.Append(str(p, "itemId"), "agentMessage", "assistant", str(p, "delta"))
	case "item/completed":
		if item, ok := p["item"].(map[string]any); ok && str(item, "type") == "agentMessage" {
			a.upsertItem(&s.History, item)
		}
	case "turn/started":
		t, _ := p["turn"].(map[string]any)
		s.TurnID = str(t, "id")
	case "turn/completed":
		for _, block := range s.History.Blocks {
			s.History.FinishBlock(block.ID)
		}
		s.Busy = false
		s.TurnID = ""
		s.Status = "Ready"
		t, _ := p["turn"].(map[string]any)
		if err, ok := t["error"].(map[string]any); ok {
			s.Status = str(err, "message")
		}
	case "error":
		if retry, _ := p["willRetry"].(bool); retry {
			return
		}
		s.Busy = false
		err, _ := p["error"].(map[string]any)
		s.Status = str(err, "message")
	}
}
func (a *App) runDynamicTool(client *codex.Client, m codex.Message) {
	a.post(func() {
		if a.client != client {
			return
		}
		m.Origin = client
		p := codex.Decode(m.Params)
		if a.crossTabTool(m, p) {
			return
		}
		if a.assistant == nil || str(p, "threadId") != a.assistant.ThreadID {
			a.work(func() { _ = client.Reject(m.ID, "Rally tools are available only in the Rally assistant") })
			return
		}
		s := a.assistant
		name := str(p, "tool")
		args, _ := json.Marshal(p["arguments"])
		if raw, ok := p["arguments"].(string); ok {
			args = []byte(raw)
		}
		c := a.rallyClient
		tools := assistant.Tools{Client: c, Scope: s.Scope,
			Show: func(view assistant.View) error {
				applied := make(chan error, 1)
				a.post(func() {
					a.openRally(view.Page)
					tab := a.state.Current()
					v := a.rallyViews[tab.ID]
					if v.Detail != nil && v.Detail.dirty() {
						a.toast = "An unsaved item is open; save it before applying the AI view"
						applied <- fmt.Errorf("view was not changed: an unsaved work item is open")
						return
					}
					v.Detail = nil
					setText(v.Query, view.Query)
					v.QueryApplied = view.Query
					if view.Mode != "" {
						v.Mode = view.Mode
					}
					if view.Group != "" {
						v.Group = view.Group
					}
					v.AIView = true
					a.refreshRally(v)
					s.View = v
					applied <- nil
				})
				select {
				case err := <-applied:
					return err
				case <-a.ctx.Done():
					return a.ctx.Err()
				case <-time.After(30 * time.Second):
					return fmt.Errorf("view update was not acknowledged")
				}
			},
			Propose: func(plan assistant.Plan) error {
				preview := proposalPreview(plan)
				a.post(func() {
					s.Plan = &plan
					s.Preview = preview
					s.PlanApplied = false
					s.PlanScope = s.Scope
					s.Selected = make([]bool, len(plan.Changes))
					s.Outcomes = make([]string, len(plan.Changes))
					for i := range s.Selected {
						s.Selected[i] = true
						s.Outcomes[i] = "Pending"
					}
				})
				return nil
			}}
		a.work(func() {
			ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
			defer cancel()
			result, e := tools.Execute(ctx, name, args)
			success := e == nil
			if e != nil {
				result = map[string]any{"error": e.Error()}
			}
			b, _ := json.Marshal(result)
			_ = client.Respond(m.ID, map[string]any{"success": success, "contentItems": []map[string]any{{"type": "inputText", "text": string(b)}}})
		})
	})
}
