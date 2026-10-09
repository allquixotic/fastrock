package ui

import (
	"context"
	"encoding/json"
	"time"

	"github.com/allquixotic/fastrock/internal/workspace"
)

func accountNeedsLogin(data map[string]any) bool {
	required, _ := data["requiresOpenaiAuth"].(bool)
	account, _ := data["account"].(map[string]any)
	return required && account == nil
}

func (a *App) refreshAccount() {
	if a.client == nil || a.serverPaused || a.accountLoading && a.accountServer == a.serverGeneration {
		return
	}
	a.accountLoading = true
	a.accountRequest++
	a.accountServer = a.serverGeneration
	request, generation := a.accountRequest, a.serverGeneration
	c := a.client
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		var data map[string]any
		err := c.Call(ctx, "account/read", map[string]any{"refreshToken": false}, &data)
		scrub(data)
		encoded, _ := json.MarshalIndent(data, "", "  ")
		a.post(func() {
			if request != a.accountRequest {
				return
			}
			a.accountLoading = false
			if c != a.client || generation != a.serverGeneration {
				return
			}
			a.accountError = ""
			if err != nil {
				a.accountError = err.Error()
			} else {
				a.accountData = data
				a.accountLoaded = true
				if s := a.settingsView; s != nil && s.Page == "Account" {
					setText(s.Output, string(encoded))
				}
			}
		})
	}, func() { a.accountLoading = false })
}

// Only active views with time-dependent content need periodic redraws.
// Event delivery, typing and notice expiry have their own wakeups.
func (a *App) scheduleTimeUpdate() {
	if a.state == nil {
		return
	}
	active := a.state.Current()
	needed := a.prefs.Sidebar
	fast, animating := false, false
	if active != nil {
		fast = a.rallyViews[active.ID] != nil && a.rallyClient != nil
		if fast {
			animating, _, _ = rallyRefreshState(a.rallyViews[active.ID])
		}
		if active.Kind == workspace.Settings && a.settingsView != nil && a.settingsView.Page == "Account" && a.accountUsage.Loaded {
			fast = true
		}
		if a.prefs.Info && a.infoViews[active.Target] != nil {
			fast = true
		}
	}
	needed = needed || fast
	if !needed {
		if a.timeRefresh != nil {
			a.timeRefresh.Stop()
			a.timeRefresh = nil
			a.timeRefreshGeneration++
		}
		return
	}
	interval := 5 * time.Second
	if !fast {
		interval = time.Minute
	}
	if animating {
		interval = 100 * time.Millisecond
	}
	if a.timeRefresh != nil && a.timeRefreshInterval != interval {
		a.timeRefresh.Stop()
		a.timeRefresh = nil
		a.timeRefreshGeneration++
	}
	if a.timeRefresh == nil {
		a.timeRefreshInterval = interval
		a.timeRefreshGeneration++
		generation := a.timeRefreshGeneration
		a.timeRefresh = time.AfterFunc(interval, func() {
			a.post(func() {
				if generation == a.timeRefreshGeneration {
					a.timeRefresh = nil
				}
			})
		})
	}
}
