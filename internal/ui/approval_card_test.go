//go:build fltk_headless

package ui

import (
	"encoding/json"
	"fmt"
	"image"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

func TestV41CommandAndPermissionPresentation(t *testing.T) {
	for _, tc := range []struct{ in, want string }{
		{`/bin/bash -lc 'echo hi && pwd'`, "echo hi && pwd"},
		{`pwsh.exe -NoProfile -Command 'Get-Date'`, "Get-Date"},
		{`pwsh.exe -Command Write-Output hello`, `pwsh.exe -Command Write-Output hello`},
		{`git status --short`, "git status --short"},
		{`C:\Tools\run.exe C:\work\file.txt`, `C:\Tools\run.exe C:\work\file.txt`},
		{`bash -lc 'unterminated`, `bash -lc 'unterminated`},
	} {
		if got := approvalCommand(tc.in); got != tc.want {
			t.Fatal(tc, got)
		}
	}
	a := transferFixture()
	r := approval{Message: codex.Message{Method: "item/commandExecution/requestApproval"}, Params: codex.Decode([]byte(`{"command":"bash -lc 'echo hi'","cwd":"/work","environmentId":"remote","additionalPermissions":{"network":{"enabled":true},"fileSystem":{"write":["/work/output"]}}}`))}
	a.prepareApproval(&r)
	if r.Title != "Run this command with additional permissions?" || r.Content.Code != "$ echo hi" || r.Content.Caption != "in /work" || strings.Join(r.Content.Details, "|") != "Permission rule: network; write /work/output|Environment: remote" {
		t.Fatal(r)
	}
	r.Params["networkApprovalContext"] = map[string]any{"host": "example.test", "protocol": "https"}
	a.prepareApproval(&r)
	if r.Title != "Allow network access to example.test?" || r.Content.Code != "https://example.test" || r.Content.Caption != "" {
		t.Fatal(r)
	}
	delete(r.Params, "networkApprovalContext")
	delete(r.Params, "additionalPermissions")
	r.Params["kind"], r.Params["command"] = "writeStdin", `write_stdin 'line\n'`
	a.prepareApproval(&r)
	if r.Title != "Send input to the running command?" || !strings.HasPrefix(r.Content.Code, `"line`) {
		t.Fatal(r)
	}
	permissions := codex.Decode([]byte(`{"network":{"enabled":true},"fileSystem":{"write":["ignored"],"entries":[{"access":"read","path":{"type":"path","path":"/read"}},{"access":"write","path":{"type":"glob_pattern","pattern":"/work/*.go"}},{"access":"deny","path":{"type":"special","value":{"kind":"project_roots","subpath":"private"}}}]}}`))
	if got := permissionRule(permissions); got != "network; read /read; write glob /work/*.go; deny read :workspace_roots/private" {
		t.Fatal(got)
	}
	r = approval{Message: codex.Message{Method: "item/permissions/requestApproval"}, Params: map[string]any{"permissions": permissions, "cwd": "/work", "environmentId": "local"}}
	a.prepareApproval(&r)
	if r.Title != "Grant additional permissions?" || len(r.Content.Details) != 1 || r.Content.Caption != "in /work" {
		t.Fatal(r)
	}
}

func TestV41ApprovalChoicesDescribeSavedRules(t *testing.T) {
	p := codex.Decode([]byte(`{"networkApprovalContext":{"host":"example.test"},"proposedExecpolicyAmendment":["curl"],"proposedNetworkPolicyAmendments":[{"host":"example.test","action":"allow"}]}`))
	choices := approvalChoices("item/commandExecution/requestApproval", p)
	if len(choices) != 4 || choices[0].Title != "Yes, just this once" || choices[1].Title != "Yes, and allow this host for this conversation" || choices[2].Key != 'h' {
		t.Fatal(choices)
	}
	for _, c := range choices {
		if strings.Contains(c.Title, "commands that start") {
			t.Fatal("network approval offered command rule", choices)
		}
	}
	p = codex.Decode([]byte(`{"proposedExecpolicyAmendment":["git","status"]}`))
	before, _ := json.Marshal(p)
	choices = approvalChoices("item/commandExecution/requestApproval", p)
	if len(choices) != 3 || choices[1].Title != "Yes, and don't ask again for commands that start with `git status`" {
		t.Fatal(choices)
	}
	after, _ := json.Marshal(p)
	if string(before) != string(after) {
		t.Fatal("display rewrote request")
	}
	p["proposedExecpolicyAmendment"] = []any{"echo", "first\nsecond"}
	if got := approvalChoices("item/commandExecution/requestApproval", p); len(got) != 2 {
		t.Fatal("multiline prefix offered", got)
	}
	p = map[string]any{"additionalPermissions": map[string]any{}}
	if got := approvalChoices("item/commandExecution/requestApproval", p); len(got) != 2 {
		t.Fatal(got)
	}
	p["availableDecisions"] = []any{"acceptForSession", "cancel"}
	if got := approvalChoices("item/commandExecution/requestApproval", p); len(got) != 2 || got[0].Key != 'a' {
		t.Fatal("ignored advertised choices", got)
	}
	p["availableDecisions"] = []any{}
	if got := approvalChoices("item/commandExecution/requestApproval", p); len(got) != 0 {
		t.Fatal("invented decisions", got)
	}
	file := approvalChoices("item/fileChange/requestApproval", nil)
	if len(file) != 4 || file[1].Title != "Yes, and don't ask again for these files" || file[2].Title != "No, continue without these changes" {
		t.Fatal(file)
	}
}

func TestV41PatchPreviewUpdatesAndBounds(t *testing.T) {
	a := infoFixture(t)
	c := &workspace.Conversation{ID: "parent", Cwd: "/work"}
	a.state.Chats[c.ID] = c
	a.state.Open(workspace.Chat, "Parent", c.ID, "")
	patch := func(text string) []any {
		return []any{map[string]any{"path": "/work/a.go", "kind": map[string]any{"type": "update"}, "diff": text}}
	}
	a.eventDecoded(codex.Message{Method: "item/started"}, map[string]any{"threadId": c.ID, "item": map[string]any{"id": "change", "type": "fileChange", "changes": patch("@@ -1 +1 @@\n-old\n+new\n")}})
	p := map[string]any{"threadId": c.ID, "itemId": "change"}
	a.serverRequest(codex.Message{Method: "item/fileChange/requestApproval", ID: json.RawMessage(`"approval"`), Origin: a.client}, p)
	r := &a.approvals[0]
	if r.Content.Diff[0].Text != "Edited a.go (+1 −1)" || r.Content.Diff[2].Kind != "remove" || r.Content.Diff[3].Kind != "add" {
		t.Fatal(r.Content)
	}
	r.Armed = time.Time{}
	a.eventDecoded(codex.Message{Method: "item/fileChange/patchUpdated"}, map[string]any{"threadId": c.ID, "itemId": "change", "changes": patch("@@ -1 +1 @@\n-old\n+revised\n")})
	if r.Content.Diff[3].Text != "+revised" || r.Armed.Before(time.Now()) {
		t.Fatal("patch did not update/arm", r.Content)
	}
	lines := approvalDiff([]any{map[string]any{"path": "/work/b.go", "kind": map[string]any{"type": "add"}, "diff": strings.Repeat("x\n", 1000)}}, "/work")
	if len(lines) != 401 || !strings.Contains(lines[0].Text, "+1000") || lines[400].Kind != "note" {
		t.Fatal(len(lines), lines[0], lines[len(lines)-1])
	}
	for i := 0; i < 80; i++ {
		a.cacheApprovalDiff(c.ID, fmt.Sprint(i), patch("+x"))
	}
	if len(a.approvalDiffs) != 64 {
		t.Fatal(len(a.approvalDiffs))
	}
	empty := approvalDiff([]any{map[string]any{"path": "x", "kind": map[string]any{"type": "add"}, "diff": ""}}, "")
	if len(empty) != 1 || empty[0].Text != "Added x (+0 −0)" {
		t.Fatal(empty)
	}
	patchLines := approvalDiff(patch("diff --git a/a.go b/a.go\n--- a/a.go\n+++ b/a.go\n@@ -1,2 +1,2 @@\n--- comment\n-old\n+++i;\n+new\n"), "/work")
	if len(patchLines) != 6 || patchLines[0].Text != "Edited a.go (+2 −2)" || patchLines[2].Kind != "remove" || patchLines[4].Kind != "add" {
		t.Fatal("mistook source lines for diff headers", patchLines)
	}
	moved := approvalDiff([]any{map[string]any{"path": "/work/a.go", "kind": map[string]any{"type": "update", "movePath": "/work/b.go"}, "diff": "@@ -1 +1 @@\n-old\n+new\n\nMoved to: /work/b.go"}}, "/work")
	if len(moved) != 4 || moved[0].Text != "Moved a.go → b.go (+1 −1)" {
		t.Fatal("rename trailer leaked into preview", moved)
	}
}

func TestV41EmptyDecisionsOfferNoFallback(t *testing.T) {
	a := infoFixture(t)
	a.p = colors(false)
	a.state.Chats["c"] = &workspace.Conversation{ID: "c"}
	a.state.Open(workspace.Chat, "C", "c", "")
	a.serverRequest(codex.Message{Method: "item/commandExecution/requestApproval", ID: json.RawMessage(`"empty"`), Origin: a.client}, map[string]any{"threadId": "c", "command": "echo test", "availableDecisions": []any{}})
	var labels []string
	h := desktop.NewHeadlessHarness(0, image.Pt(500, 700), func(w *desktop.Window) {
		a.drawApprovalFor(w, "c")
		for _, c := range w.Commands().Commands {
			if c.Kind == command.TextCmd {
				labels = append(labels, c.Text.String)
			}
		}
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	h.Frame(false)
	text := strings.Join(labels, " ")
	if !strings.Contains(text, "did not offer") || strings.Contains(text, "Allow / Submit") || strings.Contains(text, "Decline") {
		t.Fatal("invented approval choices", text)
	}
	a.submitApproval()
	if a.approvals[0].Submitting {
		t.Fatal("sent an unadvertised decision")
	}
}

func TestV41MissingPatchFetchAndStaleResults(t *testing.T) {
	for _, stale := range []bool{false, true} {
		t.Run(fmt.Sprint(stale), func(t *testing.T) {
			a := infoFixture(t)
			c := &workspace.Conversation{ID: "parent", Cwd: "/work"}
			a.state.Chats[c.ID] = c
			a.state.Open(workspace.Chat, "Parent", c.ID, "")
			p := map[string]any{"threadId": c.ID, "turnId": "turn", "itemId": "patch"}
			a.serverRequest(codex.Message{Method: "item/fileChange/requestApproval", ID: json.RawMessage(`"approval"`), Origin: a.client}, p)
			if stale {
				a.serverGeneration++
			}
			select {
			case apply := <-a.updates:
				apply()
			case <-time.After(3 * time.Second):
				t.Fatal("missing patch result")
			}
			if got := len(a.approvals[0].Content.Diff) > 0; got == stale {
				t.Fatal("wrong stale-result handling", a.approvals[0].Content)
			}
		})
	}
}

func TestV41ApprovalCardGeometryAndLiteralCode(t *testing.T) {
	for _, width := range []int{480, 700, 1000} {
		for _, scale := range []float64{1, 1.5, 2} {
			t.Run(fmt.Sprintf("%d/%g", width, scale), func(t *testing.T) {
				a := transferFixture()
				a.p = colors(false)
				a.state.Chats["c"] = &workspace.Conversation{ID: "c"}
				a.state.Open(workspace.Chat, "C", "c", "")
				r := approval{ThreadID: "c", OriginThreadID: "c", Focused: true, Message: codex.Message{Method: "item/commandExecution/requestApproval"}, Params: map[string]any{"command": `bash -lc 'echo "# literal *text*"'`, "reason": "This reason explains the requested command and wraps across several lines at narrow widths.", "cwd": "/work", "proposedExecpolicyAmendment": map[string]any{"command": []any{"echo", "long argument with spaces"}}}}
				a.prepareApproval(&r)
				r.Choices = approvalChoices(r.Message.Method, r.Params)
				a.approvals = []approval{r}
				var commands []command.Command
				reserved := 0
				h := desktop.NewHeadlessHarness(0, image.Pt(width, 1600), func(w *desktop.Window) {
					reserved = a.approvalHeight(w, "c")
					a.drawApprovalFor(w, "c")
					commands = append([]command.Command(nil), w.Commands().Commands...)
				})
				h.Master().SetStyle(makeStyle(a.p, 13))
				h.Master().Style().Scale(scale)
				a.window = h.Master()
				h.Frame(false)
				var labels []string
				lastY := 0
				for _, cmd := range commands {
					if cmd.Kind == command.TextCmd {
						labels = append(labels, cmd.Text.String)
						lastY = max(lastY, cmd.Y+cmd.H)
					}
				}
				joined := strings.Join(labels, "\n")
				for _, want := range []string{"Run this command?", "# literal *text*", "Copy code", "View full request details", "Enter confirms"} {
					if !strings.Contains(joined, want) {
						t.Fatal("missing", want, joined)
					}
				}
				if lastY > reserved+4 {
					t.Fatal("card exceeds reserved height", lastY, reserved)
				}
				editor := a.approvals[0].CodeEditor
				if editor == nil || editor.Flags&desktop.EditReadOnly == 0 || text(editor) != `$ echo "# literal *text*"` {
					t.Fatal("code is not a selectable literal", editor)
				}
				if !reflect.DeepEqual(r.Params, a.approvals[0].Params) {
					t.Fatal("display changed request")
				}
			})
		}
	}
}

func TestV41CodeSelectionDoesNotAnswerApproval(t *testing.T) {
	a := infoFixture(t)
	a.state.Chats["c"] = &workspace.Conversation{ID: "c"}
	a.state.Open(workspace.Chat, "C", "c", "")
	a.approvals = []approval{{ThreadID: "c", Focused: true, CodeEditor: textEditor("code", true), Choices: approvalChoices("item/commandExecution/requestApproval", nil)}}
	a.approvals[0].CodeEditor.Active = true
	h := desktop.NewHeadlessHarness(0, image.Pt(500, 400), func(w *desktop.Window) {
		for event := range w.Input().Keyboard.Events() {
			a.approvalKey(event)
		}
	})
	h.Key(key.CodeY, 0)
	h.Frame(false)
	if a.approvals[0].Submitting {
		t.Fatal("selection typing accepted approval")
	}
}

func TestV41ApprovalFitsChatControlsAtDisplayScales(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := transferFixture()
			a.p = colors(false)
			a.prefs.FontSize = 13
			c := &workspace.Conversation{ID: "c", Title: "Chat", Cwd: "/work"}
			a.state.Chats[c.ID] = c
			a.state.Open(workspace.Chat, c.Title, c.ID, "")
			a.chats[c.ID] = &chatView{Editor: textEditor("", true), Follow: true}
			r := approval{ThreadID: c.ID, OriginThreadID: c.ID, Message: codex.Message{Method: "item/fileChange/requestApproval"}, Params: map[string]any{}}
			a.prepareApproval(&r)
			r.Content.Reason = strings.Repeat("A long explanation for this change. ", 100)
			r.Choices = approvalChoices(r.Message.Method, r.Params)
			a.approvals = []approval{r}
			var labels []command.Command
			size := image.Pt(int(960*scale), int(850*scale))
			h := desktop.NewHeadlessHarness(0, size, func(w *desktop.Window) {
				a.drawChat(w, c.ID)
				labels = append([]command.Command(nil), w.Commands().Commands...)
			})
			h.Master().SetStyle(makeStyle(a.p, 13))
			h.Master().Style().Scale(scale)
			a.window = h.Master()
			h.Frame(false)
			var text strings.Builder
			for _, cmd := range labels {
				if cmd.Kind == command.TextCmd {
					text.WriteString(cmd.Text.String + " ")
					if cmd.Y+cmd.H > size.Y {
						t.Fatal("text outside chat viewport", cmd.Text.String, cmd.Y, cmd.H, size)
					}
				}
			}
			for _, want := range []string{"Yes, proceed", "No, continue", "Enter confirms", "Send ↑"} {
				if !strings.Contains(text.String(), want) {
					for _, cmd := range labels {
						if cmd.Kind == command.TextCmd && (strings.Contains(cmd.Text.String, "Enter confirms") || strings.Contains(cmd.Text.String, "Configured") || cmd.Text.String == "Plan") {
							t.Log(cmd.Text.String, cmd.Y, cmd.H)
						}
					}
					t.Fatal("chat clipped control", want, "limit", a.approvals[0].HeightLimit, "size", size)
				}
			}
		})
	}
}
