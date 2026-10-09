//go:build nucular_headless

package ui

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

type historyTestCaller func(context.Context, string, any, any) error

func (f historyTestCaller) Call(ctx context.Context, method string, params, out any) error {
	return f(ctx, method, params, out)
}

func TestV49HistoryViewLifecycle(t *testing.T) {
	a := shellFixture(t)
	c := &workspace.Conversation{ID: "closed-thread", Title: "Unopened conversation"}
	a.state.Chats[c.ID] = c
	a.chatAction(c.ID, "view-text")
	v := a.files[a.state.Active]
	drain(t, a, func() bool { return !v.Loading })
	if !v.Loaded || v.Error != "" || !strings.Contains(v.fileText(), "first page") || !strings.Contains(v.fileText(), "second page") || len(a.chats) != 0 {
		t.Fatal("unopened history not loaded", v.Error, v.fileText())
	}
	if v.Editor.Flags&nucular.EditReadOnly == 0 {
		t.Fatal("history editor is writable")
	}
	failed := &workspace.Conversation{ID: "fail-history", Title: "Failed history"}
	a.viewChatText(failed)
	v = a.files[a.state.Active]
	drain(t, a, func() bool { return !v.Loading })
	if v.Editor != nil || !strings.Contains(v.Error, "fixture history failed") {
		t.Fatal("history error hidden", v.Error)
	}
	// A closed view cancels the request and rejects its queued result.
	a.viewChatText(c)
	id := a.state.Active
	closed := a.files[id]
	a.closeTabNow(id)
	select {
	case f := <-a.updates:
		f()
	case <-time.After(3 * time.Second):
		t.Fatal("closed history completion did not arrive")
	}
	if !closed.Closed || closed.ReadCancel != nil || a.files[id] != nil {
		t.Fatal("closed history retained a live result")
	}
	// A new connection must not publish history from the old server.
	a.viewChatText(c)
	v = a.files[a.state.Active]
	a.client = nil
	drain(t, a, func() bool { return !v.Loading })
	if v.Editor != nil || !strings.Contains(v.Error, "reconnected") {
		t.Fatal("old server result published", v.Error)
	}
}

func TestV49HistoryTextPagesAndLimits(t *testing.T) {
	calls := 0
	client := historyTestCaller(func(ctx context.Context, method string, params, out any) error {
		if method != "thread/items/list" {
			t.Fatal(method)
		}
		p := params.(map[string]any)
		if p["threadId"] != "closed-thread" || p["sortDirection"] != "asc" {
			t.Fatal(p)
		}
		calls++
		body := `{"data":[{"item":{"id":"a","type":"agentMessage","text":"first page"}}],"nextCursor":"next"}`
		if calls == 2 {
			if p["cursor"] != "next" {
				t.Fatal(p)
			}
			body = `{"data":[{"item":{"id":"b","type":"agentMessage","text":"second page"}}]}`
		}
		return json.Unmarshal([]byte(body), out)
	})
	var out strings.Builder
	if err := writeChatHistory(context.Background(), client, "closed-thread", "Title", &out, 4096); err != nil {
		t.Fatal(err)
	}
	value := out.String()
	if calls != 2 || !strings.Contains(value, "Title") || !strings.Contains(value, "first page") || strings.Index(value, "second page") < strings.Index(value, "first page") {
		t.Fatal(calls, value)
	}
	client = historyTestCaller(func(_ context.Context, _ string, _ any, out any) error {
		return json.Unmarshal([]byte(`{"data":[{"item":{"type":"agentMessage","text":"🪨🪨🪨🪨🪨"}}],"nextCursor":"loop"}`), out)
	})
	out.Reset()
	err := writeChatHistory(context.Background(), client, "closed-thread", "Title", &out, 52)
	if !errors.Is(err, errHistoryLimit) || out.Len() > 52 || !utf8.ValidString(out.String()) {
		t.Fatal("unsafe text limit", out.Len(), err, out.String())
	}
	out.Reset()
	if err := writeChatHistory(context.Background(), client, "closed-thread", "Title", &out, 4096); err == nil || !strings.Contains(err.Error(), "cursor") {
		t.Fatal("cursor loop accepted", err)
	}
	client = historyTestCaller(func(context.Context, string, any, any) error { return io.ErrUnexpectedEOF })
	out.Reset()
	if err := writeChatHistory(context.Background(), client, "closed-thread", "Title", &out, 4096); !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatal("page error lost", err)
	}
}

func TestV49RallyCredentialErrors(t *testing.T) {
	for _, status := range []int{401, 403, 500} {
		t.Run(fmt.Sprint(status), func(t *testing.T) {
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(status)
				fmt.Fprint(w, `{"QueryResult":{"Errors":["fixture denied"]}}`)
			}))
			defer s.Close()
			c, err := rally.New(s.URL, "fixture-token", nil)
			if err != nil {
				t.Fatal(err)
			}
			a := transferFixture()
			a.ctx = context.Background()
			a.updates = make(chan func(), 16)
			a.rallyClient = c
			v := newRallyView(rally.FindPage("teamboard"))
			a.requestRallyPage(v, 1, true)
			drain(t, a, func() bool { return !v.Loading })
			if !strings.Contains(v.Error, "fixture denied") || v.CredentialError != (status == 401 || status == 403) {
				t.Fatal(v.Error, v.CredentialError)
			}
			if v.CredentialError && !strings.Contains(v.Error, "Update your API token in Settings") {
				t.Fatal("recovery hint missing", v.Error)
			}
		})
	}
	if rallyCredentialError(errors.New("401 Unauthorized")) {
		t.Fatal("string-matched a non-API error")
	}
}

func TestV49DisabledRallyControls(t *testing.T) {
	a := transferFixture()
	a.p = colors(false)
	a.navigationFace = makeStyle(a.p, 16).Font
	v := newRallyView(rally.FindPage("teamboard"))
	var commands []command.Command
	var target image.Point
	click := false
	h := nucular.NewHeadlessHarness(nucular.WindowNoScrollbar, image.Pt(900, 500), func(w *nucular.Window) {
		if click {
			m := &w.Input().Mouse
			m.Pos = target
			m.Buttons[mouse.ButtonLeft].Down = false
			m.Buttons[mouse.ButtonLeft].Clicked = true
			m.Buttons[mouse.ButtonLeft].ClickedPos = target
		}
		a.rallyNav(w, v)
		commands = append(commands[:0], w.Commands().Commands...)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	a.window = h.Master()
	h.Frame(false)
	for _, label := range []string{"Refresh", "Ask AI"} {
		found := false
		for _, c := range commands {
			if c.Kind == command.TextCmd && c.Text.String == label {
				found = true
				target = image.Pt(c.Rect.X+3, c.Rect.Y+3)
				if c.Text.Foreground != a.p.Faint {
					t.Fatal("disconnected control not visually disabled", label)
				}
			}
		}
		if !found {
			t.Fatal("missing disabled control", label)
		}
		click = true
		h.Frame(false)
		click = false
		if v.Loading || v.Generation != 0 || a.assistant != nil {
			t.Fatal("disabled action ran", label)
		}
	}
	a.openAssistant(v)
	if a.assistant != nil {
		t.Fatal("direct disconnected assistant action ran")
	}
	// Authentication recovery targets the Rally settings page directly.
	a.openRallySettings()
	if a.state.Current().Kind != workspace.Settings || a.settingsView.Page != "Rally" {
		t.Fatal("wrong recovery destination")
	}
}
