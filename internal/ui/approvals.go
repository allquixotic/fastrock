package ui

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

func (a *App) serverRequest(m codex.Message, p map[string]any) {
	if m.Origin == nil {
		m.Origin = a.client
	}
	r := approval{Message: m, Title: m.Method, ThreadID: str(p, "threadId"), Params: p}
	r.OriginThreadID = r.ThreadID
	switch m.Method {
	case "currentTime/read":
		client := m.Origin
		if client != nil {
			a.controlWork(func() { _ = client.Respond(m.ID, map[string]any{"currentTimeAt": time.Now().Unix()}) })
		}
		return
	case "item/commandExecution/requestApproval":
		r.Title = "Allow command: " + str(p, "command")
	case "item/fileChange/requestApproval":
		r.Title = "Allow file changes"
		if reason := str(p, "reason"); reason != "" {
			r.Title += ": " + reason
		}
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
					item.Other = true
					item.Options = append(item.Options, "None of the above")
					item.Descriptions = append(item.Descriptions, "Describe what you want in the note below")
				}
				r.Questions = append(r.Questions, item)
			}
		}
		if len(r.Questions) == 1 {
			r.Title = "Codex has a question"
		} else {
			r.Title = fmt.Sprintf("Codex has %d questions", len(r.Questions))
		}
	case "mcpServer/elicitation/request":
		mode := str(p, "mode")
		if mode != "form" && mode != "url" {
			client := m.Origin
			if client != nil {
				a.controlWork(func() { _ = client.Respond(m.ID, map[string]any{"action": "cancel", "content": nil}) })
			}
			a.toast = "Unsupported Codex elicitation mode: " + mode
			return
		}
		configureElicitation(&r)
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
		client := m.Origin
		if client != nil {
			a.controlWork(func() {
				_ = client.RejectCode(m.ID, codex.CodeMethodNotFound, "Fastrock does not support this server request: "+m.Method)
			})
		}
		a.toast = "Unsupported Codex request: " + m.Method
		return
	}
	if !r.Elicitation {
		r.Choices = approvalChoices(m.Method, p)
	}
	a.prepareApproval(&r)

	if r.ThreadID != "" && (a.assistant == nil || r.ThreadID != a.assistant.ThreadID) {
		found := false
		for _, tab := range a.state.Tabs {
			if tab.Target == r.ThreadID {
				found = true
				break
			}
		}
		if !found {
			for _, tab := range a.state.Tabs {
				if parent := a.state.Chats[tab.Target]; parent != nil {
					for _, agent := range parent.Agents {
						if agent.ID == r.ThreadID {
							r.ThreadID = parent.ID
							found = true
							break
						}
					}
				}
				if found {
					break
				}
			}
		}
		if !found {
			client := m.Origin
			if client != nil {
				a.controlWork(func() { _ = client.Reject(m.ID, "No open document can display this request") })
			}
			a.toast = "A request for a closed conversation was declined"
			return
		}
	}
	r.Armed = time.Now().Add(300 * time.Millisecond)
	a.approvals = append(a.approvals, r)
	a.fetchApprovalDiff(r)
}
func (a *App) drawApproval(w *desktop.Window) {
	t := a.state.Current()
	if t == nil {
		return
	}
	a.drawApprovalFor(w, t.Target)
}
func (a *App) drawApprovalFor(w *desktop.Window, thread string) {
	if len(a.approvals) == 0 {
		return
	}
	if !a.activeApprovalFor(thread) {
		return
	}
	r := &a.approvals[0]
	count := 0
	for _, pending := range a.approvals {
		if pending.ThreadID == thread {
			count++
		}
	}
	if r.Submitting {
		muted(w, "Sending your answer…", a.p)
		return
	}
	a.drawApprovalCard(w, r, count)
}
func (a *App) openApprovalDetails(r *approval) {
	original, _ := json.Marshal(r.Params)
	copied := codex.Decode(original)
	scrub(copied)
	b, _ := json.MarshalIndent(copied, "", "  ")
	a.openText("Approval details", string(b))
}
func (a *App) drawApprovalActions(w *desktop.Window, r *approval) {
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
			if a.drawApprovalChoice(w, r, i, choice) {
				a.answerApproval(choice.Result)
				return
			}
		}
		a.drawApprovalHint(w, approvalKeyHint)
		return
	}
	if approvalHasDecisions(r) {
		a.drawApprovalHint(w, approvalNoChoicesHint)
		return
	}
	if r.FormError != "" {
		a.drawSettingsError(w, r.FormError)
	}
	if r.Message.Method == "item/tool/requestUserInput" {
		a.drawApprovalHint(w, approvalUnansweredHint)
	}
	buttons := 2
	if r.Elicitation {
		buttons = 3
	}
	w.Row(28).Dynamic(buttons)
	submitLabel, declineLabel := "Allow / Submit", "Decline"
	if r.Elicitation || r.Message.Method == "item/tool/requestUserInput" {
		submitLabel = "Submit"
	}
	if r.Message.Method == "item/tool/requestUserInput" {
		declineLabel = "Skip"
	}
	if primary(w, submitLabel, a.p) {
		a.submitApproval()
	}

	if w.ButtonText(declineLabel) {
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
	if r.Elicitation && w.ButtonText("Cancel") {
		a.answerApproval(elicitationResponse("cancel", nil))
	}
}
func (a *App) submitApproval() {
	if len(a.approvals) == 0 || a.approvals[0].Submitting {
		return
	}
	r := &a.approvals[0]
	if approvalHasDecisions(r) && len(r.Choices) == 0 {
		return
	}
	result := any(map[string]any{"decision": "accept"})
	if r.Message.Method == "item/tool/requestUserInput" {
		answers := map[string]any{}
		for _, q := range r.Questions {
			answers[q.ID] = map[string]any{"answers": questionAnswers(q)}
		}
		result = map[string]any{"answers": answers}
	}
	if r.Message.Method == "item/permissions/requestApproval" {
		p := r.Params
		result = map[string]any{"permissions": p["permissions"], "scope": "turn"}
	}
	if r.Elicitation {
		content, valid := elicitationContent(r)
		if !valid {
			return
		}
		result = map[string]any{"action": "accept", "content": content}
	}
	a.answerApproval(result)
}
func (a *App) answerApproval(result any) {
	if len(a.approvals) == 0 || a.approvals[0].Submitting {
		return
	}
	r := a.approvals[0]
	if r.Delivery != nil {
		a.answerDelivery(r, result)
		return
	}
	openURL := ""
	value, _ := json.Marshal(result)
	for _, choice := range r.Choices {
		encoded, _ := json.Marshal(choice.Result)
		if string(encoded) == string(value) {
			openURL = choice.OpenURL
			break
		}
	}
	c := r.Message.Origin
	if c == nil || c != a.client {
		a.toast = "This request belongs to a disconnected Codex session"
		return
	}
	a.approvals[0].Submitting = true
	a.controlWork(func() {
		err := c.Respond(r.Message.ID, result)
		a.post(func() {
			if err == nil {
				if openURL != "" {
					a.openURL(openURL)
				}
				// A resolved notification may remove the card before this write
				// acknowledgement reaches the UI. Keep the delivered decision.
				note := approvalDecisionText(r, result)
				if chat := a.state.Chats[r.ThreadID]; chat != nil {
					chat.Append(workspace.NewID("decision"), "notice", "notice", note)
				} else if a.assistant != nil && a.assistant.ThreadID == r.ThreadID {
					a.assistant.History.Append(workspace.NewID("decision"), "notice", "notice", note)
				}
			}
			for i := range a.approvals {
				if string(a.approvals[i].Message.ID) != string(r.Message.ID) || a.approvals[i].Message.Origin != c {
					continue
				}
				if err != nil {
					a.approvals[i].Submitting = false
					a.toast = fmt.Sprintf("Could not answer Codex: %v", err)
				} else {
					a.approvals = append(a.approvals[:i], a.approvals[i+1:]...)
				}
				break
			}
		})
	}, func() {
		for i := range a.approvals {
			if string(a.approvals[i].Message.ID) == string(r.Message.ID) {
				a.approvals[i].Submitting = false
			}
		}
	})
}

func approvalDecisionText(r approval, result any) string {
	value, _ := json.Marshal(result)
	for _, choice := range r.Choices {
		candidate, _ := json.Marshal(choice.Result)
		if string(candidate) == string(value) {
			return "You chose “" + choice.Title + "” for Codex's request."
		}
	}
	if r.Elicitation {
		return "You answered Codex's information request."
	}
	if len(r.Questions) > 0 {
		return "You submitted answers to Codex."
	}
	return "You answered Codex's request."
}
func (a *App) pruneApprovals(thread, id string) {
	out := a.approvals[:0]
	for _, r := range a.approvals {
		if (thread != "" && r.ThreadID == thread) || (id != "" && string(r.Message.ID) == id) {
			continue
		}
		out = append(out, r)
	}
	clear(a.approvals[len(out):])
	a.approvals = out
}

// Decisions are server-provided values, including structured policy amendments.
type approvalChoice struct {
	Title   string
	Key     rune
	Result  any
	OpenURL string
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
		return []approvalChoice{{Title: "Grant for this turn", Key: 'y', Result: grant("turn", false)}, {Title: "Grant with strict auto review", Key: 'r', Result: grant("turn", true)}, {Title: "Grant for this session", Key: 'a', Result: grant("session", false)}, {Title: "Continue without permissions", Key: 'd', Result: map[string]any{"permissions": map[string]any{}, "scope": "turn"}}}
	}
	if method != "item/commandExecution/requestApproval" && method != "item/fileChange/requestApproval" {
		return nil
	}
	decisions, _ := p["availableDecisions"].([]any)
	fileChange := method == "item/fileChange/requestApproval"
	network := p["networkApprovalContext"] != nil
	if decisions == nil {
		decisions = []any{"accept"}
		switch {
		case fileChange:
			decisions = append(decisions, "acceptForSession", "decline")
		case network:
			decisions = append(decisions, "acceptForSession")
			if proposals, ok := p["proposedNetworkPolicyAmendments"].([]any); ok {
				for _, proposal := range proposals {
					if rule, ok := proposal.(map[string]any); ok && str(rule, "action") == "allow" {
						decisions = append(decisions, map[string]any{"applyNetworkPolicyAmendment": map[string]any{"network_policy_amendment": rule}})
						break
					}
				}
			}
		case p["additionalPermissions"] == nil:
			if proposal := p["proposedExecpolicyAmendment"]; proposal != nil {
				decisions = append(decisions, map[string]any{"acceptWithExecpolicyAmendment": map[string]any{"execpolicy_amendment": proposal}})
			}
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
			c.Title = "Yes, proceed"
			if network {
				c.Title = "Yes, just this once"
			}
			c.Key = 'y'
		case "acceptForSession":
			c.Title = "Yes, and don't ask again for this command in this session"
			if fileChange {
				c.Title = "Yes, and don't ask again for these files"
			} else if network {
				c.Title = "Yes, and allow this host for this conversation"
			} else if p["additionalPermissions"] != nil {
				c.Title = "Yes, and allow these permissions for this session"
			}
			c.Key = 'a'
		case "decline":
			c.Title = "No, continue without running it"
			if fileChange {
				c.Title = "No, continue without these changes"
			}
			c.Key = 'd'
		case "cancel":
			c.Title = "No, and tell Codex what to do differently"
			c.Key = 'n'
		case "acceptWithExecpolicyAmendment":
			if network {
				continue
			}
			amendment, _ := decision.(map[string]any)[name].(map[string]any)
			prefix := approvalPrefix(amendment["execpolicy_amendment"])
			if prefix == "" || strings.ContainsAny(prefix, "\r\n") {
				continue
			}
			c.Title = "Yes, and don't ask again for commands that start with `" + prefix + "`"
			c.Key = 'p'
		case "applyNetworkPolicyAmendment":
			amendment, _ := decision.(map[string]any)[name].(map[string]any)
			rule, _ := amendment["network_policy_amendment"].(map[string]any)
			if str(rule, "action") == "allow" {
				c.Title = "Yes, and allow this host in the future"
				c.Key = 'h'
			} else {
				c.Title = "No, and block this host in the future"
				c.Key = 'b'
			}
		default:
			continue
		}
		out = append(out, c)
	}
	return out
}
func (a *App) activeApprovalFor(thread string) bool {
	for i, r := range a.approvals {
		if r.ThreadID == thread {
			if i > 0 {
				a.approvals[0], a.approvals[i] = a.approvals[i], a.approvals[0]
			}
			return true
		}
	}
	return false
}
func (a *App) activateApproval() bool {
	t := a.state.Current()
	if t == nil {
		return false
	}
	return a.activeApprovalFor(t.Target)
}
func (a *App) approvalKey(event *desktop.KeyboardEvent) bool {
	if len(a.approvals) == 0 || !a.activateApproval() {
		return false
	}
	r := &a.approvals[0]
	if r.Submitting || time.Now().Before(r.Armed) {
		return false
	}
	t := a.state.Current()
	if v := a.chats[t.Target]; v != nil && v.Editor.Active {
		r.Focused = false
		return false
	}
	if r.Elicitation && event.HandleKey(key.CodeEscape, 0) {
		if cancel := approvalCancelIndex(r.Choices); cancel >= 0 {
			a.answerApproval(r.Choices[cancel].Result)
		} else {
			a.answerApproval(elicitationResponse("cancel", nil))
		}
		return true
	}
	if r.CodeEditor != nil && r.CodeEditor.Active {
		return false
	}
	if len(r.Choices) == 0 {
		return false
	}
	if !event.HandleKeyAny() {
		return false
	}
	e := event.Key()
	event.Unhandle()
	if e.Modifiers != 0 {
		return false
	}
	if event.HandleKey(key.CodeDownArrow, 0) {
		if r.Focused {
			r.Selected = (r.Selected + 1) % len(r.Choices)
		}
		r.Focused = true
		return true
	}
	if event.HandleKey(key.CodeUpArrow, 0) {
		if r.Focused {
			r.Selected = (r.Selected + len(r.Choices) - 1) % len(r.Choices)
		} else {
			r.Selected = len(r.Choices) - 1
		}
		r.Focused = true
		return true
	}
	if !r.Focused {
		return false
	}
	if event.HandleKey(key.CodeReturnEnter, 0) {
		a.answerApproval(r.Choices[r.Selected].Result)
		return true
	}
	if cancel := approvalCancelIndex(r.Choices); cancel >= 0 && event.HandleKey(key.CodeEscape, 0) {
		a.answerApproval(r.Choices[cancel].Result)
		return true
	}
	for i, c := range r.Choices {
		code := key.CodeA + key.Code(c.Key-'a')
		if event.HandleKey(code, 0) || i < 9 && event.HandleKey(key.Code1+key.Code(i), 0) {
			a.answerApproval(c.Result)
			return true
		}
	}
	return false
}

func approvalCancelIndex(choices []approvalChoice) int {
	fallback := -1
	for i, c := range choices {
		if c.Key == 'n' {
			return i
		}
		if c.Key == 'd' {
			fallback = i
		}
	}
	return fallback
}

// Explicitly end requests whose owning document closes; hiding a card must not
// strand the server waiting for an answer that no window can provide.
func (a *App) rejectClosingApprovals(thread string) {
	for _, request := range a.approvals {
		if request.ThreadID != thread || request.Submitting || request.Message.Origin == nil {
			continue
		}
		message := request.Message
		if request.Delivery != nil {
			a.rpc("fastrock/cancelDelivery", map[string]any{"data": map[string]string{"ID": request.Delivery.ID}}, nil)
			continue
		}
		a.controlWork(func() {
			var err error
			if request.Elicitation {
				err = message.Origin.Respond(message.ID, elicitationResponse("cancel", nil))
			} else {
				err = message.Origin.Reject(message.ID, "The user closed the owning conversation")
			}
			if err != nil {
				a.post(func() { a.report(fmt.Errorf("dismiss closed conversation request: %w", err)) })
			}
		})
	}
	a.pruneApprovals(thread, "")
}
