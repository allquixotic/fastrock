package ui

import (
	"encoding/json"
	"fmt"
	"golang.org/x/mobile/event/key"
	"sort"
	"time"

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
				item := question{ID: str(q, "id"), Header: str(q, "header"), Text: str(q, "question"), Selected: -1, Editor: textEditor("", false)}
				if secret, ok := q["isSecret"].(bool); ok && secret {
					item.Secret = true
					item.Editor.PasswordChar = '●'
				}
				if opts, ok := q["options"].([]any); ok {
					for _, v := range opts {
						o, _ := v.(map[string]any)
						item.Options = append(item.Options, str(o, "label"))
						item.Descriptions = append(item.Descriptions, str(o, "description"))
					}
				}
				if other, _ := q["isOther"].(bool); other && len(item.Options) > 0 {
					item.Options = append(item.Options, "None of the above")
					item.Descriptions = append(item.Descriptions, "Describe what you want in the note below")
				}
				r.Questions = append(r.Questions, item)
			}
		}
	case "mcpServer/elicitation/request":
		mode := str(p, "mode")
		if mode != "form" && mode != "url" {
			client := a.client
			a.work(func() { _ = client.Reject(m.ID, "Unsupported elicitation mode: "+mode) })
			a.toast = "Unsupported Codex elicitation mode: " + mode
			return
		}
		r.Elicitation = true
		r.Title = str(p, "message")
		r.URL = str(p, "url")
		schema, _ := p["requestedSchema"].(map[string]any)
		props, _ := schema["properties"].(map[string]any)
		names := make([]string, 0, len(props))
		for name := range props {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			field, _ := props[name].(map[string]any)
			required := false
			if list, ok := schema["required"].([]any); ok {
				for _, nameRequired := range list {
					if nameRequired == name {
						required = true
					}
				}
			}
			q := elicitationQuestion(name, field, required)
			r.Questions = append(r.Questions, q)
		}
	default:
		client := a.client
		a.work(func() { _ = client.Reject(m.ID, "Fastrock does not support this server request: "+m.Method) })
		a.toast = "Unsupported Codex request: " + m.Method
		return
	}
	r.Choices = approvalChoices(m.Method, p)
	r.Armed = time.Now().Add(300 * time.Millisecond)
	a.approvals = append(a.approvals, r)
}
func (a *App) drawApproval(w *nucular.Window) {
	if len(a.approvals) == 0 {
		return
	}
	if !a.activateApproval() {
		return
	}
	r := &a.approvals[0]
	w.Row(140).Dynamic(1)
	if box := w.GroupBegin("approval", nucular.WindowBorder); box != nil {
		box.Row(28).Dynamic(1)
		if box.ButtonText(r.Title) {
			r.Focused = true
			r.Armed = time.Now().Add(300 * time.Millisecond)
		}
		p := codex.Decode(r.Message.Params)
		box.Row(24).Dynamic(1)
		if box.ButtonText("View full request details") {
			scrub(p)
			b, _ := json.MarshalIndent(p, "", "  ")
			a.openText("Approval details", string(b))
		}
		if reason := str(p, "reason"); reason != "" {
			box.Row(35).Dynamic(1)
			box.LabelWrap(reason)
		}
		for i := range r.Questions {
			a.drawQuestion(box, &r.Questions[i])
		}
		box.GroupEnd()
	}
	if r.URL != "" {
		w.Row(28).Dynamic(2)
		if w.ButtonText("Open authorization page") {
			a.openURL(r.URL)
		}
		if w.ButtonText("Copy URL") {
			a.copyText(r.URL)
		}
	}
	if len(r.Choices) > 0 {
		for i, choice := range r.Choices {
			w.Row(28).Dynamic(1)
			if w.ButtonText(fmt.Sprintf("%d. %s [%c]", i+1, choice.Title, choice.Key)) {
				a.answerApproval(choice.Result)
				return
			}
		}
		return
	}
	w.Row(28).Dynamic(2)
	if primary(w, "Allow / Submit", a.p) {
		result := any(map[string]any{"decision": "accept"})
		if r.Message.Method == "item/tool/requestUserInput" {
			answers := map[string]any{}
			for _, q := range r.Questions {
				answers[q.ID] = map[string]any{"answers": questionAnswers(q)}
			}
			result = map[string]any{"answers": answers}
		}
		if r.Message.Method == "item/permissions/requestApproval" {
			p := codex.Decode(r.Message.Params)
			result = map[string]any{"permissions": p["permissions"], "scope": "turn"}
		}
		if r.Elicitation {
			content := map[string]any{}
			for _, q := range r.Questions {
				value, err := elicitationValue(q)
				if err != nil {
					a.toast = q.Header + ": " + err.Error()
					return
				}
				if value != nil {
					content[q.ID] = value
				}
			}
			result = map[string]any{"action": "accept", "content": content}
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
		if r.Elicitation {
			result = map[string]any{"action": "decline", "content": nil}
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

// Decisions are server-provided values, including structured policy amendments.
type approvalChoice struct {
	Title  string
	Key    rune
	Result any
}

func approvalChoices(method string, p map[string]any) []approvalChoice {
	if method == "item/permissions/requestApproval" {
		grant := func(scope string, strict bool) any {
			r := map[string]any{"permissions": p["permissions"], "scope": scope}
			if strict {
				r["strictAutoReview"] = true
			}
			return r
		}
		return []approvalChoice{{"Grant for this turn", 'y', grant("turn", false)}, {"Grant with strict auto review", 'r', grant("turn", true)}, {"Grant for this session", 'a', grant("session", false)}, {"Continue without permissions", 'd', map[string]any{"permissions": map[string]any{}, "scope": "turn"}}}
	}
	if method != "item/commandExecution/requestApproval" && method != "item/fileChange/requestApproval" {
		return nil
	}
	decisions, _ := p["availableDecisions"].([]any)
	if decisions == nil {
		decisions = []any{"accept"}
		if method == "item/fileChange/requestApproval" || p["networkApprovalContext"] != nil {
			decisions = append(decisions, "acceptForSession")
		}
		if p["additionalPermissions"] == nil {
			if proposal := p["proposedExecpolicyAmendment"]; proposal != nil {
				decisions = append(decisions, map[string]any{"acceptWithExecpolicyAmendment": map[string]any{"execpolicy_amendment": proposal}})
			}
			if proposals, ok := p["proposedNetworkPolicyAmendments"].([]any); ok {
				for _, proposal := range proposals {
					if rule, ok := proposal.(map[string]any); ok && str(rule, "action") == "allow" {
						decisions = append(decisions, map[string]any{"applyNetworkPolicyAmendment": map[string]any{"network_policy_amendment": rule}})
						break
					}
				}
			}
		}
		if method == "item/fileChange/requestApproval" {
			decisions = append(decisions, "decline")
		}
		decisions = append(decisions, "cancel")
	}
	out := make([]approvalChoice, 0, len(decisions))
	for _, decision := range decisions {
		name, _ := decision.(string)
		if obj, ok := decision.(map[string]any); ok {
			for k := range obj {
				name = k
			}
		}
		c := approvalChoice{Result: map[string]any{"decision": decision}}
		switch name {
		case "accept":
			c.Title = "Allow once"
			c.Key = 'y'
		case "acceptForSession":
			c.Title = "Allow for this session"
			c.Key = 'a'
		case "decline":
			c.Title = "Continue without this action"
			c.Key = 'd'
		case "cancel":
			c.Title = "Cancel and tell Codex what to do differently"
			c.Key = 'n'
		case "acceptWithExecpolicyAmendment":
			c.Title = "Allow and save the proposed command rule"
			c.Key = 'p'
		case "applyNetworkPolicyAmendment":
			c.Title = "Apply proposed network rule"
			c.Key = 'p'
		default:
			continue
		}
		out = append(out, c)
	}
	return out
}
func (a *App) activateApproval() bool {
	t := a.state.Current()
	if t == nil {
		return false
	}
	for i, r := range a.approvals {
		p := codex.Decode(r.Message.Params)
		if str(p, "threadId") == t.Target {
			if i > 0 {
				a.approvals[0], a.approvals[i] = a.approvals[i], a.approvals[0]
			}
			return true
		}
	}
	return false
}
func (a *App) approvalKey(event *nucular.KeyboardEvent) bool {
	if len(a.approvals) == 0 || !a.activateApproval() {
		return false
	}
	r := &a.approvals[0]
	if !r.Focused || time.Now().Before(r.Armed) || len(r.Choices) == 0 {
		return false
	}
	t := a.state.Current()
	if v := a.chats[t.Target]; v != nil && v.Editor.Active {
		r.Focused = false
		return false
	}
	e := event.Key()
	if e.Modifiers != 0 {
		return false
	}
	for i, c := range r.Choices {
		code := key.CodeA + key.Code(c.Key-'a')
		if event.HandleKey(code, 0) || i < 9 && event.HandleKey(key.Code1+key.Code(i), 0) || c.Key == 'n' && event.HandleKey(key.CodeEscape, 0) {
			a.answerApproval(c.Result)
			return true
		}
	}
	return false
}
