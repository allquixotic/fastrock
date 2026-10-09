package ui

import (
	"context"
	"fmt"
	"maps"
	"net/url"
	"path"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
)

// A selection editor uses the same draft, reference-picker and navigation
// lifecycle as an artifact editor, but keeps a separate immutable write plan.
type selectionEdit struct {
	Kinds                        []string
	StateField                   string
	Targets                      []rally.Object
	Enabled                      map[string]bool
	Rows                         []selectionEditRow
	Endpoint, Workspace, Project string
	Parents, Children            bool
	Attempted, Complete          bool
}
type selectionEditRow struct {
	Before, Fields  rally.Object
	Status, Message string
}

func (s *selectionEdit) clone() *selectionEdit {
	if s == nil {
		return nil
	}
	out := *s
	out.Kinds = slices.Clone(s.Kinds)
	out.Targets = cloneObjects(s.Targets)
	out.Enabled = maps.Clone(s.Enabled)
	out.Rows = slices.Clone(s.Rows)
	for i := range out.Rows {
		out.Rows[i].Before = out.Rows[i].Before.Clone()
		out.Rows[i].Fields = out.Rows[i].Fields.Clone()
	}
	return &out
}

func (a *App) openSelectionEditor(v *rallyView) {
	if a.rallyClient == nil || v.Closed || v.Mutating || v.Detail != nil || len(v.PendingCards) > 0 {
		return
	}
	plan := a.selectionPlan(v, "update", nil)
	if len(plan.Changes) == 0 {
		return
	}
	s := &selectionEdit{StateField: v.stateField(), Enabled: map[string]bool{}, Endpoint: a.prefs.RallyEndpoint, Workspace: a.prefs.RallyWorkspace, Project: a.prefs.RallyProject, Parents: a.prefs.ProjectParents, Children: a.prefs.ProjectChildren}
	for _, ch := range plan.Changes {
		kind, valid := a.rallyClient.ReferenceKind(ch.Ref)
		if !valid || kind != ch.Kind {
			a.toast = "Reload the selection before editing: an artifact type or reference is invalid."
			return
		}
		if !slices.Contains(s.Kinds, kind) {
			s.Kinds = append(s.Kinds, kind)
		}
		s.Targets = append(s.Targets, ch.Before.Clone())
	}
	d := makeDetail(rally.Object{"Name": "Edit selected"}, s.Kinds[0], false)
	d.selection = s
	metadata := commonRallyMetadata(metadataList(v.TypeMetadata, s.Kinds))
	if len(v.TypeMetadata) == 0 && len(s.Kinds) == 1 && s.Kinds[0] == v.Spec.Kind {
		metadata = rallyTypeMetadata{Fields: v.Fields, Workflow: v.Workflow}
	}
	mergeSchemaEditors(d, metadata.Fields)
	d.setStates(cloneObjects(metadata.Workflow))
	v.Detail = d
	if len(metadata.Fields) == 0 {
		a.rehydrateDetail(v)
	}
}

func (d *detailView) selectionStateField() string {
	if d.selection != nil && d.selection.StateField != "" {
		return d.selection.StateField
	}
	return rally.StateField(d.Kind)
}

func (a *App) rehydrateSelection(v *rallyView, d *detailView) {
	s, c := d.selection, a.rallyClient
	if s == nil || c == nil {
		return
	}
	kinds := slices.Clone(s.Kinds)
	if len(kinds) == 0 {
		kinds = []string{d.Kind}
	}
	stateField := d.selectionStateField()
	d.SchemaLoading = true
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		all, err := loadRallyTypeMetadata(ctx, c, kinds, s.Workspace)
		if stateField == "ScheduleState" {
			scheduleMetadata(all)
		}
		metadata := commonRallyMetadata(metadataList(all, kinds))
		a.post(func() {
			if v.Closed || v.Detail != d {
				return
			}
			d.SchemaLoading = false
			if a.rallyClient != c || !a.selectionScopeValid(s) {
				d.SchemaError = "Rally connection or scope changed. Reopen the selection editor."
				return
			}
			if err != nil {
				d.SchemaError = err.Error()
				return
			}
			mergeSchemaEditors(d, metadata.Fields)
			d.setStates(metadata.Workflow)
			d.SchemaError = ""
		})
	}, func() { d.SchemaLoading = false; d.SchemaError = errWorkQueueFull.Error() })
}

func (a *App) selectionScopeValid(s *selectionEdit) bool {
	return a.rallyClient != nil && s.Endpoint == a.prefs.RallyEndpoint && s.Workspace == a.prefs.RallyWorkspace && s.Project == a.prefs.RallyProject && s.Parents == a.prefs.ProjectParents && s.Children == a.prefs.ProjectChildren
}

func (d *detailView) selectionFields() []rally.Field {
	var fields []rally.Field
	for _, name := range []string{d.selectionStateField(), "Owner", "Iteration"} {
		f, ok := d.schemaField(name)
		if !ok || f.ReadOnly {
			continue
		}
		if name == d.selectionStateField() && len(f.AllowedValues) == 0 && len(d.States) == 0 {
			continue
		}
		fields = append(fields, f)
	}
	return fields
}

func (a *App) selectionValues(d *detailView) (rally.Object, error) {
	s := d.selection
	if !a.selectionScopeValid(s) {
		return nil, fmt.Errorf("Rally connection or scope changed. Reopen Edit selected in the current scope.")
	}
	if d.SchemaLoading || d.SchemaError != "" || d.Loading {
		return nil, fmt.Errorf("Wait for the selected items and field metadata to load.")
	}
	values := rally.Object{}
	for name, enabled := range s.Enabled {
		if !enabled {
			continue
		}
		f, ok := d.schemaField(name)
		if !ok || f.ReadOnly || !slices.ContainsFunc(d.selectionFields(), func(x rally.Field) bool { return x.Name == name }) {
			return nil, fmt.Errorf("%s is not an available writable field", name)
		}
		value := text(d.Editors[name])
		if strings.TrimSpace(value) == "" {
			if f.Required {
				return nil, fmt.Errorf("%s is required", detailCaption(f))
			}
			values[name] = nil
			continue
		}
		if name == d.selectionStateField() && !strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem") {
			if !slices.Contains(f.AllowedValues, value) {
				return nil, fmt.Errorf("Choose a valid %s from the workspace schema", detailCaption(f))
			}
		} else {
			kind, valid := a.rallyClient.ReferenceKind(value)
			expected := ""
			switch name {
			case "Owner":
				expected = "User"
			case "Iteration":
				expected = "Iteration"
			case "State":
				expected = "State"
			}
			if !valid || kind != expected {
				return nil, fmt.Errorf("Choose a valid %s from this Rally connection", detailCaption(f))
			}
			if name == "State" && !slices.ContainsFunc(d.States, func(o rally.Object) bool { return a.rallyClient.SameReference(o.String("_ref"), value) }) {
				return nil, fmt.Errorf("Choose a valid state for this artifact type")
			}
		}
		values[name] = value
	}
	if len(values) == 0 {
		return nil, fmt.Errorf("Choose at least one field to change")
	}
	return values, nil
}

func (a *App) selectionRows(d *detailView) ([]selectionEditRow, error) {
	values, err := a.selectionValues(d)
	if err != nil {
		return nil, err
	}
	if len(d.selection.Targets) == 0 {
		return nil, fmt.Errorf("No selected items remain")
	}
	var rows []selectionEditRow
	seen := map[string]bool{}
	for _, o := range d.selection.Targets {
		ref := o.String("_ref")
		kind, valid := a.rallyClient.ReferenceKind(ref)
		allowed := slices.Contains(d.selection.Kinds, kind) || len(d.selection.Kinds) == 0 && kind == d.Kind
		if !valid || !allowed {
			return nil, fmt.Errorf("Selection contains an invalid or different artifact type")
		}
		u, _ := url.Parse(ref) // ReferenceKind already validates the URL.
		identity := kind + "/" + strings.TrimLeft(path.Base(u.Path), "0")
		if seen[identity] {
			return nil, fmt.Errorf("Selection contains a duplicate artifact")
		}
		seen[identity] = true
		if o.String("LastUpdateDate") == "" && o.String("VersionId") == "" {
			return nil, fmt.Errorf("Reload selected items to read the revision of %s", o.ID())
		}
		fields := rally.Object{}
		for name, value := range values {
			previous := o.String(name)
			if name == "Owner" || name == "Iteration" || name == "State" && strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem") {
				previous = o.Ref(name)
			}
			next, _ := value.(string)
			if previous == next || previous != "" && next != "" && a.rallyClient.SameReference(previous, next) {
				continue
			}
			fields[name] = value
		}
		status := "Ready"
		if len(fields) == 0 {
			status = "Unchanged"
		}
		rows = append(rows, selectionEditRow{Before: o.Clone(), Fields: fields, Status: status})
	}
	return rows, nil
}

func (a *App) prepareSelection(v *rallyView) error {
	d := v.Detail
	if d == nil || d.selection == nil || d.Saving || v.Closed {
		return fmt.Errorf("Selection editor is unavailable")
	}
	if d.selection.Attempted {
		return fmt.Errorf("Reload remaining items before reviewing another batch")
	}
	rows, err := a.selectionRows(d)
	if err != nil {
		d.Error = err.Error()
		return err
	}
	d.selection.Rows = rows
	d.Error = ""
	d.snapshotRevision++
	return nil
}

func (a *App) applySelection(v *rallyView) {
	d := v.Detail
	if d == nil || d.selection == nil || d.Saving || d.Loading || v.Closed || v.Mutating || len(v.PendingCards) > 0 {
		return
	}
	s := d.selection
	if s.Attempted || len(s.Rows) == 0 {
		d.Error = "Review changes before applying; reload after a previous attempt."
		return
	}
	current, err := a.selectionRows(d)
	if err != nil {
		d.Error = err.Error()
		return
	}
	if !reflect.DeepEqual(current, s.Rows) {
		d.Error = "The draft changed after review. Review changes again before applying."
		return
	}
	if d.referencePicker != nil {
		d.referencePicker.close()
	}
	s.Attempted = true
	d.Saving, v.Mutating = true, true
	d.Error = ""
	d.snapshotRevision++
	a.applySelectionNext(v, d, a.rallyClient, 0)
}

func (a *App) finishSelection(v *rallyView, d *detailView) {
	d.Saving, v.Mutating = false, false
	d.selection.Complete = !slices.ContainsFunc(d.selection.Rows, func(row selectionEditRow) bool { return row.Status != "Updated" && row.Status != "Unchanged" })
	d.snapshotRevision++
	updated := 0
	for _, row := range d.selection.Rows {
		if row.Status == "Updated" {
			updated++
		}
	}
	if d.selection.Complete {
		a.toast = fmt.Sprintf("Updated %d selected items", updated)
		if updated == 0 {
			a.toast = "Selected items already match"
		}
	} else {
		d.Error = fmt.Sprintf("Updated %d of %d selected items. %s", updated, len(d.selection.Rows), d.Error)
	}
	a.refreshRallyItems(v)
	if d.selection.Complete && d.afterSave != nil {
		next := d.afterSave
		d.afterSave = nil
		next()
	}
}

func (a *App) applySelectionNext(v *rallyView, d *detailView, c *rally.Client, index int) {
	if v.Closed || v.Detail != d {
		d.Saving = false
		return
	}
	s := d.selection
	if c != a.rallyClient || !a.selectionScopeValid(s) {
		d.Error = "Rally connection or scope changed. Remaining items were not attempted. Reload in the original scope before retrying."
		for i := index; i < len(s.Rows); i++ {
			if s.Rows[i].Status == "Ready" {
				s.Rows[i].Status = "Not attempted"
			}
		}
		a.finishSelection(v, d)
		return
	}
	for index < len(s.Rows) && s.Rows[index].Status == "Unchanged" {
		index++
	}
	if index == len(s.Rows) {
		a.finishSelection(v, d)
		return
	}
	s.Rows[index].Status = "Verify before retry" // persisted if interrupted during the request
	s.Rows[index].Message = "Write in progress; reload to verify if interrupted."
	d.snapshotRevision++
	before, fields := s.Rows[index].Before.Clone(), s.Rows[index].Fields.Clone()
	kind, valid := c.ReferenceKind(before.String("_ref"))
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	d.cancel = cancel
	stop := func(message string) {
		if v.Closed || v.Detail != d {
			d.Saving = false
			return
		}
		s.Rows[index].Message = message
		for i := index + 1; i < len(s.Rows); i++ {
			if s.Rows[i].Status == "Ready" {
				s.Rows[i].Status = "Not attempted"
			}
		}
		d.Error = message
		a.finishSelection(v, d)
	}
	if !valid || !(slices.Contains(s.Kinds, kind) || len(s.Kinds) == 0 && kind == d.Kind) {
		cancel()
		stop("The selected artifact type changed. Reload before retrying.")
		return
	}
	a.writeWork(func() {
		defer cancel()
		saved, err := c.UpdateIfUnchanged(ctx, before, kind, fields)
		a.post(func() {
			if v.Closed || v.Detail != d {
				d.Saving = false
				return
			}
			if err != nil {
				stop(err.Error())
				return
			}
			// The write returned on the original connection. Keep that confirmed
			// result, then let the next dispatch stop if the current scope changed.
			row := &s.Rows[index]
			row.Status, row.Message = "Updated", ""
			row.Before = d.savedSnapshot(before, saved, fields)
			if v.Selected[before.String("_ref")] {
				v.selectItem(before, false)
			}
			d.snapshotRevision++
			a.applySelectionNext(v, d, c, index+1)
		})
	}, func() { cancel(); s.Rows[index].Status = "Not attempted"; stop(errWorkQueueFull.Error()) })
}

func (a *App) reloadSelection(v *rallyView) {
	d := v.Detail
	if d == nil || d.selection == nil || d.Saving || d.Loading || v.Closed {
		return
	}
	s, c := d.selection, a.rallyClient
	if !a.selectionScopeValid(s) {
		d.Error = "Restore the original Rally connection and scope before reloading this selection."
		return
	}
	targets := cloneObjects(s.Targets)
	if s.Attempted {
		targets = nil
		for _, row := range s.Rows {
			if row.Status != "Updated" && row.Status != "Unchanged" {
				targets = append(targets, row.Before.Clone())
			}
		}
	}
	if len(targets) == 0 {
		return
	}
	if d.referencePicker != nil {
		d.referencePicker.close()
	}
	d.Loading = true
	d.Error = ""
	ctx, cancel := context.WithTimeout(a.ctx, 2*time.Minute)
	d.cancel = cancel
	a.work(func() {
		defer cancel()
		fresh := make([]rally.Object, 0, len(targets))
		var err error
		for _, o := range targets {
			var current rally.Object
			current, err = c.Get(ctx, o.String("_ref"))
			if err != nil {
				break
			}
			if !c.SameReference(o.String("_ref"), current.String("_ref")) {
				err = fmt.Errorf("Rally returned a different item while reloading %s", o.ID())
				break
			}
			fresh = append(fresh, current)
		}
		a.post(func() {
			if v.Closed || v.Detail != d {
				return
			}
			d.Loading = false
			if c != a.rallyClient || !a.selectionScopeValid(s) {
				d.Error = "Rally connection or scope changed; the selection and draft were retained."
				return
			}
			if err != nil {
				d.Error = err.Error()
				return
			}
			s.Targets = fresh
			s.Rows = nil
			s.Attempted, s.Complete = false, false
			d.snapshotRevision++
		})
	}, func() { cancel(); d.Loading = false; d.Error = errWorkQueueFull.Error() })
}

func selectionValueLabel(d *detailView, name string, o rally.Object) string {
	value := o.String(name)
	if name == "State" && strings.HasPrefix(strings.ToLower(d.Kind), "portfolioitem") {
		for _, state := range d.States {
			if state.String("_ref") == value || state.String("_ref") == o.Ref(name) {
				return state.String("Name")
			}
		}
	}
	if label := d.referenceLabels[name+"\x00"+value]; label != "" {
		return label
	}
	return fallback(value, "None")
}

func (a *App) drawSelectionEditor(w *nucular.Window, v *rallyView) {
	d, s := v.Detail, v.Detail.selection
	title(w, fmt.Sprintf("Edit selected · %d work items", len(s.Targets)), a.p)
	w.Row(32).Dynamic(3)
	if enabledButton(w, "← Back", !d.Saving, false, a.p) {
		a.closeRallyDetail(v, false)
		return
	}
	if enabledButton(w, "Review changes", !d.Saving && !d.Loading && !d.SchemaLoading && !s.Attempted, false, a.p) {
		d.afterSave = nil
		_ = a.prepareSelection(v)
	}
	if enabledButton(w, "Apply reviewed", !d.Saving && !d.Loading && len(s.Rows) > 0 && !s.Attempted, true, a.p) {
		a.applySelection(v)
	}
	if d.Error != "" {
		muted(w, d.Error, a.p)
	}
	if d.SchemaError != "" {
		muted(w, d.SchemaError, a.p)
	}
	if d.Saving {
		muted(w, "Updating selected items…", a.p)
	}
	if d.Loading || d.SchemaLoading {
		muted(w, "Loading selected items / field metadata…", a.p)
	}
	w.Row(30).Dynamic(1)
	if enabledButton(w, "Reload remaining items", !d.Saving && !d.Loading, false, a.p) {
		a.reloadSelection(v)
	}
	w.RowScaled(max(80, w.LayoutAvailableHeight()-8)).Dynamic(1)
	if body := w.GroupBegin("selection-editor", nucular.WindowNoHScrollbar); body != nil {
		if !d.Saving && !d.Loading && !s.Attempted {
			fields := d.selectionFields()
			if len(fields) == 0 {
				muted(body, "No supported writable fields are available in the workspace schema.", a.p)
			}
			for _, f := range fields {
				enabled := s.Enabled[f.Name]
				body.Row(28).Dynamic(1)
				if body.CheckboxText("Change "+detailCaption(f), &enabled) {
					s.Enabled[f.Name] = enabled
					d.snapshotRevision++
					s.Rows = nil
				}
				if enabled {
					a.detailProperty(body, d, f)
				}
			}
		}
		spacing, scale := body.WindowStyle().Spacing.Y, body.Master().Style().Scaling
		if len(s.Rows) > 0 {
			title(body, "Reviewed changes", a.p)
			fieldCount := 0
			for _, on := range s.Enabled {
				if on {
					fieldCount++
				}
			}
			stride := int(28*scale) + fieldCount*int(40*scale) + int(50*scale) + (fieldCount+2)*spacing
			start, end := sidebarVisible(body.LayoutNextRowY(), body.Bounds.Y, body.Bounds.Y+body.Bounds.H, stride, len(s.Rows))
			sidebarSkip(body, start, stride, spacing)
			for _, row := range s.Rows[start:end] {
				body.Row(28).Ratio(.8, .2)
				body.Label(row.Before.ID()+" · "+row.Status, "LC")
				if body.ButtonText("Details") {
					a.openText("Bulk edit · "+row.Before.ID(), selectionRowText(d, row))
				}
				names := slices.Sorted(maps.Keys(row.Fields))
				for i := 0; i < fieldCount; i++ {
					body.Row(40).Dynamic(1)
					if i < len(names) {
						name := names[i]
						f, _ := d.schemaField(name)
						body.LabelWrap(detailCaption(f) + ": " + selectionValueLabel(d, name, row.Before) + " → " + selectionValueLabel(d, name, row.Fields))
					} else {
						body.Spacing(1)
					}
				}
				body.Row(50).Dynamic(1)
				body.LabelWrap(row.Message)
			}
			sidebarSkip(body, len(s.Rows)-end, stride, spacing)
		} else {
			title(body, "Selected work items", a.p)
			stride := int(28*scale) + spacing
			start, end := sidebarVisible(body.LayoutNextRowY(), body.Bounds.Y, body.Bounds.Y+body.Bounds.H, stride, len(s.Targets))
			sidebarSkip(body, start, stride, spacing)
			for _, o := range s.Targets[start:end] {
				body.Row(28).Dynamic(1)
				body.Label(o.ID()+" · "+o.String("Name"), "LC")
			}
			sidebarSkip(body, len(s.Targets)-end, stride, spacing)
		}
		body.GroupEnd()
	}
}

func selectionRowText(d *detailView, row selectionEditRow) string {
	var out strings.Builder
	fmt.Fprintf(&out, "%s · %s\n%s\n\n", row.Before.ID(), row.Before.String("Name"), row.Status)
	for _, name := range slices.Sorted(maps.Keys(row.Fields)) {
		f, _ := d.schemaField(name)
		fmt.Fprintf(&out, "%s\n  Before: %s\n  After: %s\n\n", detailCaption(f), selectionValueLabel(d, name, row.Before), selectionValueLabel(d, name, row.Fields))
	}
	out.WriteString(row.Message)
	return out.String()
}
