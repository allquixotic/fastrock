//go:build fltk_headless

package ui

import (
	"encoding/json"
	"image"
	"reflect"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestV39FormErrorsBlockDeliveryAndAllowCorrection(t *testing.T) {
	a := infoFixture(t)
	p := codex.Decode([]byte(`{"mode":"form","serverName":"Docs","requestedSchema":{"properties":{"count":{"type":"integer","minimum":1,"maximum":10},"email":{"type":"string","format":"email"},"optional":{"type":"string"}},"required":["count","email"]}}`))
	a.serverRequest(codex.Message{ID: json.RawMessage(`"form"`), Method: "mcpServer/elicitation/request", Origin: a.client}, p)
	setText(a.approvals[0].Questions[0].Editor, "11")
	a.submitApproval()
	r := &a.approvals[0]
	if r.Submitting || r.FormError != "Fix the highlighted fields to continue." || r.Questions[0].Error == "" || r.Questions[1].Error == "" || r.Questions[2].Error != "" || a.toast != "" {
		t.Fatal("invalid form lost inline errors", r, a.toast)
	}
	var answers []map[string]any
	if err := a.client.Call(a.ctx, "fixture/answers", nil, &answers); err != nil || len(answers) != 0 {
		t.Fatal("invalid form was delivered", answers, err)
	}
	setText(r.Questions[0].Editor, "7")
	setText(r.Questions[1].Editor, "person@example.test")
	a.submitApproval()
	drain(t, a, func() bool { return len(a.approvals) == 0 })
	if err := a.client.Call(a.ctx, "fixture/answers", nil, &answers); err != nil {
		t.Fatal(err)
	}
	if len(answers) != 1 || answers[0]["action"] != "accept" || !reflect.DeepEqual(answers[0]["content"], map[string]any{"count": float64(7), "email": "person@example.test"}) {
		t.Fatal("corrected form lost typed values or sent blank optional field", answers)
	}
}

func TestV39QuestionAndFormPresentation(t *testing.T) {
	a := infoFixture(t)
	question := map[string]any{"id": "choice", "header": "Approach", "question": "Choose the approach that fits this project, then add any details you need.", "isOther": true, "options": []any{map[string]any{"label": "Small change", "description": "Keep the current structure"}}}
	p := map[string]any{"questions": []any{question}}
	a.serverRequest(codex.Message{ID: json.RawMessage(`"question"`), Method: "item/tool/requestUserInput", Origin: a.client}, p)
	if a.approvals[0].Title != "Codex has a question" {
		t.Fatal(a.approvals[0].Title)
	}
	q := &a.approvals[0].Questions[0]
	if questionPlaceholder(*q) != "Add a note (optional)" {
		t.Fatal(questionPlaceholder(*q))
	}
	q.Selected = 1
	if questionPlaceholder(*q) != "Describe what you want instead" {
		t.Fatal(questionPlaceholder(*q))
	}
	setText(q.Editor, "Use another approach")
	if !reflect.DeepEqual(questionAnswers(*q), []string{"None of the above", "user_note: Use another approach"}) {
		t.Fatal(questionAnswers(*q))
	}
	p["questions"] = append(p["questions"].([]any), map[string]any{"id": "details", "header": "Details", "question": "Tell me more"})
	a.serverRequest(codex.Message{ID: json.RawMessage(`"questions"`), Method: "item/tool/requestUserInput", Origin: a.client}, p)
	if a.approvals[1].Title != "Codex has 2 questions" || questionPlaceholder(a.approvals[1].Questions[1]) != "Type your answer" {
		t.Fatal(a.approvals[1])
	}
	count := elicitationQuestion("count", codex.Decode([]byte(`{"type":"integer","title":"Count","minimum":1,"maximum":10}`)), true)
	count.Error = "enter a value of at least 1"
	boolean := elicitationQuestion("ready", codex.Decode([]byte(`{"type":"boolean","title":"Ready","description":"Confirm you are ready to proceed."}`)), true)
	var labels []command.Command
	h := desktop.NewHeadlessHarness(0, image.Pt(700, 1000), func(w *desktop.Window) {
		a.drawQuestion(w, q)
		a.drawQuestion(w, &count)
		a.drawQuestion(w, &boolean)
		labels = append([]command.Command(nil), w.Commands().Commands...)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	var texts []string
	var errorRed bool
	for _, label := range labels {
		if label.Kind != command.TextCmd {
			continue
		}
		texts = append(texts, label.Text.String)
		if label.Text.String == count.Error && label.Text.Foreground == a.p.Danger {
			errorRed = true
		}
	}
	joined := strings.Join(texts, "\n")
	for _, want := range []string{"Approach", "Choose the approach that fits this project", "Count *", "Whole number from 1 to 10", "Ready *", "Confirm you are ready to proceed."} {
		if !strings.Contains(joined, want) {
			t.Fatal("missing form text", want, joined)
		}
	}
	if strings.Count(joined, "Ready") != 1 || strings.Contains(joined, "Approach:") || !errorRed {
		t.Fatal("duplicate or clipped header, or error color missing", joined, errorRed)
	}
}

func TestV39RequestBadgeCountsOnlyOwningConversation(t *testing.T) {
	a := infoFixture(t)
	a.state.Chats["parent"] = &workspace.Conversation{ID: "parent", Agents: []workspace.Agent{{ID: "child"}}}
	a.state.Chats["other"] = &workspace.Conversation{ID: "other"}
	a.state.Open(workspace.Chat, "Parent", "parent", "")
	a.state.Open(workspace.Chat, "Other", "other", "")
	for i, thread := range []string{"child", "parent", "other"} {
		a.serverRequest(codex.Message{ID: json.RawMessage(string(rune('1' + i))), Method: "item/tool/requestUserInput", Origin: a.client}, map[string]any{"threadId": thread})
	}
	var labels []string
	h := desktop.NewHeadlessHarness(0, image.Pt(700, 800), func(w *desktop.Window) {
		a.drawApprovalFor(w, "parent")
		labels = nil
		for _, cmd := range w.Commands().Commands {
			if cmd.Kind == command.TextCmd {
				labels = append(labels, cmd.Text.String)
			}
		}
	})
	a.window = h.Master()
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	joined := strings.Join(labels, "\n")
	for _, want := range []string{"1 of 2", "From sub-agent child", "Submit", "Skip", "Skip sends no answers."} {
		if !strings.Contains(joined, want) {
			t.Fatal(want, joined)
		}
	}
	if strings.Contains(joined, "1 of 3") {
		t.Fatal("count included another conversation", joined)
	}
	a.pruneApprovals("", "1")
	h.Frame(false)
	if strings.Contains(strings.Join(labels, "\n"), "1 of") {
		t.Fatal("single request retained count badge", labels)
	}
}
