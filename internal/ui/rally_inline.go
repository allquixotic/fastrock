package ui

import (
	"fmt"
	"math"
	"net/url"
	"slices"
	"strconv"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/rally"
)

func (a *App) inlineScope() string {
	return a.prefs.RallyEndpoint + "\x00" + a.prefs.RallyWorkspace + "\x00" + a.prefs.RallyProject + fmt.Sprint(a.prefs.ProjectParents, a.prefs.ProjectChildren)
}

func inlineField(v *rallyView, o rally.Object, name string) (rally.Field, bool) {
	for _, f := range v.metadataFor(o).Fields {
		if f.Name == name {
			switch name {
			case "Rank", "DragAndDropRank", "FormattedID", "ObjectID", "VersionId", "LastUpdateDate", "CreationDate":
				return f, false
			}
			return f, !f.ReadOnly && f.AttributeType != "COLLECTION" && f.AttributeType != "TEXT"
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
		d.setStates(v.metadataFor(o).Workflow)
		d.inlineField, d.inlineScope = name, a.inlineScope()
		d.Fields = []rally.Field{f} // Field-level validation cannot require unfetched unrelated fields.
		if value != nil {
			setText(d.Editors[name], *value)
		}
		v.Detail = d
		if value != nil && f.AttributeType == "BOOLEAN" {
			a.saveDetail(v)
		} else {
			d.Editors[name].Flags |= desktop.EditSigEnter
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
	value := d.originalValue(d.inlineField)
	if f, _ := d.schemaField(d.inlineField); f.AttributeType == "BOOLEAN" {
		value = strconv.FormatBool(d.Original.Bool(d.inlineField))
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
	value := d.fieldValue(d.inlineField)
	if f.AttributeType == "BOOLEAN" && value != "true" && value != "false" {
		return nil, fmt.Errorf("%s must be checked or unchecked", detailCaption(f))
	}
	if f.AttributeType != "STRING" {
		value = strings.TrimSpace(value)
	}
	if f.Name == "State" && strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem") && value != "" && !slices.ContainsFunc(d.States, func(o rally.Object) bool { return o.String("_ref") == value }) {
		return nil, fmt.Errorf("Choose a valid state")
	}
	return d.changesFor(map[string]string{d.inlineField: value})
}

func (a *App) finishInline(v *rallyView, d *detailView) {
	if v.Detail == d && d.inlineField != "" && !d.dirty() {
		a.disposeDetail(d)
		v.Detail = nil
	}
}

func (a *App) drawInlineStatus(w *desktop.Window, v *rallyView) {
	d := v.Detail
	if d == nil || d.inlineField == "" {
		return
	}
	// Starting a reference search must not move its trigger and dismiss the
	// dropdown. Reveal the status bar after selection or dismissal instead.
	if d.referencePicker != nil && d.referencePicker.hideInlineStatus {
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

// inlineChoices uses the concrete artifact's schema, including reference-backed states.
func inlineChoices(v *rallyView, o rally.Object, f rally.Field) (names, values []string) {
	if f.Name == "State" && strings.HasPrefix(strings.ToLower(v.objectKind(o)), "portfolioitem") {
		for _, state := range v.metadataFor(o).Workflow {
			names = append(names, state.String("Name"))
			values = append(values, state.String("_ref"))
		}
	} else {
		names = append(names, f.AllowedValues...)
		values = append(values, f.AllowedValues...)
	}
	if len(values) > 0 && !f.Required {
		names, values = append([]string{"None"}, names...), append([]string{""}, values...)
	}
	return
}

func (a *App) drawInlineCell(w *desktop.Window, v *rallyView, o rally.Object, name string) bool {
	f, editable := inlineField(v, o, name)
	if !editable {
		return false
	}
	d := v.Detail
	active := d != nil && d.inlineField == name && d.Original.String("_ref") == o.String("_ref")
	enabled := a.rallyClient != nil && !v.Mutating && len(v.PendingCards) == 0 && (d == nil || !d.Saving)
	value := o.String(name)
	if active {
		value = d.fieldValue(name)
	}
	if !enabled {
		w.Label(fallback(value, "—"), "LC")
		return true
	}
	if f.AttributeType == "BOOLEAN" {
		checked := o.Bool(name)
		if active {
			checked = value == "true"
		}
		if w.CheckboxText("", &checked) {
			next := strconv.FormatBool(checked)
			if active {
				setText(d.Editors[name], next)
				a.saveDetail(v)
			} else {
				a.startInline(v, o, name, &next)
			}
		}
		return true
	}
	if names, values := inlineChoices(v, o, f); len(values) > 0 {
		current := value
		if f.AttributeType == "OBJECT" && !active {
			current = o.Ref(name)
		}
		caption := fallback(value, "None")
		if i := slices.Index(values, current); i >= 0 {
			caption = names[i]
		}
		if menu := w.ComboWidth(label.T(caption), 180, 280, nil); menu != nil {
			menu.Row(30).Dynamic(1)
			for i, option := range names {
				if menu.MenuItem(label.T(option)) {
					if active {
						setText(d.Editors[name], values[i])
					} else {
						a.startInline(v, o, name, &values[i])
					}
				}
			}
		}
		return true
	}
	if f.AttributeType == "OBJECT" || isReferenceField(name) && name != "State" {
		caption := fallback(o.String(name), "None")
		if active {
			if value == "" {
				caption = "None"
			} else if label := d.referenceLabels[name+"\x00"+value]; label != "" {
				caption = label
			}
		}
		if menu := w.ComboWidth(label.T(caption), 320, 400, nil); menu != nil {
			hadStatus := v.Detail != nil && v.Detail.inlineField != ""
			if !active {
				a.startInline(v, o, name, nil)
			}
			next := v.Detail
			if next == nil || next.inlineField != name || next.Original.String("_ref") != o.String("_ref") {
				menu.Close()
				return true
			}
			p := next.referencePicker
			if p == nil {
				p = a.newReferencePicker(next, f)
				if p != nil {
					p.hideInlineStatus = !hadStatus
				}
			}
			if p == nil {
				menu.Row(30).Dynamic(1)
				menu.Label("No choices available", "LC")
				return true
			}
			p.current = func() bool { return v.Detail == next && a.inlineScope() == next.inlineScope && !next.Saving }
			menu.OnClose(p.close)
			// A reference may have thousands of values. Load and search one bounded
			// page in the background while retaining labels and validated refs.
			menu.Row(30).Dynamic(1)
			p.search.Flags |= desktop.EditSigEnter
			if p.search.Edit(menu)&desktop.EditCommitted != 0 {
				a.searchReferences(p, 1)
			}
			if menu.ButtonText("Search") {
				a.searchReferences(p, 1)
			}
			if !f.Required && menu.MenuItem(label.T("None")) {
				setText(next.Editors[name], "")
				p.close()
				return true
			}
			if p.loading {
				menu.Label("Loading…", "LC")
			}
			if p.err != "" {
				menu.LabelWrap(p.err)
			}
			for _, item := range p.items {
				if menu.MenuItem(label.T(referenceLabel(item))) {
					a.selectReference(p, item)
					break
				}
			}
			if !p.loading && p.err == "" && len(p.items) == 0 {
				menu.Label("No matching items", "LC")
			}
			if p.start > 1 && menu.ButtonText("Previous choices") {
				a.searchReferences(p, max(1, p.start-50))
			}
			if p.start+50 <= p.total && menu.ButtonText("More choices") {
				a.searchReferences(p, p.start+50)
			}
		}
		return true
	}
	if active {
		if d.Editors[name].Edit(w)&desktop.EditCommitted != 0 {
			a.saveDetail(v)
		}
	} else if enabledButton(w, fallback(value, "—"), true, false, a.p) {
		a.startInline(v, o, name, nil)
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

func (a *App) tableRowMenu(w *desktop.Window, v *rallyView, o rally.Object) {
	if menu := w.Menu(label.S(label.SymbolChevronDown), 220, nil); menu != nil {
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
