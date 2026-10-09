package ui

import (
	"context"
	"fmt"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/settings"
	"reflect"
	"time"
)

type preferenceWrite struct {
	Patch    settings.Patch
	Snapshot settings.Preferences
	Client   *codex.Client
}

func (a *App) persistPreferences() {
	defer close(a.preferencesDone)
	var pending *preferenceWrite
	var retry <-chan time.Time
	delay := time.Second
	for {
		select {
		case <-a.ctx.Done():
			return
		case next := <-a.preferences:
			if pending != nil {
				for k, v := range next.Patch {
					pending.Patch[k] = v
				}
				next.Patch = pending.Patch
			}
			pending = &next
		case <-retry:
		}
		if pending == nil {
			continue
		}
		write := *pending
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		var state codex.PreferencesState
		var err error
		if write.Client == nil {
			err = fmt.Errorf("Codex broker is disconnected")
		} else {
			err = write.Client.Call(ctx, "fastrock/preferences", map[string]any{"data": write.Patch}, &state)
		}
		cancel()
		if err != nil {
			a.post(func() { a.report(fmt.Errorf("preferences not saved: %w", err)) })
			retry = time.After(delay)
			delay = min(30*time.Second, delay*2)
			continue
		}
		pending = nil
		retry = nil
		delay = time.Second
		a.post(func() { a.applyPreferences(state, &write.Snapshot) })
	}
}

func (a *App) applyPreferences(state codex.PreferencesState, acknowledged *settings.Preferences) {
	base := a.preferencesServer
	if acknowledged != nil {
		base = *acknowledged
	}
	local := settings.Diff(base, a.prefs)
	if state.Revision >= a.preferencesRevision {
		normal, err := settings.Normalize(state.Data)
		if err != nil {
			a.report(err)
			return
		}
		a.preferencesServer, a.preferencesRevision = normal, state.Revision
	}
	next, err := settings.Apply(a.preferencesServer, local)
	if err != nil {
		a.report(err)
		return
	}
	old := a.prefs
	a.reconcileSavedViewNames(old.Views, next.Views)
	a.prefs, a.preferencesQueued = next, next
	if !reflect.DeepEqual(old.Views, next.Views) {
		a.savedViewsRevision++
	}
	if old.Theme != next.Theme || old.FontSize != next.FontSize {
		a.applyTheme()
		for _, view := range a.chats {
			view.Layouts = nil
			view.RichSelection = transcriptSelection{}
		}
	}
	if old.RallyEndpoint != next.RallyEndpoint {
		a.connectRally()
	} else if old.RallyWorkspace != next.RallyWorkspace || old.RallyProject != next.RallyProject || old.ProjectParents != next.ProjectParents || old.ProjectChildren != next.ProjectChildren {
		a.loadScope()
		a.reloadRally()
	}
}
