package ui

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type backgroundTerminal struct {
	ItemID     string   `json:"itemId"`
	ProcessID  string   `json:"processId"`
	Command    string   `json:"command"`
	Cwd        string   `json:"cwd"`
	OSPID      *uint32  `json:"osPid"`
	CPUPercent *float64 `json:"cpuPercent"`
	RSSKB      *uint64  `json:"rssKb"`
}

func terminalList(ctx context.Context, client *codex.Client, id string) ([]backgroundTerminal, error) {
	var rows []backgroundTerminal
	cursor := ""
	seen := map[string]bool{}
	for pageIndex := 0; pageIndex < 100; pageIndex++ {
		var page struct {
			Data []backgroundTerminal
			Next string `json:"nextCursor"`
		}
		params := map[string]any{"threadId": id, "limit": 100}
		if cursor != "" {
			params["cursor"] = cursor
		}
		if err := client.Call(ctx, "thread/backgroundTerminals/list", params, &page); err != nil {
			return nil, err
		}
		if len(rows)+len(page.Data) > 1000 {
			return nil, fmt.Errorf("Too many background terminals to display. Stop commands and refresh")
		}
		rows = append(rows, page.Data...)
		if page.Next == "" {
			return rows, nil
		}
		if seen[page.Next] {
			return nil, fmt.Errorf("Codex repeated a terminal page. Refresh to try again")
		}
		seen[page.Next] = true
		cursor = page.Next
	}
	return nil, fmt.Errorf("Background terminal list did not finish. Refresh to try again")
}
func (a *App) loadTerminals(c *workspace.Conversation, v *conversationInfo) {
	if a.client == nil || a.serverPaused {
		v.TerminalNote = "Background terminals are unavailable while Codex is disconnected."
		return
	}
	if v.TerminalsLoading {
		v.TerminalsDirty = true
		return
	}
	v.TerminalsLoading = true
	v.TerminalsDirty = false
	v.TerminalNote = ""
	v.TerminalUpdated = time.Now()
	v.TerminalGeneration++
	generation, client, id := v.TerminalGeneration, a.client, c.ID
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		rows, err := terminalList(ctx, client, id)
		a.post(func() {
			if v.TerminalGeneration != generation || a.client != client || a.infoViews[id] != v {
				return
			}
			v.TerminalsLoading = false
			if err != nil {
				v.TerminalNote = "Background terminals unavailable: " + err.Error()
			} else {
				v.TerminalsAvailable = true
				v.Terminals = rows
				for id := range v.Stopping {
					found := false
					for _, row := range rows {
						found = found || row.ProcessID == id
					}
					if !found {
						delete(v.Stopping, id)
					}
				}
			}
			// A command may have appeared after the request snapshot was taken.
			if v.TerminalsDirty {
				a.loadTerminals(c, v)
			}
		})
	}, func() { v.TerminalsLoading = false; v.TerminalNote = errWorkQueueFull.Error() })
}
func terminalEvent(method string, p map[string]any) bool {
	if method == "item/commandExecution/terminalInteraction" {
		return true
	}
	if method != "item/started" && method != "item/completed" {
		return false
	}
	item, _ := p["item"].(map[string]any)
	return str(item, "type") == "commandExecution" && str(item, "processId") != ""
}
func (a *App) infoEvent(c *workspace.Conversation, method string, p map[string]any) {
	if method == "thread/settings/updated" {
		if current, ok := p["threadSettings"].(map[string]any); ok {
			c.Settings = threadSettings(current)
			c.Model, c.Effort, c.Tier = str(current, "model"), str(current, "effort"), str(current, "serviceTier")
		}
	}
	if terminalEvent(method, p) {
		if v := a.infoViews[c.ID]; v != nil {
			a.loadTerminals(c, v)
		}
	}
}
func (a *App) stopTerminal(c *workspace.Conversation, v *conversationInfo, id string) {
	if a.client == nil || v.StoppingAll || v.Stopping[id] {
		return
	}
	if v.Stopping == nil {
		v.Stopping = map[string]bool{}
	}
	v.Stopping[id] = true
	client, epoch := a.client, v.TerminalEpoch
	a.controlWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		err := client.Call(ctx, "thread/backgroundTerminals/terminate", map[string]any{"threadId": c.ID, "processId": id}, nil)
		a.post(func() {
			if a.client != client || v.TerminalEpoch != epoch || a.infoViews[c.ID] != v {
				return
			}
			delete(v.Stopping, id)
			if err != nil {
				v.TerminalNote = "Could not stop command: " + err.Error()
				return
			}
			a.loadTerminals(c, v)
		})
	}, func() {
		if v.TerminalEpoch == epoch {
			delete(v.Stopping, id)
			v.TerminalNote = errWorkQueueFull.Error()
		}
	})
}
func (a *App) cleanTerminals(c *workspace.Conversation, v *conversationInfo) {
	if a.client == nil || v.StoppingAll {
		return
	}
	v.StoppingAll = true
	client, epoch := a.client, v.TerminalEpoch
	a.controlWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		err := client.Call(ctx, "thread/backgroundTerminals/clean", map[string]any{"threadId": c.ID}, nil)
		a.post(func() {
			if a.client != client || v.TerminalEpoch != epoch || a.infoViews[c.ID] != v {
				return
			}
			v.StoppingAll = false
			if err != nil {
				v.TerminalNote = "Could not stop background commands: " + err.Error()
				return
			}
			a.loadTerminals(c, v)
		})
	}, func() { v.StoppingAll = false; v.TerminalNote = errWorkQueueFull.Error() })
}
func terminalLocation(cwd, project, home string) string {
	if cwd == "" {
		return ""
	}
	for _, base := range []struct{ path, prefix string }{{project, "."}, {home, "~"}} {
		if base.path == "" {
			continue
		}
		if relative, err := filepath.Rel(base.path, cwd); err == nil && relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
			if relative == "." {
				return base.prefix
			}
			return base.prefix + string(filepath.Separator) + relative
		}
	}
	return cwd
}
func terminalDetail(t backgroundTerminal) string {
	var parts []string
	if t.OSPID != nil {
		parts = append(parts, fmt.Sprintf("PID %d", *t.OSPID))
	}
	if t.CPUPercent != nil {
		parts = append(parts, fmt.Sprintf("CPU %.1f%%", *t.CPUPercent))
	}
	if t.RSSKB != nil {
		parts = append(parts, fmt.Sprintf("Memory %.1f MiB", float64(*t.RSSKB)/1024))
	}
	return strings.Join(parts, " · ")
}
func terminalStopMessage(rows []backgroundTerminal) string {
	var b strings.Builder
	b.WriteString("Stop every background command in this conversation?\n")
	for i, row := range rows {
		if i == 8 {
			fmt.Fprintf(&b, "\n…and %d more", len(rows)-i)
			break
		}
		b.WriteString("\n• " + cut(strings.SplitN(strings.TrimSpace(row.Command), "\n", 2)[0], 100))
	}
	return b.String()
}
func (a *App) drawTerminals(w *desktop.Window, c *workspace.Conversation, v *conversationInfo) {
	if len(v.Terminals) > 0 && time.Since(v.TerminalUpdated) > 5*time.Second && !v.TerminalsLoading {
		a.loadTerminals(c, v)
	}
	if v.TerminalsAvailable && len(v.Terminals) == 0 && !v.TerminalsLoading && v.TerminalNote == "" {
		return
	}
	summary := fmt.Sprintf("%d running", len(v.Terminals))
	if v.TerminalsLoading {
		summary = "Loading…"
	} else if v.TerminalNote != "" {
		summary = "Unavailable"
	}
	if !a.infoSection(w, "terminals", "Background terminals", summary) {
		return
	}
	if v.TerminalNote != "" {
		a.drawSettingsError(w, v.TerminalNote)
	}
	home, _ := os.UserHomeDir()
	for _, terminal := range v.Terminals {
		w.Row(28).Dynamic(1)
		w.LabelColored("● "+cut(strings.SplitN(terminal.Command, "\n", 2)[0], 100), "LC", a.p.Text)
		muted(w, terminalLocation(terminal.Cwd, c.Cwd, home), a.p)
		if detail := terminalDetail(terminal); detail != "" {
			muted(w, detail, a.p)
		}
		w.Row(27).Dynamic(3)
		if w.ButtonText("Copy command") {
			a.copyText(terminal.Command)
		}
		if w.ButtonText("Open folder") && terminal.Cwd != "" {
			a.openPath(terminal.Cwd, false)
		}
		if v.Stopping[terminal.ProcessID] || v.StoppingAll {
			w.Label("Stopping…", "LC")
		} else if dangerButton(w, "Stop", a.p) {
			a.stopTerminal(c, v, terminal.ProcessID)
		}
	}
	w.Row(27).Dynamic(2)
	if v.TerminalsLoading {
		w.Label("Loading…", "LC")
	} else if w.ButtonText("Refresh") {
		a.loadTerminals(c, v)
	}
	if len(v.Terminals) > 0 {
		if v.StoppingAll {
			w.Label("Stopping all…", "LC")
		} else if dangerButton(w, "Stop all…", a.p) {
			a.confirm("Stop background commands?", terminalStopMessage(v.Terminals), func() { a.cleanTerminals(c, v) })
		}
	}
}
