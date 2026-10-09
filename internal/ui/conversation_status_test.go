//go:build fltk_headless

package ui

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"image"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestV48ConversationStatusAndAge(t *testing.T) {
	a := transferFixture()
	a.p = colors(false)
	longName := strings.Repeat("Long named conversation ", 10)
	if threadTitle(map[string]any{"name": longName}) != longName {
		t.Fatal("full title lost before tooltip rendering")
	}
	c := &workspace.Conversation{ID: "thread", Status: "idle"}
	if dot := a.conversationDot(c, false); dot.Color.A != 0 {
		t.Fatal("closed idle dot", dot)
	}
	if dot := a.conversationDot(c, true); !dot.Hollow || dot.Color != a.p.BorderStrong {
		t.Fatal("open idle ring", dot)
	}
	c.Unread = true
	if dot := a.conversationDot(c, true); dot.Hollow || dot.Color != a.p.Success {
		t.Fatal("unread", dot)
	}
	c.Status = "running"
	if dot := a.conversationDot(c, true); dot.Color != a.p.Accent {
		t.Fatal("running", dot)
	}
	c.Status = "starting"
	if dot := a.conversationDot(c, true); dot.Color != a.p.Faint {
		t.Fatal("starting", dot)
	}
	a.approvals = []approval{{ThreadID: c.ID}}
	if dot := a.conversationDot(c, true); dot.Color != a.p.Warning {
		t.Fatal("waiting", dot)
	}
	c.Status = "error"
	if dot := a.conversationDot(c, true); dot.Color != a.p.Danger {
		t.Fatal("error", dot)
	}
	a.state.Chats[c.ID] = c
	if dot := a.tabDot(workspace.Tab{Kind: workspace.File, Target: c.ID}); dot.Color.A != 0 {
		t.Fatal("non-chat inherited a chat status", dot)
	}
	// Same-size close/open sequences must invalidate the membership index.
	id := a.state.Open(workspace.Chat, "old", "old", "")
	if !a.chatIsOpen("old") {
		t.Fatal("missing open chat")
	}
	a.state.Close(id)
	a.state.Open(workspace.Chat, "new", "new", "")
	if a.chatIsOpen("old") || !a.chatIsOpen("new") {
		t.Fatal("stale open-chat index")
	}
	now := time.Unix(1800000000, 0)
	for _, tc := range []struct {
		age  time.Duration
		want string
	}{
		{-time.Hour, "now"}, {30 * time.Second, "now"}, {5 * time.Minute, "5m"}, {3 * time.Hour, "3h"},
		{48 * time.Hour, "2d"}, {28 * 24 * time.Hour, "4w"}, {150 * 24 * time.Hour, "5mo"}, {730 * 24 * time.Hour, "2y"},
	} {
		if got := conversationAge(now.Add(-tc.age).Unix(), now); got != tc.want {
			t.Fatalf("age %s: %q", tc.age, got)
		}
	}
	if conversationAge(0, now) != "" {
		t.Fatal("unknown timestamp should be empty")
	}
}

func TestV48SidebarRefreshSchedule(t *testing.T) {
	a := transferFixture()
	a.prefs.Sidebar = true
	a.scheduleTimeUpdate()
	if a.timeRefresh == nil || a.timeRefreshInterval != time.Minute {
		t.Fatal("sidebar timestamps have no minute update")
	}
	a.state.Open(workspace.Chat, "chat", "thread", "")
	a.infoViews = map[string]*conversationInfo{"thread": {}}
	a.prefs.Info = true
	old, generation := a.timeRefresh, a.timeRefreshGeneration
	a.scheduleTimeUpdate()
	if a.timeRefresh == old || a.timeRefreshInterval != 5*time.Second || a.timeRefreshGeneration <= generation {
		t.Fatal("time-sensitive info retained slow sidebar timer")
	}
	a.prefs.Sidebar = false
	a.prefs.Info = false
	a.scheduleTimeUpdate()
	if a.timeRefresh != nil {
		t.Fatal("hidden time-dependent views kept a timer")
	}
}

func TestV48StatusRowGeometry(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			p := colors(false)
			var row image.Rectangle
			var commands []command.Command
			h := desktop.NewHeadlessHarness(desktop.WindowNoScrollbar, image.Pt(int(320*scale), int(130*scale)), func(w *desktop.Window) {
				w.Row(28).Dynamic(1)
				flatStatusRow(w, "Visible conversation", "5mo", true, statusDot{Color: p.BorderStrong, Hollow: true}, 22, p)
				r := w.LastWidgetBounds
				row = image.Rect(r.X, r.Y, r.X+r.W, r.Y+r.H)
				commands = append(commands[:0], w.Commands().Commands...)
			})
			s := makeStyle(p, 13)
			s.Scale(scale)
			h.Master().SetStyle(s)
			h.Frame(true)
			lines, labels := 0, map[string]image.Rectangle{}
			for _, c := range commands {
				if c.Kind == command.LineCmd {
					lines++
				}
				if c.Kind == command.TextCmd {
					labels[c.Text.String] = image.Rect(c.Rect.X, c.Rect.Y, c.Rect.X+c.Rect.W, c.Rect.Y+c.Rect.H)
				}
			}
			if lines != 16 || labels["5mo"].Empty() || labels["Visible conversation"].Empty() {
				t.Fatal("ring/date/title missing", lines, labels)
			}
			if labels["Visible conversation"].Min.X < row.Min.X+int(37*scale) || labels["Visible conversation"].Max.X > labels["5mo"].Min.X || labels["5mo"].Max.X > row.Max.X {
				t.Fatal("row overlaps", row, labels)
			}
		})
	}
}

func TestV48TabOverflowVirtualization(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := transferFixture()
			a.p = colors(false)
			for i := range 10000 {
				id := fmt.Sprint(i)
				a.state.Tabs = append(a.state.Tabs, workspace.Tab{ID: id, Kind: workspace.Chat, Target: id, Title: fmt.Sprintf("Thread %05d", i)})
				a.state.Chats[id] = &workspace.Conversation{ID: id}
			}
			scroll := 0
			h := desktop.NewHeadlessHarness(desktop.WindowNoScrollbar, image.Pt(int(340*scale), int(460*scale)), func(w *desktop.Window) {
				w.Row(400).Dynamic(1)
				if g := w.GroupBegin("overflow-list", desktop.WindowNoHScrollbar); g != nil {
					g.Scrollbar.Y = scroll
					a.drawTabOverflowList(g)
					g.GroupEnd()
				}
			})
			s := makeStyle(a.p, 13)
			s.Scale(scale)
			h.Master().SetStyle(s)
			stride := int(30*scale) + s.GroupWindow.Spacing.Y
			for _, pos := range []int{0, 5000 * stride, 9990 * stride} {
				scroll = pos
				h.Frame(false)
				if n := h.Frame(true); n > 450 {
					t.Fatalf("unbounded overflow: %d commands", n)
				}
				want := fmt.Sprintf("Thread %05d", pos/stride)
				found := false
				for _, c := range h.Commands() {
					if c.Kind == command.TextCmd && c.Text.String == want {
						found = true
					}
				}
				if !found {
					t.Fatal("unreachable overflow row", want)
				}
			}
		})
	}
}

func TestV48ShellRPCFixture(t *testing.T) {
	if len(os.Args) == 0 || os.Args[len(os.Args)-1] != "fastrock-shell-fixture" {
		return
	}
	reader, writer := bufio.NewScanner(os.Stdin), json.NewEncoder(os.Stdout)
	count := 0
	for reader.Scan() {
		var m codex.Message
		if json.Unmarshal(reader.Bytes(), &m) != nil {
			continue
		}
		var result any = map[string]any{}
		var rpcError *codex.RPCError
		if m.Method == "thread/start" {
			count++
			if filepath.Base(str(codex.Decode(m.Params), "cwd")) == "fail" {
				rpcError = &codex.RPCError{Code: -32000, Message: "fixture start failed"}
			} else {
				result = map[string]any{"thread": map[string]any{"id": fmt.Sprint("thread-", count)}}
			}
		}
		if m.Method == "thread/items/list" {
			params := codex.Decode(m.Params)
			if str(params, "threadId") == "fail-history" {
				rpcError = &codex.RPCError{Code: -32000, Message: "fixture history failed"}
			} else if str(params, "cursor") == "older" {
				result = map[string]any{"data": []any{map[string]any{"item": map[string]any{"id": "b", "type": "agentMessage", "text": "second page"}}}}
			} else {
				result = map[string]any{"nextCursor": "older", "data": []any{map[string]any{"item": map[string]any{"id": "a", "type": "agentMessage", "text": "first page"}}}}
			}
		}
		body, _ := json.Marshal(result)
		_ = writer.Encode(codex.Message{ID: m.ID, Result: body, Error: rpcError})
	}
	os.Exit(0)
}

func shellFixture(t *testing.T) *App {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	t.Cleanup(cancel)
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client, err := codex.StartCommand(ctx, exe, []string{"-test.run=^TestV48ShellRPCFixture$", "--", "fastrock-shell-fixture"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	a := transferFixture()
	a.ctx = ctx
	a.client = client
	a.updates = make(chan func(), 32)
	return a
}

func TestV48NewThreadLifecycle(t *testing.T) {
	a := shellFixture(t)
	start := a.state.OpenNew()
	a.newThread("")
	a.newThread("")
	other := a.state.Open(workspace.File, "other", "other", "")
	drain(t, a, func() bool { return !a.newThreadPending })
	if len(a.state.Tabs) != 2 || a.state.Tabs[0].ID != start || a.state.Tabs[0].Target != "thread-1" || a.state.Active != other {
		t.Fatal("duplicate start or lost origin/focus", a.state.Tabs)
	}
	start = a.state.OpenNew()
	a.newThread(filepath.Join(t.TempDir(), "fail"))
	drain(t, a, func() bool { return !a.newThreadPending })
	if a.state.Current().ID != start || a.state.Current().Kind != workspace.New {
		t.Fatal("failure consumed start page")
	}
	a.newThread("")
	drain(t, a, func() bool { return !a.newThreadPending })
	if a.state.Current().ID != start || a.state.Current().Target != "thread-3" {
		t.Fatal("retry failed", a.state.Current())
	}
}
