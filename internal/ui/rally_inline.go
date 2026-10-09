package ui

import (
	"fmt"
	"math"
	"net/url"
	"strconv"
	"strings"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/label"
	"github.com/allquixotic/fastrock/internal/rally"
)

func (a *App) inlineScope() string {
	return a.prefs.RallyEndpoint + "\x00" + a.prefs.RallyWorkspace + "\x00" + a.prefs.RallyProject + fmt.Sprint(a.prefs.ProjectParents, a.prefs.ProjectChildren)
}

func inlineField(v *rallyView, o rally.Object, name string) (rally.Field, bool) {
	for _, f := range v.metadataFor(o).Fields {
		if f.Name == name {
			return f, !f.ReadOnly && (name == "PlanEstimate" || name == "Blocked")
		}
	}
	return rally.Field{}, false
}

func (a *App) startInline(v *rallyView, o rally.Object, name string, value *string) {
	f, editable := inlineField(v, o, name)
	if !editable || a.rallyClient == nil || v.Closed || v.Mutating || len(v.PendingCards) > 0 {
		return
	}
	kind, valid := a.rallyClient.ReferenceKind(o.String("_ref"))
	if !valid {
		a.toast = "Reload this item before editing"
		return
	}
	client, previous := a.rallyClient, v.Detail
	begin := func() {
		if a.rallyClient != client || v.Detail != previous {
			a.toast = "The editor or Rally connection changed; reopen the inline edit."
			return
		}
		if v.Closed || v.Mutating || len(v.PendingCards) > 0 {
			return
		}
		source := o
		if old := v.Detail; old != nil && a.rallyClient.SameReference(old.Original.String("_ref"), o.String("_ref")) {
			source = old.Original
		}
		a.disposeDetail(v.Detail)
		d := makeDetail(source, kind, false)
		mergeSchemaEditors(d, v.metadataFor(o).Fields)
		d.inlineField, d.inlineScope = name, a.inlineScope()
		d.Fields = []rally.Field{f} // Field-level validation cannot require unfetched unrelated fields.
		if value != nil {
			setText(d.Editors[name], *value)
		}
		v.Detail = d
		if name == "Blocked" {
			a.saveDetail(v)
		} else {
			d.Editors[name].Flags |= nucular.EditSigEnter
			d.Editors[name].Active = true
		}
	}
	if v.Detail != nil {
		a.leaveDetail(v, begin)
	} else {
		begin()
	}
}

func (d *detailView) inlineDirty() bool {
	value := d.Original.String(d.inlineField)
	if d.inlineField == "Blocked" {
		value = strconv.FormatBool(d.Original.Bool("Blocked"))
	}
	return d.fieldValue(d.inlineField) != value || d.Conflict || len(d.fieldConflicts) > 0
}

func (a *App) inlineChanges(d *detailView) (rally.Object, error) {
	if a.inlineScope() != d.inlineScope {
		return nil, fmt.Errorf("Restore the original Rally connection and scope before saving this edit.")
	}
	if d.Original.String("LastUpdateDate") == "" && d.Original.String("VersionId") == "" {
		return nil, fmt.Errorf("Reload this item to read its revision before editing.")
	}
	f, ok := d.schemaField(d.inlineField)
	if !ok || f.ReadOnly {
		return nil, fmt.Errorf("This field is no longer editable.")
	}
	if !d.inlineDirty() {
		return rally.Object{}, nil
	}
	value := strings.TrimSpace(d.fieldValue(d.inlineField))
	var result any
	switch d.inlineField {
	case "PlanEstimate":
		if value == "" {
			if f.Required {
				return nil, fmt.Errorf("%s is required", detailCaption(f))
			}
		} else {
			n, err := strconv.ParseFloat(value, 64)
			if err != nil || math.IsNaN(n) || math.IsInf(n, 0) || n < 0 {
				return nil, fmt.Errorf("%s must be a finite non-negative number", detailCaption(f))
			}
			result = n
		}
	case "Blocked":
		if value != "true" && value != "false" {
			return nil, fmt.Errorf("Blocked must be checked or unchecked")
		}
		result = value == "true"
	default:
		return nil, fmt.Errorf("Unsupported inline field")
	}
	return rally.Object{d.inlineField: result}, nil
}

func (a *App) finishInline(v *rallyView, d *detailView) {
	if v.Detail == d && d.inlineField != "" && !d.dirty() {
		a.disposeDetail(d)
		v.Detail = nil
	}
}

func (a *App) drawInlineStatus(w *nucular.Window, v *rallyView) {
	d := v.Detail
	if d == nil || d.inlineField == "" {
		return
	}
	f, _ := d.schemaField(d.inlineField)
	ready := !d.Saving && !v.Mutating && len(v.PendingCards) == 0
	w.Row(30).Ratio(.5, .16, .16, .18)
	w.Label(d.Original.ID()+" · Editing "+detailCaption(f), "LC")
	if enabledButton(w, "Save", ready && !d.Conflict && len(d.fieldConflicts) == 0, true, a.p) {
		a.saveDetail(v)
	}
	if enabledButton(w, "Cancel", ready, false, a.p) {
		a.disposeDetail(d)
		v.Detail = nil
		return
	}
	if enabledButton(w, "Full editor", ready, false, a.p) {
		d.inlineField = ""
		d.snapshotRevision++
		a.rehydrateDetail(v)
		return
	}
	if d.Saving {
		muted(w, "Saving / reloading…", a.p)
	}
	if d.Error != "" {
		muted(w, d.Error, a.p)
	}
	a.drawDetailRecovery(w, v, d)
}

func (a *App) drawInlineCell(w *nucular.Window, v *rallyView, o rally.Object, name string) bool {
	if name != "PlanEstimate" && name != "Blocked" {
		return false
	}
	_, editable := inlineField(v, o, name)
	d := v.Detail
	active := d != nil && d.inlineField == name && d.Original.String("_ref") == o.String("_ref")
	enabled := editable && a.rallyClient != nil && !v.Mutating && len(v.PendingCards) == 0 && (d == nil || !d.Saving)
	if active {
		if name == "PlanEstimate" {
			if !enabled {
				w.Label(text(d.Editors[name]), "LC")
			} else if d.Editors[name].Edit(w)&nucular.EditCommitted != 0 {
				a.saveDetail(v)
			}
		} else {
			value := text(d.Editors[name]) == "true"
			if enabled && w.CheckboxText("", &value) {
				setText(d.Editors[name], strconv.FormatBool(value))
				a.saveDetail(v)
			} else if !enabled {
				w.Label(strconv.FormatBool(value), "LC")
			}
		}
		return true
	}
	if name == "Blocked" {
		value := o.Bool(name)
		if enabled {
			if w.CheckboxText("", &value) {
				text := strconv.FormatBool(value)
				a.startInline(v, o, name, &text)
			}
		} else {
			w.Label(strconv.FormatBool(value), "LC")
		}
	} else {
		value := fallback(o.String(name), "—")
		if enabledButton(w, value, enabled, false, a.p) {
			a.startInline(v, o, name, nil)
		}
		if w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
			w.Tooltip("Edit " + rallyFieldLabel(v, name))
		}
	}
	return true
}

func artifactWebURL(endpoint, kind string, o rally.Object) string {
	if o.String("ObjectID") == "" {
		return ""
	}
	if kind == "HierarchicalRequirement" {
		kind = "userstory"
	}
	return strings.TrimRight(endpoint, "/") + "/#/detail/" + strings.ToLower(kind) + "/" + url.PathEscape(o.String("ObjectID"))
}

func (a *App) tableRowMenu(w *nucular.Window, v *rallyView, o rally.Object) {
	if menu := w.Menu(label.T("⋮"), 220, nil); menu != nil {
		menu.Row(28).Dynamic(1)
		if menu.MenuItem(label.T("Open work item")) {
			a.openArtifact(v, o)
		}
		if menu.MenuItem(label.T("Copy ID")) {
			a.copyText(o.ID())
		}
		if menu.MenuItem(label.T("Open in Rally")) {
			if link := artifactWebURL(a.prefs.RallyEndpoint, v.objectKind(o), o); link != "" {
				a.openURL(link)
			}
		}
		if menu.MenuItem(label.T("Delete…")) {
			if v.Mutating || len(v.PendingCards) > 0 || v.Detail != nil && (v.Detail.Saving || v.Detail.dirty()) {
				a.toast = "Save or cancel the current edit before deleting"
			} else {
				before := o.Clone()
				a.confirm("Delete "+o.ID()+"?", "This deletes the work item from Rally.", func() { a.deleteArtifact(v, before) })
			}
		}
	}
	if w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
		w.Tooltip("Actions for " + o.ID())
	}
}

func tableTotals(columns []string, items []rally.Object) map[string]string {
	totals := map[string]string{}
	for _, name := range columns {
		switch name {
		case "PlanEstimate", "Estimate", "ToDo", "Actuals":
			sum := 0.0
			for _, o := range items {
				if n := o.Number(name); !math.IsNaN(n) && !math.IsInf(n, 0) {
					sum += n
				}
			}
			totals[name] = strconv.FormatFloat(sum, 'f', -1, 64)
		case "Blocked":
			count := 0
			for _, o := range items {
				if o.Bool(name) {
					count++
				}
			}
			totals[name] = strconv.Itoa(count)
		}
	}
	return totals
}
