package ui

import (
	"context"
	"fmt"
	"math"
	"sort"
	"strings"
	"time"
)

type usageWindow struct {
	UsedPercent *float64 `json:"usedPercent"`
	Minutes     *int64   `json:"windowDurationMins"`
	ResetsAt    *int64   `json:"resetsAt"`
}
type usageCredits struct {
	HasCredits bool    `json:"hasCredits"`
	Unlimited  bool    `json:"unlimited"`
	Balance    *string `json:"balance"`
}
type usageSnapshot struct {
	ID        string        `json:"limitId"`
	Name      string        `json:"limitName"`
	Plan      string        `json:"planType"`
	Primary   *usageWindow  `json:"primary"`
	Secondary *usageWindow  `json:"secondary"`
	Credits   *usageCredits `json:"credits"`
	Reached   *string       `json:"rateLimitReachedType"`
}
type usageLimits struct {
	Legacy *usageSnapshot           `json:"rateLimits"`
	ByID   map[string]usageSnapshot `json:"rateLimitsByLimitId"`
}
type usageState struct {
	Data                      usageLimits
	Loaded, Loading           bool
	Error                     string
	Request, Revision, Server uint64
	Pending                   map[string]usageSnapshot
}
type accountFact struct{ Name, Value string }

func (a *App) refreshUsage() {
	u := &a.accountUsage
	if a.client == nil || a.serverPaused || u.Loading && u.Server == a.serverGeneration {
		return
	}
	u.Request++
	u.Loading, u.Server = true, a.serverGeneration
	u.Pending = nil
	request, generation, client := u.Request, a.serverGeneration, a.client
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		var data usageLimits
		err := client.Call(ctx, "account/rateLimits/read", map[string]any{}, &data)
		a.post(func() {
			if request != u.Request {
				return
			}
			u.Loading = false
			if a.client != client || generation != a.serverGeneration {
				return
			}
			u.Error = ""
			if err != nil {
				u.Error = err.Error()
				return
			}
			for _, patch := range u.Pending {
				data.update(patch)
			}
			u.Pending = nil
			u.Data, u.Loaded = data, true
		})
	}, func() {
		if u.Request == request {
			u.Loading, u.Error = false, errWorkQueueFull.Error()
		}
	})
}

func mergeUsageWindow(old, fresh *usageWindow) *usageWindow {
	if fresh == nil {
		return old
	}
	result := *fresh
	if old != nil {
		if result.UsedPercent == nil {
			result.UsedPercent = old.UsedPercent
		}
		if result.Minutes == nil {
			result.Minutes = old.Minutes
		}
		if result.ResetsAt == nil {
			result.ResetsAt = old.ResetsAt
		}
	}
	return &result
}
func mergeUsageSnapshot(old, fresh usageSnapshot) usageSnapshot {
	if fresh.ID != "" {
		old.ID = fresh.ID
	}
	if fresh.Name != "" {
		old.Name = fresh.Name
	}
	if fresh.Plan != "" {
		old.Plan = fresh.Plan
	}
	old.Primary = mergeUsageWindow(old.Primary, fresh.Primary)
	old.Secondary = mergeUsageWindow(old.Secondary, fresh.Secondary)
	if fresh.Credits != nil {
		credits := *fresh.Credits
		if credits.Balance == nil && old.Credits != nil {
			credits.Balance = old.Credits.Balance
		}
		old.Credits = &credits
	}
	if fresh.Reached != nil {
		old.Reached = fresh.Reached
	}
	return old
}
func (u *usageState) update(snapshot usageSnapshot) {
	u.Revision++
	u.Loaded, u.Error = true, ""
	if u.Loading {
		if u.Pending == nil {
			u.Pending = map[string]usageSnapshot{}
		}
		u.Pending[snapshot.ID] = mergeUsageSnapshot(u.Pending[snapshot.ID], snapshot)
	}
	u.Data.update(snapshot)
}
func (data *usageLimits) update(snapshot usageSnapshot) {
	if len(data.ByID) > 0 || snapshot.ID != "" {
		if data.ByID == nil {
			data.ByID = map[string]usageSnapshot{}
		}
		id := snapshot.ID
		if id == "" {
			id = "codex"
		}
		data.ByID[id] = mergeUsageSnapshot(data.ByID[id], snapshot)
	} else {
		old := usageSnapshot{}
		if data.Legacy != nil {
			old = *data.Legacy
		}
		old = mergeUsageSnapshot(old, snapshot)
		data.Legacy = &old
	}
}

func usageWindowLabel(minutes *int64) string {
	if minutes == nil || *minutes <= 0 {
		return "Window"
	}
	n := *minutes
	switch {
	case n == 10080:
		return "Weekly window"
	case n == 1440:
		return "Daily window"
	case n%1440 == 0:
		return fmt.Sprintf("%d-day window", n/1440)
	case n%60 == 0:
		return fmt.Sprintf("%d-hour window", n/60)
	default:
		return fmt.Sprintf("%d-minute window", n)
	}
}
func usageReset(at *int64, now time.Time) string {
	if at == nil {
		return ""
	}
	left := *at - now.Unix()
	switch {
	case left <= 0:
		return "resets now"
	case left < 60:
		return "resets in under a minute"
	case left < 3600:
		return fmt.Sprintf("resets in %dm", left/60)
	case left < 86400:
		return fmt.Sprintf("resets in %dh %dm", left/3600, left%3600/60)
	default:
		return fmt.Sprintf("resets in %dd %dh", left/86400, left%86400/3600)
	}
}
func usageRows(data usageLimits, now time.Time) []accountFact {
	var snapshots []usageSnapshot
	if len(data.ByID) > 0 {
		ids := make([]string, 0, len(data.ByID))
		for id := range data.ByID {
			ids = append(ids, id)
		}
		sort.Slice(ids, func(i, j int) bool {
			if ids[i] == "codex" {
				return true
			}
			if ids[j] == "codex" {
				return false
			}
			return ids[i] < ids[j]
		})
		for _, id := range ids {
			s := data.ByID[id]
			if s.ID == "" {
				s.ID = id
			}
			snapshots = append(snapshots, s)
		}
	} else if data.Legacy != nil {
		snapshots = append(snapshots, *data.Legacy)
	}
	var rows []accountFact
	for _, s := range snapshots {
		name := fallback(s.Name, fallback(s.ID, "Usage"))
		for _, window := range []*usageWindow{s.Primary, s.Secondary} {
			if window == nil {
				continue
			}
			value := "Usage unavailable"
			if window.UsedPercent != nil && !math.IsNaN(*window.UsedPercent) && !math.IsInf(*window.UsedPercent, 0) {
				value = fmt.Sprintf("%g%% used", math.Max(0, math.Min(100, *window.UsedPercent)))
			}
			if reset := usageReset(window.ResetsAt, now); reset != "" {
				value += " · " + reset
			}
			rows = append(rows, accountFact{name + " · " + usageWindowLabel(window.Minutes), value})
		}
		if c := s.Credits; c != nil {
			value := "None"
			if c.Unlimited {
				value = "Unlimited"
			} else if c.HasCredits {
				value = "Available"
				if c.Balance != nil {
					value = *c.Balance + " remaining"
				}
			}
			rows = append(rows, accountFact{name + " · Credits", value})
		}
		if s.Reached != nil {
			rows = append(rows, accountFact{name + " · Status", strings.ReplaceAll(*s.Reached, "_", " ")})
		}
	}
	return rows
}
