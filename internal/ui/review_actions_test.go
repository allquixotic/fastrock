//go:build nucular_headless

package ui

import (
	"bufio"
	"context"
	"encoding/json"
	"image"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

// This subprocess handles only local protocol fixtures; it never starts Codex.
func TestReviewRPCFixture(t *testing.T) {
	if len(os.Args) == 0 || os.Args[len(os.Args)-1] != "fastrock-review-fixture" {
		return
	}
	scanner := bufio.NewScanner(os.Stdin)
	writer := json.NewEncoder(os.Stdout)
	pageAttempts := 0
	for scanner.Scan() {
		var request codex.Message
		if json.Unmarshal(scanner.Bytes(), &request) != nil || request.Method == "" {
			continue
		}
		params := codex.Decode(request.Params)
		var result any
		var rpcError *codex.RPCError
		switch request.Method {
		case "thread/list":
			if str(params, "cursor") == "next" {
				pageAttempts++
				if pageAttempts == 1 {
					rpcError = &codex.RPCError{Code: -32000, Message: "fixture page unavailable"}
				} else {
					result = map[string]any{"data": []any{map[string]any{"id": "older", "name": "Older thread"}}, "nextCursor": nil}
				}
			} else {
				result = map[string]any{"data": []any{map[string]any{"id": "recent", "name": "Recent thread"}}, "nextCursor": "next"}
			}
		case "thread/goal/get":
			result = map[string]any{"goal": nil}
			if str(params, "threadId") == "unsupported" {
				rpcError = &codex.RPCError{Code: codex.CodeMethodNotFound, Message: "method unavailable"}
			}
		default:
			result = map[string]any{}
		}
		encoded, _ := json.Marshal(result)
		_ = writer.Encode(codex.Message{ID: request.ID, Result: encoded, Error: rpcError})
	}
	os.Exit(0)
}

func reviewFixture(t *testing.T) *App {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client, err := codex.StartCommand(ctx, executable, []string{"-test.run=^TestReviewRPCFixture$", "--", "fastrock-review-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	return &App{ctx: ctx, client: client, state: workspace.NewState(), updates: make(chan func(), 128), chats: map[string]*chatView{}, p: colors(false)}
}

func TestReviewTargetsUseProtocolVariants(t *testing.T) {
	for _, tc := range []struct {
		kind  int
		input string
		want  map[string]any
	}{
		{0, "ignored", map[string]any{"type": "uncommittedChanges"}},
		{1, " main ", map[string]any{"type": "baseBranch", "branch": "main"}},
		{2, "abc123", map[string]any{"type": "commit", "sha": "abc123"}},
		{3, "Check correctness", map[string]any{"type": "custom", "instructions": "Check correctness"}},
	} {
		got, err := reviewTarget(tc.kind, tc.input)
		if err != nil || !reflect.DeepEqual(got, tc.want) {
			t.Fatalf("review target %d: %v, %v", tc.kind, got, err)
		}
	}
	for _, kind := range []int{-1, 1, 2, 3, 4} {
		if _, err := reviewTarget(kind, " "); err == nil {
			t.Fatalf("invalid target %d accepted", kind)
		}
	}
}

func TestMemoryDefaultsAndLocks(t *testing.T) {
	fields := schemaConfigFields(map[string]any{})
	seen := map[string]bool{}
	for i := range fields {
		f := &fields[i]
		if f.Key == "memories.use_memories" || f.Key == "memories.generate_memories" {
			seen[f.Key] = true
			if !memoryValue(f) {
				t.Fatalf("%s did not default on", f.Key)
			}
			f.Value = false
			if memoryValue(f) {
				t.Fatalf("explicit false ignored for %s", f.Key)
			}
		}
	}
	if len(seen) != 2 {
		t.Fatal("memory schema did not expose both settings")
	}
	f := &configField{Key: "memories.use_memories", Value: false, Baseline: "false", Editor: textEditor("true", false)}
	if !memoryValue(f) {
		t.Fatal("pending memory choice was lost before acknowledgement")
	}
	data := map[string]any{"origins": map[string]any{"memories": map[string]any{"name": map[string]any{"type": "managed"}}}}
	if _, locked := configOrigin(data, "memories.use_memories"); !locked {
		t.Fatal("parent policy did not lock the memory setting")
	}
}

func TestBedrockRegionsPreserveUnknownConfiguration(t *testing.T) {
	labels, selected := bedrockRegionChoices(" eu-future-1 ")
	if len(labels) != 15 || selected != 14 || !strings.Contains(labels[selected], "eu-future-1") {
		t.Fatalf("unknown region lost: %v / %d", labels, selected)
	}
	_, selected = bedrockRegionChoices(" US-EAST-1 ")
	if selected != 1 {
		t.Fatal("known region was not matched")
	}
	seen := map[string]bool{}
	for _, region := range bedrockRegions {
		if seen[region.code] {
			t.Fatalf("duplicate region %s", region.code)
		}
		seen[region.code] = true
	}
}

func TestApprovalNoticeDoesNotRepeatPrivateAnswers(t *testing.T) {
	r := approval{Elicitation: true}
	if notice := approvalDecisionText(r, map[string]any{"password": "private-secret"}); strings.Contains(notice, "private-secret") {
		t.Fatal("answer leaked into transcript")
	}
	r.Choices = approvalChoices("item/fileChange/requestApproval", nil)
	if notice := approvalDecisionText(r, r.Choices[0].Result); !strings.Contains(notice, r.Choices[0].Title) {
		t.Fatal("notice lost the selected decision")
	}
}

func TestApprovalKeyboardFocusAndDecisions(t *testing.T) {
	a := reviewFixture(t)
	c := &workspace.Conversation{ID: "a"}
	a.state.Chats[c.ID] = c
	a.state.Open(workspace.Chat, "a", "a", "")
	a.chats[c.ID] = newChatView()
	choices := approvalChoices("item/permissions/requestApproval", map[string]any{})
	a.approvals = []approval{{ThreadID: c.ID, Message: codex.Message{ID: json.RawMessage("7"), Origin: a.client}, Choices: choices}}
	h := nucular.NewHeadlessHarness(0, image.Pt(500, 300), func(w *nucular.Window) {
		for event := range w.Input().Keyboard.Events() {
			a.approvalKey(event)
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	press := func(code key.Code, modifiers key.Modifiers) { h.Key(code, modifiers); h.Frame(false) }
	press(key.CodeY, key.ModShift)
	if a.approvals[0].Submitting || a.approvals[0].Focused {
		t.Fatal("unfocused modified letter answered approval")
	}
	a.chats[c.ID].Editor.Active = true
	press(key.CodeDownArrow, 0)
	if a.approvals[0].Focused {
		t.Fatal("approval stole input from composer")
	}
	a.chats[c.ID].Editor.Active = false
	press(key.CodeDownArrow, 0)
	press(key.CodeDownArrow, 0)
	if !a.approvals[0].Focused || a.approvals[0].Selected != 1 {
		t.Fatal("arrow navigation did not move focus")
	}
	press(key.CodeY, key.ModControl)
	if a.approvals[0].Submitting || a.approvals[0].Selected != 1 {
		t.Fatal("modified letter answered a focused approval")
	}
	press(key.CodeReturnEnter, 0)
	drain(t, a, func() bool { return len(a.approvals) == 0 })
	if len(c.Blocks) != 1 || !strings.Contains(c.Blocks[0].Text, choices[1].Title) {
		t.Fatal("Enter did not record the selected approval")
	}
	if i := approvalCancelIndex(choices); i != len(choices)-1 {
		t.Fatal("permission requests have no Escape decision")
	}
	a.approvals = []approval{{Focused: true, ThreadID: c.ID, Message: codex.Message{ID: json.RawMessage("8"), Origin: a.client}, Choices: choices}}
	press(key.CodeEscape, 0)
	drain(t, a, func() bool { return len(a.approvals) == 0 })
	if len(c.Blocks) != 2 || !strings.Contains(c.Blocks[1].Text, "Continue without permissions") {
		t.Fatal("Escape did not select the non-granting decision")
	}
	a.approvals = []approval{{ThreadID: c.ID, Message: codex.Message{ID: json.RawMessage("9"), Origin: a.client}, Choices: choices}}
	a.answerApproval(choices[0].Result)
	a.pruneApprovals(c.ID, "") // Server resolution may arrive before write acknowledgement.
	drain(t, a, func() bool { return len(c.Blocks) == 3 })
	if !strings.Contains(c.Blocks[2].Text, choices[0].Title) {
		t.Fatal("early server resolution lost the delivered decision")
	}
}

func TestGoalEditPreservesStatusAndClearsRemovedBudget(t *testing.T) {
	p, err := goalParams("a", " objective ", "", false)
	if err != nil || p["objective"] != "objective" || p["tokenBudget"] != nil {
		t.Fatalf("invalid goal edit: %v, %v", p, err)
	}
	if _, exists := p["status"]; exists {
		t.Fatal("editing a paused goal unexpectedly resumed it")
	}
	p, err = goalParams("a", "objective", "1200000", true)
	if err != nil || p["status"] != "active" || p["tokenBudget"] != int64(1200000) {
		t.Fatal("new goal lost its budget or active state")
	}
	for _, budget := range []string{"-1", "0", "1.2", "overflow999999999999999999999"} {
		if _, err := goalParams("a", "objective", budget, true); err == nil {
			t.Fatalf("invalid budget %s accepted", budget)
		}
	}
	if _, err := goalParams("a", " ", "", true); err == nil {
		t.Fatal("empty goal accepted")
	}
}

func TestGoalUsageAndLiveUpdates(t *testing.T) {
	goal := map[string]any{"tokensUsed": float64(1200000), "timeUsedSeconds": float64(7500)}
	if got := goalDetail(goal); got != "1.2M tokens · 2h 5m" {
		t.Fatal(got)
	}
	if goalStatusLabel("budgetLimited") != "Budget reached" {
		t.Fatal("raw goal status shown")
	}
	a := &App{state: workspace.NewState(), infoViews: map[string]*conversationInfo{"a": {GoalLoading: true, GoalGeneration: 1}}}
	a.state.Chats["a"] = &workspace.Conversation{ID: "a"}
	a.eventDecoded(codex.Message{Method: "thread/goal/updated"}, map[string]any{"threadId": "a", "goal": goal})
	v := a.infoViews["a"]
	if v.GoalLoading || !v.GoalAvailable || v.GoalGeneration != 2 || v.Goal == nil {
		t.Fatal("live goal failed to supersede pending load")
	}
	a.eventDecoded(codex.Message{Method: "thread/goal/cleared"}, map[string]any{"threadId": "a"})
	if v.Goal != nil {
		t.Fatal("cleared goal retained")
	}
}

func TestSidebarPagingRetainsFailedCursor(t *testing.T) {
	a := reviewFixture(t)
	a.requestThreads(false, "")
	drain(t, a, func() bool { return !a.threadPage(false).loading })
	if a.historyCursor[false] != "next" || a.state.Chats["recent"] == nil {
		t.Fatal("first page lost")
	}
	a.requestThreads(false, "next")
	drain(t, a, func() bool { return !a.threadPage(false).loading })
	if a.historyCursor[false] != "next" || a.threadPage(false).err == "" {
		t.Fatal("failed page lost retry cursor or error")
	}
	a.requestThreads(false, a.historyCursor[false])
	drain(t, a, func() bool { return !a.threadPage(false).loading })
	if a.state.Chats["older"] == nil || a.historyCursor[false] != "" || a.threadPage(false).err != "" {
		t.Fatal("page retry failed")
	}
}

func TestGoalEmptyStateFromServer(t *testing.T) {
	a := reviewFixture(t)
	v := &conversationInfo{}
	a.loadGoal(&workspace.Conversation{ID: "a"}, v)
	drain(t, a, func() bool { return !v.GoalLoading })
	if !v.GoalAvailable || v.Goal != nil || v.GoalUnsupported {
		t.Fatal("supported empty goal was hidden")
	}
	v = &conversationInfo{}
	a.loadGoal(&workspace.Conversation{ID: "unsupported"}, v)
	drain(t, a, func() bool { return !v.GoalLoading })
	if !v.GoalUnsupported || a.toast != "" {
		t.Fatal("unavailable goals produced controls or a recurring error toast")
	}
}
