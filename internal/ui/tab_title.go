package ui

import "github.com/allquixotic/fastrock/internal/workspace"

func (a *App) tabTitle(t workspace.Tab) string {
	if v := a.rallyViews[t.ID]; v != nil && v.Detail != nil {
		d := v.Detail
		if d.selection != nil {
			return "Edit selected · " + t.Title
		}
		if d.New {
			return "Create " + rallyKindLabel(d.Kind)
		}
		return d.Original.ID() + " · " + fallback(text(d.Editors["Name"]), t.Title)
	}
	return t.Title
}
