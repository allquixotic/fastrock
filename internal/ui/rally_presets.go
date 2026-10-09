package ui

import (
	"context"
	"fmt"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
)

func (a *App) rallyPresetScope() string {
	return fmt.Sprintf("%s\x00%s\x00%t\x00%t", a.prefs.RallyWorkspace, a.prefs.RallyProject, a.prefs.ProjectParents, a.prefs.ProjectChildren)
}

func (a *App) prepareRallyPreset(v *rallyView) bool {
	v.PresetError = ""
	if v.Spec.ID == "mywork" && a.rallyUser == nil {
		v.PresetError = "Loading your Rally identity…"
		if a.rallyUserError != "" {
			v.PresetError = "Could not identify your Rally account: " + a.rallyUserError
		}
		return false
	}
	if v.CurrentIteration {
		if a.iterationScope != a.rallyPresetScope() {
			v.PresetError = "Loading current iteration…"
			if a.iterationError != "" {
				v.PresetError = "Could not find the current iteration: " + a.iterationError
			}
			return false
		}
		v.Timebox = currentIteration(a.iterations, a.prefs.RallyProject, time.Now())
		v.TimeboxName = lookupTimeboxName(a.iterations, v.Timebox)
	}
	a.prepareTimeboxNames(v)
	return true
}

func currentIteration(rows []rally.Object, project string, now time.Time) string {
	var chosen rally.Object
	var chosenStart time.Time
	for _, row := range rows {
		start, se := time.Parse(time.RFC3339, row.String("StartDate"))
		end, ee := time.Parse(time.RFC3339, row.String("EndDate"))
		if se != nil || ee != nil || now.Before(start) || !now.Before(end) || row.String("_ref") == "" {
			continue
		}
		exact, priorExact := project != "" && row.Ref("Project") == project, project != "" && chosen.Ref("Project") == project
		if chosen == nil || exact && !priorExact || exact == priorExact && (start.After(chosenStart) || start.Equal(chosenStart) && row.String("_ref") < chosen.String("_ref")) {
			chosen, chosenStart = row, start
		}
	}
	return chosen.String("_ref")
}

func (a *App) loadRallyUser() {
	c := a.rallyClient
	if c == nil || a.rallyUserLoading {
		return
	}
	a.rallyUserLoading, a.rallyUserError = true, ""
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		user, err := c.CurrentUser(ctx)
		a.post(func() {
			if a.rallyClient != c {
				return
			}
			a.rallyUserLoading = false
			if err != nil {
				a.rallyUserError = err.Error()
			} else {
				a.rallyUser, a.rallyUserError = user, ""
			}
			for _, v := range a.rallyViews {
				if v.Spec.ID == "mywork" {
					a.refreshRallyItems(v)
				}
				if err == nil {
					applyDefaultOwner(v.Detail, user)
				}
			}
		})
	}, func() {
		if a.rallyClient == c {
			a.rallyUserLoading, a.rallyUserError = false, errWorkQueueFull.Error()
			for _, v := range a.rallyViews {
				if v.Spec.ID == "mywork" {
					a.refreshRallyItems(v)
				}
			}
		}
	})
}

func applyDefaultOwner(d *detailView, user rally.Object) {
	if d == nil || !d.New || d.ownerDefaultEditor == nil || user.String("_ref") == "" {
		return
	}
	ed := d.ownerDefaultEditor
	d.ownerDefaultEditor = nil
	if d.Editors["Owner"] == ed && ed.TextRevision() == d.ownerDefaultRevision && text(ed) == "" {
		setText(ed, user.String("_ref"))
		d.Original["Owner"] = map[string]any(user.Clone())
		d.snapshotRevision++
	}
}

func (d *detailView) pendingDefaultOwner() bool {
	return d != nil && d.New && d.ownerDefaultEditor != nil && d.Editors["Owner"] == d.ownerDefaultEditor &&
		d.ownerDefaultEditor.TextRevision() == d.ownerDefaultRevision && text(d.ownerDefaultEditor) == ""
}
