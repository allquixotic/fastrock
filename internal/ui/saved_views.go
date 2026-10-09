package ui

import (
	"github.com/allquixotic/fastrock/internal/settings"
	"reflect"
	"slices"
	"strings"
)

func savedViewEqual(a, b settings.SavedView) bool {
	a, b = normalizeSavedTimeboxes(a), normalizeSavedTimeboxes(b)
	// Old definitions have only a reference. Resolving their display/query name
	// does not make an otherwise untouched view dirty.
	if a.Timebox == b.Timebox && (a.TimeboxName == "" || b.TimeboxName == "") {
		a.TimeboxName, b.TimeboxName = "", ""
	}
	if a.ReleaseTimebox == b.ReleaseTimebox && (a.ReleaseName == "" || b.ReleaseName == "") {
		a.ReleaseName, b.ReleaseName = "", ""
	}

	if a.Display == nil {
		a.Display = settings.DefaultBoardDisplay().Copy()
	}
	if b.Display == nil {
		b.Display = settings.DefaultBoardDisplay().Copy()
	}
	return reflect.DeepEqual(a, b)
}
func normalizeSavedTimeboxes(v settings.SavedView) settings.SavedView {
	if legacyRelease(v.Timebox) {
		if v.ReleaseTimebox == "" {
			v.ReleaseTimebox, v.ReleaseName = v.Timebox, v.TimeboxName
		}
		v.Timebox, v.TimeboxName = "", ""
	}
	return v
}

func (a *App) findSavedView(page, name string) *settings.SavedView {
	for _, v := range a.prefs.Views {
		if v.Page == page && v.Name == name {
			copy := v.Clone()
			return &copy
		}
	}
	return nil
}
func (a *App) storeSavedView(v *rallyView, saved settings.SavedView, before *settings.SavedView) {
	if v.Closed {
		return
	}
	if err := a.changeSavedView(before, &saved); err != nil {
		a.report(err)
		return
	}
	v.ViewName = strings.TrimSpace(saved.Name)
}

func (v *rallyView) savedView(name string) settings.SavedView {
	timebox, timeboxName := v.Timebox, v.TimeboxName
	if v.CurrentIteration {
		timebox, timeboxName = "", ""
	}
	return settings.SavedView{Display: v.Display.Copy(), Filters: slices.Clone(v.StructuredFilters), CardFields: append([]string{}, v.CardFields...), Name: name, Page: v.Spec.ID, Query: v.QueryApplied, Group: v.Group, Mode: v.Mode, Search: text(v.Search), Timebox: timebox, TimeboxName: timeboxName, ReleaseTimebox: v.ReleaseTimebox, ReleaseName: v.ReleaseName, CurrentIteration: v.CurrentIteration, Owner: v.OwnerFilter, State: v.StateFilter, Blocked: v.OnlyBlocked, Ready: v.OnlyReady, Columns: append([]string(nil), v.Columns...), Sort: v.Sort, Descending: v.Descending}
}
func (v *rallyView) applySavedView(s settings.SavedView) {
	if s.Display != nil {
		v.Display = settings.DisplayOrDefault(s.Display)
	} else if s.Name == "" && s.Page == "" {
		v.Display = settings.DefaultBoardDisplay()
	}
	v.DisplayDraft = nil
	v.AIView = false
	v.CurrentIteration = s.CurrentIteration || s.Name == "" && s.Page == "" && s.Timebox == "" && v.Spec.ID == "iterationstatus"
	setText(v.Query, s.Query)
	v.QueryApplied = s.Query
	v.StructuredFilters = slices.Clone(s.Filters)
	v.resetFilterDraft("Iteration", "is")
	v.filterValid = false
	setText(v.Search, s.Search)
	v.ViewName = s.Name
	v.Group = fallback(s.Group, "None")
	v.Mode = fallback(s.Mode, v.Spec.Mode)
	v.Timebox, v.OwnerFilter, v.StateFilter = s.Timebox, s.Owner, s.State
	v.TimeboxName, v.ReleaseTimebox, v.ReleaseName = s.TimeboxName, s.ReleaseTimebox, s.ReleaseName
	v.migrateTimeboxes()
	v.OnlyBlocked, v.OnlyReady = s.Blocked, s.Ready
	v.CardFields = append([]string{}, s.CardFields...)
	if s.CardFields == nil {
		v.CardFields = newRallyView(v.Spec).CardFields
	}
	v.Columns = append([]string(nil), s.Columns...)
	if len(v.Columns) == 0 {
		v.Columns = newRallyView(v.Spec).Columns
	}
	v.Sort, v.Descending = fallback(s.Sort, newRallyView(v.Spec).Sort), s.Descending
	v.Page = 1
}
