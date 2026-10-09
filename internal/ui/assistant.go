package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/rect"
	"github.com/allquixotic/fastrock/internal/assistant"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/rally"
)

type assistantView struct {
	ThreadID, TurnID, Model, Effort, Tier, Transcript, Status string
	Editor                                                    *nucular.TextEditor
	Busy, Visible                                             bool
	View                                                      *rallyView
	Scope                                                     rally.Query
	Plan                                                      *assistant.Plan
}

func (a *App) openAssistant(v *rallyView) {
	q := a.rallyQuery(v)
	if a.assistant != nil && (a.assistant.Scope.Workspace != q.Workspace || a.assistant.Scope.Project != q.Project || a.assistant.Scope.Children != q.Children || a.assistant.Scope.Parents != q.Parents) {
		if a.assistant.Busy {
			a.toast = "Stop the current Rally assistant before changing its scope"
			return
		}
		a.assistant = nil
	}
	if a.assistant == nil {
		model := a.catalog.DefaultModel()
		m, _ := a.catalog.Find(model)
		a.assistant = &assistantView{Model: model, Effort: m.DefaultEffort, Tier: "default", Editor: textEditor("", true), Scope: a.rallyQuery(v)}
	}
	s := a.assistant
	s.View = v
	s.Visible = true
	a.window.PopupOpen("Rally assistant", nucular.WindowTitle|nucular.WindowMovable|nucular.WindowScalable|nucular.WindowClosable|nucular.WindowNonmodal, rect.Rect{X: 350, Y: 100, W: 760, H: 690}, true, func(w *nucular.Window) { a.drawAssistant(w) })
}
func (a *App) drawAssistant(w *nucular.Window) {
	s := a.assistant
	if s == nil {
		return
	}
	muted(w, "Uses your installed Codex and its configured model provider.", a.p)
	w.Row(28).Dynamic(3)
	models := []string{}
	mi := 0
	for i, m := range a.catalog.Models {
		models = append(models, m.Name)
		if m.Model == s.Model {
			mi = i
		}
	}
	if len(models) > 0 {
		next := w.ComboSimple(models, mi, 28)
		if next != mi && !s.Busy {
			s.Model = a.catalog.Models[next].Model
			s.Effort = a.catalog.Models[next].DefaultEffort
			s.Tier = "default"
		}
	}
	m, _ := a.catalog.Find(s.Model)
	efforts := []string{}
	ei := 0
	for i, e := range m.Efforts {
		efforts = append(efforts, e.ID)
		if e.ID == s.Effort {
			ei = i
		}
	}
	if len(efforts) > 0 {
		s.Effort = efforts[w.ComboSimple(efforts, ei, 28)]
	} else {
		w.Label("Default effort", "LC")
	}
	tiers := a.catalog.Speeds(m)
	names := []string{}
	ti := 0
	for i, t := range tiers {
		names = append(names, t.Name)
		if t.ID == s.Tier {
			ti = i
		}
	}
	s.Tier = tiers[w.ComboSimple(names, ti, 28)].ID
	h := max(120, w.LayoutAvailableHeight()-145)
	if s.Plan != nil {
		h = max(110, h-140)
	}
	w.Row(h).Dynamic(1)
	if body := w.GroupBegin("rally-assistant-transcript", nucular.WindowNoHScrollbar); body != nil {
		if s.Transcript == "" {
			muted(body, "Try: Show blocked stories, group by owner, and explain the risks.", a.p)
		} else {
			a.markdown(body, s.Transcript)
		}
		body.GroupEnd()
	}
	if s.Plan != nil {
		title(w, s.Plan.Summary, a.p)
		w.Row(70).Dynamic(1)
		if preview := w.GroupBegin("change-preview", nucular.WindowNoHScrollbar); preview != nil {
			for _, ch := range s.Plan.Changes {
				b, _ := json.Marshal(ch.Fields)
				preview.Row(25).Dynamic(1)
				preview.Label(ch.Operation+" "+fallback(ch.Before.ID(), ch.Kind)+"  "+string(b), "LC")
			}
			preview.GroupEnd()
		}
		w.Row(28).Dynamic(2)
		if primary(w, fmt.Sprintf("Apply %d changes", len(s.Plan.Changes)), a.p) && !s.Busy {
			p := *s.Plan
			s.Busy = true
			c := a.rallyClient
			view := s.View
			s.Plan = nil
			a.work(func() {
				ctx, cancel := context.WithTimeout(a.ctx, 2*time.Minute)
				defer cancel()
				n, e := assistant.Apply(ctx, c, p)
				a.post(func() {
					s.Busy = false
					s.Status = fmt.Sprintf("Applied %d of %d changes", n, len(p.Changes))
					if e != nil {
						s.Status += ". " + e.Error() + ". Refresh and request a new plan before retrying."
					}
					s.Transcript += "\n\n" + s.Status
					a.refreshRally(view)
				})
			})
		}
		if w.ButtonText("Discard") {
			s.Plan = nil
		}
	}
	w.Row(62).Dynamic(1)
	s.Editor.Edit(w)
	w.Row(30).Ratio(.75, .25)
	w.LabelColored(s.Status, "LC", a.p.Muted)
	if s.Busy {
		if w.ButtonText("Stop") && s.TurnID != "" {
			a.rpc("turn/interrupt", map[string]any{"threadId": s.ThreadID, "turnId": s.TurnID}, nil)
		}
	} else if primary(w, "Ask AI", a.p) {
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
	if warning := a.catalog.SpeedWarning(s.Model, s.Tier); warning != "" {
		s.Status = warning
		return
	}
	s.Busy = true
	s.Transcript += "\n\nYou: " + prompt + "\n\n"
	setText(s.Editor, "")
	s.Status = "Thinking…"
	startTurn := func() {
		a.rpcResult("turn/start", map[string]any{"threadId": s.ThreadID, "input": codex.TextInput(prompt), "model": s.Model, "effort": s.Effort, "serviceTier": s.Tier}, func(raw json.RawMessage) {
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
	s.Scope.Expression = ""
	instructions := assistant.Instructions + "\nSelected workspace: " + s.Scope.Workspace + "\nSelected project: " + s.Scope.Project
	a.rpcResult("thread/start", map[string]any{"ephemeral": true, "model": s.Model, "serviceTier": s.Tier, "cwd": a.prefs.WorkingDirectory, "developerInstructions": instructions, "dynamicTools": assistant.Specs(), "sandbox": "read-only"}, func(raw json.RawMessage) {
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
		s.Transcript += str(p, "delta")
	case "turn/started":
		t, _ := p["turn"].(map[string]any)
		s.TurnID = str(t, "id")
	case "turn/completed":
		s.Busy = false
		s.TurnID = ""
		s.Status = "Ready"
		t, _ := p["turn"].(map[string]any)
		if err, ok := t["error"].(map[string]any); ok {
			s.Status = str(err, "message")
		}
	case "error":
		s.Busy = false
		err, _ := p["error"].(map[string]any)
		s.Status = str(err, "message")
	}
}
func (a *App) runDynamicTool(client *codex.Client, m codex.Message) {
	a.post(func() {
		p := codex.Decode(m.Params)
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
				a.post(func() {
					a.openRally(view.Page)
					tab := a.state.Current()
					v := a.rallyViews[tab.ID]
					if v.Detail != nil && v.Detail.dirty() {
						a.toast = "An unsaved item is open; save it before applying the AI view"
						return
					}
					v.Detail = nil
					setText(v.Query, view.Query)
					if view.Mode != "" {
						v.Mode = view.Mode
					}
					if view.Group != "" {
						v.Group = view.Group
					}
					a.refreshRally(v)
					s.View = v
				})
				return nil
			},
			Propose: func(plan assistant.Plan) error { a.post(func() { s.Plan = &plan }); return nil }}
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
