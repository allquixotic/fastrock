package ui

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
)

func (a *App) serverRequest(m codex.Message, p map[string]any) {
	r := approval{Message: m, Title: m.Method}
	switch m.Method {
	case "item/commandExecution/requestApproval":
		r.Title = "Allow command: " + str(p, "command")
	case "item/fileChange/requestApproval":
		r.Title = "Allow file changes: " + str(p, "reason")
	case "item/permissions/requestApproval":
		r.Title = "Codex requests additional permissions"
	case "item/tool/requestUserInput":
		r.Title = "Codex needs your input"
		if questions, ok := p["questions"].([]any); ok {
			for _, v := range questions {
				q, _ := v.(map[string]any)
				item := question{ID: str(q, "id"), Header: str(q, "header"), Text: str(q, "question"), Editor: textEditor("", false)}
				if secret, ok := q["isSecret"].(bool); ok && secret {
					item.Secret = true
					item.Editor.PasswordChar = '●'
				}
				if opts, ok := q["options"].([]any); ok {
					for _, v := range opts {
						o, _ := v.(map[string]any)
						item.Options = append(item.Options, str(o, "label"))
					}
				}
				r.Questions = append(r.Questions, item)
			}
		}
	default:
		client := a.client
		a.work(func() { _ = client.Reject(m.ID, "Fastrock does not support this server request: "+m.Method) })
		a.toast = "Unsupported Codex request: " + m.Method
		return
	}
	a.approvals = append(a.approvals, r)
}
func (a *App) drawApproval(w *nucular.Window) {
	if len(a.approvals) == 0 {
		return
	}
	r := &a.approvals[0]
	w.Row(140).Dynamic(1)
	if box := w.GroupBegin("approval", nucular.WindowBorder); box != nil {
		title(box, r.Title, a.p)
		p := codex.Decode(r.Message.Params)
		if reason := str(p, "reason"); reason != "" {
			box.Row(35).Dynamic(1)
			box.LabelWrap(reason)
		}
		for i := range r.Questions {
			q := &r.Questions[i]
			muted(box, q.Header+": "+q.Text, a.p)
			if len(q.Options) > 0 {
				box.Row(28).Dynamic(1)
				q.Selected = box.ComboSimple(q.Options, q.Selected, 28)
			}
			box.Row(28).Dynamic(1)
			q.Editor.Edit(box)
		}
		box.GroupEnd()
	}
	w.Row(28).Dynamic(2)
	if primary(w, "Allow / Submit", a.p) {
		result := any(map[string]any{"decision": "accept"})
		if r.Message.Method == "item/tool/requestUserInput" {
			answers := map[string]any{}
			for _, q := range r.Questions {
				value := strings.TrimSpace(text(q.Editor))
				if value == "" && len(q.Options) > 0 {
					value = q.Options[q.Selected]
				}
				answers[q.ID] = map[string]any{"answers": []string{value}}
			}
			result = map[string]any{"answers": answers}
		}
		if r.Message.Method == "item/permissions/requestApproval" {
			p := codex.Decode(r.Message.Params)
			result = map[string]any{"permissions": p["permissions"], "scope": "turn"}
		}
		a.answerApproval(result)
	}
	if w.ButtonText("Decline") {
		result := any(map[string]any{"decision": "decline"})
		if r.Message.Method == "item/tool/requestUserInput" {
			result = map[string]any{"answers": map[string]any{}}
		}
		if r.Message.Method == "item/permissions/requestApproval" {
			result = map[string]any{"permissions": map[string]any{}, "scope": "turn"}
		}
		a.answerApproval(result)
	}
}
func (a *App) answerApproval(result any) {
	r := a.approvals[0]
	a.approvals = a.approvals[1:]
	c := a.client
	a.work(func() {
		e := c.Respond(r.Message.ID, result)
		if e != nil {
			a.post(func() { a.toast = fmt.Sprintf("Could not answer Codex: %v", e) })
		}
	})
}

var _ json.RawMessage
