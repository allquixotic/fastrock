package ui

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/rally"
)

type detailFieldConflict struct{ Name, Before, Server string }

func (d *detailView) fieldValue(name string) string {
	if r := d.Rich[name]; r != nil {
		return r.html()
	}
	return text(d.Editors[name])
}

func (d *detailView) setFieldValue(name, value string) {
	if old := d.Rich[name]; old != nil {
		presentation := old.presentationState()
		old.doc.DiscardHistory()
		d.Rich[name] = newRichEditor(value)
		d.Rich[name].restorePresentation(presentation)
	} else if ed := d.Editors[name]; ed != nil {
		cursor := position(ed)
		setText(ed, value)
		cursor.apply(ed)
	}
}

// Compare at publication time so typing while the read is in flight is retained.
func (d *detailView) mergeReload(latest rally.Object) {
	before := makeDetail(d.Original, d.Kind, false)
	server := makeDetail(latest, d.Kind, false)
	mergeSchemaEditors(before, d.Fields)
	mergeSchemaEditors(server, d.Fields)
	prior := d.fieldConflicts
	d.fieldConflicts = nil
	for name, local := range d.values() {
		old, next := before.fieldValue(name), server.fieldValue(name)
		if local == old {
			d.setFieldValue(name, next)
		} else if local != next && (old != next || slices.ContainsFunc(prior, func(c detailFieldConflict) bool { return c.Name == name })) {
			d.fieldConflicts = append(d.fieldConflicts, detailFieldConflict{Name: name, Before: old, Server: next})
		}
	}
	sort.Slice(d.fieldConflicts, func(i, j int) bool { return d.fieldConflicts[i].Name < d.fieldConflicts[j].Name })
	d.Original = latest.Clone()
	for _, f := range d.Fields {
		if editableCollection(f) {
			d.rememberCollectionLabels(f.Name, collectionObjects(latest[f.Name]))
		}
	}
	d.cancelCollectionEditors()
	d.Conflict, d.Error = false, ""
	d.snapshotRevision++
	if d.referencePicker != nil {
		d.referencePicker.close()
	}
	clear(d.collectionCounts)
}

func (d *detailView) resolveFieldConflict(name string, useServer bool) {
	i := slices.IndexFunc(d.fieldConflicts, func(c detailFieldConflict) bool { return c.Name == name })
	if i < 0 {
		return
	}
	if useServer {
		d.setFieldValue(name, d.fieldConflicts[i].Server)
	}
	d.fieldConflicts = slices.Delete(d.fieldConflicts, i, i+1)
	d.Error = ""
	d.snapshotRevision++
}

func (a *App) reloadDetail(v *rallyView) {
	if v.Detail != nil && v.Detail.selection != nil {
		a.reloadSelection(v)
		return
	}
	d, client := v.Detail, a.rallyClient
	if d != nil && d.inlineField != "" && d.inlineScope != a.inlineScope() {
		d.Error = "Restore the original Rally connection and scope before reloading this edit."
		return
	}
	if d == nil || d.New || d.Saving || d.Pending || d.Loading || d.SchemaLoading || v.Closed || client == nil {
		return
	}
	if d.collectionsLoading() {
		d.Error = "Wait for selected properties to finish loading before reloading."
		return
	}
	var collections []rally.Field
	for _, f := range d.Fields {
		if editableCollection(f) && d.Editors[f.Name] != nil {
			collections = append(collections, f)
		}
	}
	d.Saving = true
	ref := d.Original.String("_ref")
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	d.cancel = cancel
	a.work(func() {
		defer cancel()
		latest, err := client.Get(ctx, ref)
		if err == nil && !client.SameReference(latest.String("_ref"), ref) {
			err = fmt.Errorf("Rally returned a different work item; local edits are retained")
		}
		for _, f := range collections {
			if err != nil {
				break
			}
			var objects []rally.Object
			objects, err = readPropertyCollection(ctx, client, latest, f)
			if err == nil {
				latest[f.Name] = collectionObjectValues(objects)
			}
		}
		a.post(func() {
			d.Saving = false
			if v.Closed || v.Detail != d {
				return
			}
			if client != a.rallyClient {
				d.Error = "Rally connection changed. Local edits are retained; reconnect before reloading."
				return
			}
			if err != nil {
				d.Error = "Could not reload item: " + err.Error()
				return
			}
			d.mergeReload(latest)
			if d.Tab == "Details" || d.Tab == "More fields" {
				d.Refreshed = time.Now()
			}
			if d.inlineField != "" {
				v.replaceBoardObject(ref, d.Original)
			}
			a.toast = "Reloaded " + latest.ID() + "; local edits retained"
			if d.Tab != "Details" && d.Tab != "More fields" {
				a.loadCollection(d)
			}
		})
	}, func() { cancel(); d.Saving = false; d.Error = errWorkQueueFull.Error() })
}

func (a *App) drawDetailRecovery(w *nucular.Window, v *rallyView, d *detailView) {
	if d.Conflict {
		w.Row(30).Static(160)
		if enabledButton(w, "Reload item", !d.Saving && !d.Pending, false, a.p) {
			a.reloadDetail(v)
		}
	}
	if len(d.fieldConflicts) == 0 {
		return
	}
	muted(w, "These fields changed here and in Rally. Choose which value to keep before saving.", a.p)
	w.Row(210).Dynamic(1)
	if body := w.GroupBegin("changed-rally-fields", nucular.WindowNoHScrollbar); body != nil {
		for _, change := range slices.Clone(d.fieldConflicts) {
			field, _ := d.schemaField(change.Name)
			title(body, detailCaption(field), a.p)
			local, server := d.fieldValue(change.Name), change.Server
			if d.Rich[change.Name] != nil {
				local, server = plainHTML(local), plainHTML(server)
			}
			body.Row(48).Dynamic(2)
			body.LabelWrap("Your edit: " + fallback(cut(local, 240), "(empty)"))
			body.LabelWrap("Rally: " + fallback(cut(server, 240), "(empty)"))
			body.Row(28).Dynamic(1)
			if body.ButtonText("View full comparison") {
				a.openText(detailCaption(field)+" comparison", "Your edit\n\n"+d.fieldValue(change.Name)+"\n\nRally value\n\n"+change.Server+"\n\nPrevious value\n\n"+change.Before)
			}
			body.Row(28).Dynamic(2)
			if enabledButton(body, "Keep my edit", !d.Saving, false, a.p) {
				d.resolveFieldConflict(change.Name, false)
			}
			if enabledButton(body, "Use Rally value", !d.Saving, false, a.p) {
				d.resolveFieldConflict(change.Name, true)
			}
		}
		body.GroupEnd()
	}
}
