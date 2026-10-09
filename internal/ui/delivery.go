package ui

import (
	"encoding/json"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func deliveryScope(c *workspace.Conversation) codex.PermissionScope {
	if !c.Settings.Current {
		return codex.PermissionScope{}
	}
	return codex.PermissionScope{Approval: c.Settings.Approval, Sandbox: c.Settings.Sandbox, Network: c.Settings.Network, Roots: c.Settings.WritableRoots}
}

func deliveryTitle(title, fallback string) string {
	if title == "" {
		title = fallback
	}
	return "“" + cut(strings.Join(strings.Fields(title), " "), 40) + "”"
}
func deliveryApproval(d codex.DeliveryRequest, client *codex.Client) approval {
	from := deliveryTitle(d.From.Title, d.From.ID)
	to := deliveryTitle(d.To.Title, d.To.ID)
	id, _ := json.Marshal("delivery:" + d.ID)
	r := approval{Delivery: &d, ThreadID: d.To.ID, OriginThreadID: d.To.ID, Message: codex.Message{ID: id, Origin: client, Method: "fastrock/deliveryRequest"}, Title: "Agent in tab " + from + " wants to send this message", Armed: time.Now().Add(300 * time.Millisecond)}
	r.Params = map[string]any{"from": d.From.ID, "to": d.To.ID, "message": d.Text, "alwaysAsk": d.AlwaysAsk, "waitForReply": d.Wait}
	r.Content = approvalContent{Code: d.Text, Reason: "Delivering starts a turn in this tab, with this tab's permissions, as soon as it is idle. Deliver only messages you expect.", Caption: "The sending agent does not wait for an answer."}
	if d.Wait {
		r.Content.Caption = "The sending agent waits for this tab's answer."
	}
	if d.AlwaysAsk != "" {
		r.Content.Details = []string{"Always asks: " + d.AlwaysAsk + "."}
	}
	choice := func(label string, key rune, value string) approvalChoice {
		return approvalChoice{Title: label, Key: key, Result: map[string]any{"choice": value}}
	}
	r.Choices = []approvalChoice{choice("Deliver", 'y', "deliver")}
	if d.AlwaysAsk == "" {
		r.Choices = append(r.Choices, choice("Deliver, and allow "+from+" → "+to+" for this session", 'a', "session"))
	}
	r.Choices = append(r.Choices, choice("Decline", 'd', "decline"))
	return r
}
func (a *App) deliveryEvent(m codex.Message) {
	switch m.Method {
	case "fastrock/deliveryPending":
		var request codex.DeliveryRequest
		if json.Unmarshal(m.Params, &request) != nil {
			return
		}
		for _, d := range a.deliveryConsents {
			if d.ID == request.ID {
				d.To = request.To.ID
				if d.Waiter != nil {
					d.Waiter.Target = request.To.ID
				}
			}
		}
	case "fastrock/deliveryRequest":
		var request codex.DeliveryRequest
		if json.Unmarshal(m.Params, &request) != nil || request.ID == "" {
			return
		}
		c := a.state.Chats[request.To.ID]
		open := false
		for _, tab := range a.state.Tabs {
			if tab.Kind == workspace.Chat && tab.Target == request.To.ID {
				open = true
				break
			}
		}
		if !open || c == nil || c.NoMessages {
			a.rpc("fastrock/cancelDelivery", map[string]any{"data": map[string]string{"ID": request.ID}}, nil)
			return
		}
		r := deliveryApproval(request, a.client)
		for i, old := range a.approvals {
			if old.Delivery != nil && old.Delivery.ID == request.ID {
				if old.Delivery.Revision >= request.Revision {
					return
				}
				r.Focused = old.Focused
				a.approvals[i] = r
				return
			}
		}
		a.approvals = append(a.approvals, r)
	case "fastrock/deliveryResult":
		var result codex.DeliveryResult
		if json.Unmarshal(m.Params, &result) != nil {
			return
		}
		for _, d := range a.deliveryConsents {
			if d.ID != result.ID || !a.finishConsent(d) {
				continue
			}
			if !result.Success {
				a.removeReplyWait(d.Waiter)
				a.answerTool(d.Message, result.Error, false)
				return
			}
			a.recordMail(d.From, mailMessage{From: d.From, To: result.Request.To.ID, Text: result.Request.Text, At: time.Now().Unix()})
			if d.Waiter == nil {
				a.answerTool(d.Message, map[string]any{"delivered": true, "threadId": result.Request.To.ID}, true)
			} else {
				d.Waiter.Ready = true
				if d.Waiter.Completed {
					a.finishReply(d.Waiter)
				} else {
					a.armReplyTimeout(d.Waiter, d.Timeout)
				}
			}
			return
		}
	case "fastrock/deliveryResolved":
		var result codex.DeliveryResult
		if json.Unmarshal(m.Params, &result) != nil {
			return
		}
		for i := len(a.approvals) - 1; i >= 0; i-- {
			if d := a.approvals[i].Delivery; d != nil && d.ID == result.ID {
				a.approvals = append(a.approvals[:i], a.approvals[i+1:]...)
			}
		}
		if result.Success {
			a.recordMail(result.Request.To.ID, mailMessage{From: result.Request.From.ID, To: result.Request.To.ID, Text: result.Request.Text, At: time.Now().Unix()})
		}
		if c := a.state.Chats[result.Request.To.ID]; c != nil {
			text := "You declined a message from tab " + deliveryTitle(result.Request.From.Title, result.Request.From.ID)
			if result.Success {
				text = "You delivered a message from tab " + deliveryTitle(result.Request.From.Title, result.Request.From.ID)
				if result.Session {
					text += " and allowed its messages to this tab for this session"
				}
			} else if result.Error != "the user declined delivery" {
				text = "Message delivery ended: " + result.Error
			}
			c.Append(workspace.NewID("delivery-decision"), "notice", "notice", text)
		}
	}
}
func (a *App) answerDelivery(r approval, result any) {
	choice, _ := result.(map[string]any)
	answer := codex.DeliveryAnswer{ID: r.Delivery.ID, Revision: r.Delivery.Revision, Choice: str(choice, "choice")}
	a.approvals[0].Submitting = true
	a.rpcResult("fastrock/answerDelivery", map[string]any{"data": answer}, nil, func(err error) {
		for i := range a.approvals {
			d := a.approvals[i].Delivery
			if d != nil && d.ID == answer.ID && d.Revision == answer.Revision {
				a.approvals[i].Submitting = false
			}
		}
		a.toast = err.Error()
	})
}
func (a *App) removeReplyWait(wait *replyWait) {
	if wait == nil {
		return
	}
	if wait.Timer != nil {
		wait.Timer.Stop()
	}
	for i, candidate := range a.replyWaits {
		if candidate == wait {
			a.replyWaits = append(a.replyWaits[:i], a.replyWaits[i+1:]...)
			return
		}
	}
}
func (a *App) finishReply(wait *replyWait) {
	var reply strings.Builder
	for _, block := range wait.Order {
		if reply.Len() >= 20000 {
			break
		}
		reply.WriteString(wait.Blocks[block])
		reply.WriteByte('\n')
	}
	a.removeReplyWait(wait)
	a.answerTool(wait.Message, map[string]any{"threadId": wait.Target, "reply": cut(reply.String(), 20000), "status": wait.Status}, true)
}
