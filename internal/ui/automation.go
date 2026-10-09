package ui

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/platform"
)

type automationStep struct {
	Action       string `json:"action"`
	Value        string `json:"value"`
	Path         string `json:"path"`
	Milliseconds int    `json:"milliseconds"`
}

// Automation only runs on Windows, with the same actions as the visible UI.
func (a *App) startAutomation() {
	path := os.Getenv("FASTROCK_AUTOMATION")
	if path == "" {
		return
	}
	if runtime.GOOS != "windows" {
		a.toast = "GUI automation is only supported on Windows"
		return
	}
	a.work(func() {
		b, e := os.ReadFile(path)
		if e != nil {
			a.post(func() { a.report(e) })
			return
		}
		var steps []automationStep
		if e = json.Unmarshal(b, &steps); e != nil {
			a.post(func() { a.report(e) })
			return
		}
		log := ""
		for _, step := range steps {
			if step.Action == "wait" {
				timer := time.NewTimer(time.Duration(step.Milliseconds) * time.Millisecond)
				select {
				case <-timer.C:
				case <-a.ctx.Done():
					timer.Stop()
					return
				}
				continue
			}
			if step.Action == "snapshot" {
				e := platform.Screenshot(step.Path)
				if e != nil {
					log += e.Error() + "\n"
				}
				continue
			}
			done := make(chan struct{})
			a.post(func() {
				switch step.Action {
				case "rally":
					a.openRally(step.Value)
				case "theme":
					if step.Value != "dark" && step.Value != "light" {
						log += "FAIL: invalid theme\n"
						break
					}
					a.prefs.Theme = step.Value
					a.theme()
				case "settings":
					a.openSettings()
					a.settingsView.Page = step.Value
					a.loadSettingsPage(step.Value)
				case "item":
					if tab := a.state.Current(); tab != nil {
						if v := a.rallyViews[tab.ID]; v != nil {
							for _, o := range v.Items {
								if o.ID() == step.Value {
									a.openArtifact(v, o)
									break
								}
							}
						}
					}
				case "mode":
					if tab := a.state.Current(); tab != nil {
						if v := a.rallyViews[tab.ID]; v != nil {
							v.Mode = step.Value
						}
					}
				case "new_chat":
					a.newThread(step.Value)
				case "send":
					if tab := a.state.Current(); tab != nil {
						if v := a.chats[tab.Target]; v != nil {
							setText(v.Editor, step.Value)
							a.send(a.state.Chats[tab.Target], "send")
						}
					}
				case "assistant":
					if tab := a.state.Current(); tab != nil {
						if v := a.rallyViews[tab.ID]; v != nil {
							a.openAssistant(v)
						}
					}
				case "ask":
					if a.assistant != nil {
						setText(a.assistant.Editor, step.Value)
						a.sendAssistant()
					}
				case "assert_chat":
					found := false
					if tab := a.state.Current(); tab != nil {
						if c := a.state.Chats[tab.Target]; c != nil {
							for _, b := range c.Blocks {
								if b.Role == "assistant" && strings.Contains(b.Text, step.Value) {
									found = true
								}
							}
						}
					}
					if !found {
						log += "FAIL: missing expected assistant message\n"
						a.exitCode = 1
					}
				case "assert_assistant":
					if a.assistant == nil || a.assistant.Busy || !strings.Contains(a.assistant.Transcript, step.Value) {
						log += "FAIL: Rally assistant did not complete\n"
						a.exitCode = 1
					}
				case "quit":
					log += "quit\n"
					_ = os.WriteFile(path+".result", []byte(log), 0600)
					a.window.Close()
				default:
					log += "Unknown action: " + step.Action + "\n"
				}
				close(done)
			})
			select {
			case <-done:
			case <-a.ctx.Done():
				return
			}
			log += fmt.Sprintf("%s %s\n", step.Action, step.Value)
		}
		_ = os.WriteFile(path+".result", []byte(log), 0600)
	})
}
