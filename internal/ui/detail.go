package ui

import (
	"context"
	"errors"
	"fmt"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"math"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/assistant"
	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/key"
)

type detailView struct {
	Refreshed                time.Time
	inlineField, inlineScope string
	selection                *selectionEdit
	collectionEditors        map[string]*detailCollectionEditor
	Conflict                 bool
	fieldConflicts           []detailFieldConflict
	taskLayout               *detailTaskLayout
	referencePicker          *referencePicker
	referenceLabels          map[string]string
	ownerDefaultEditor       *desktop.TextEditor
	ownerDefaultRevision     uint64
	snapshotRevision         uint64
	cancel                   context.CancelFunc
	collectionCancel         context.CancelFunc
	collectionGeneration     int
	collectionTab            string
	collectionCounts         map[string]int
	collectionLayout         *detailCollectionLayout
	collectionLayoutPending  *detailCollectionLayout
	SchemaLoading            bool
	SchemaError              string
	Pending                  bool
	afterSave                func()
	Original                 rally.Object
	Kind                     string
	New                      bool
	Tab                      string
	Editors                  map[string]*desktop.TextEditor
	Rich                     map[string]*richEditor
	CommentRich              *richEditor
	Fields                   []rally.Field
	States                   []rally.Object
	Items                    []rally.Object
	CollectionLoading        bool
	CollectionMore           bool
	CollectionNext           int
	CollectionStart          int
	CollectionTotal          int
	Loading, Saving          bool
	Error                    string
	titleFont                font.Face
	titleSize                int
}

func makeDetail(o rally.Object, kind string, isNew bool) *detailView {
	if canonical, ok := rally.CanonicalKind(kind); ok {
		kind = canonical
	}
	d := &detailView{Rich: map[string]*richEditor{}, CommentRich: newRichEditor(""), Original: o.Clone(), Kind: kind, New: isNew, Tab: "Details", Editors: map[string]*desktop.TextEditor{}}
	for _, k := range []string{"Name", "Description", "Notes", "AcceptanceCriteria", "BlockedReason", "PlanEstimate", "Estimate", "ToDo", "Actuals", "Priority", "Severity", "FormattedID", "Owner", "Iteration", "Release", "Project", "Feature", "Parent", "State", "ScheduleState", "Blocked", "Ready", "LastVerdict"} {
		if k == "AcceptanceCriteria" && kind != "HierarchicalRequirement" {
			continue
		}
		if slices.Contains(richDetailFields(kind), k) {
			continue
		}
		value := o.String(k)
		if isReferenceField(k) && o.Ref(k) != "" {
			value = o.Ref(k)
		}
		if k == "Blocked" || k == "Ready" {
			value = strconv.FormatBool(o.Bool(k))
		}
		d.Editors[k] = textEditor(value, k == "Description" || k == "Notes" || k == "AcceptanceCriteria")
	}
	for _, key := range richDetailFields(kind) {
		d.Rich[key] = newRichEditor(o.String(key))
	}
	for k := range o {
		if strings.HasPrefix(k, "c_") {
			d.Editors[k] = textEditor(o.String(k), false)
		}
	}
	return d
}
func isReferenceField(k string) bool {
	switch k {
	case "State", "Owner", "Iteration", "Release", "Project", "Feature", "Parent", "WorkProduct", "Requirement", "PortfolioItem":
		return true
	}
	return false
}
func richDetailFields(kind string) []string {
	fields := []string{"Description", "Notes"}
	if kind == "HierarchicalRequirement" {
		fields = append(fields, "AcceptanceCriteria")
	}
	return fields
}
func (a *App) leaveDetail(v *rallyView, next func()) {
	d := v.Detail
	if d != nil && d.Saving {
		a.toast = "Wait for this work item to finish saving"
		return
	}
	if d == nil || !d.dirty() {
		next()
		return
	}
	if a.window == nil {
		return
	}
	a.window.PopupOpen("Unsaved work item", desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(600, 500), false, func(w *desktop.Window) {
		for event := range w.Input().Keyboard.Events() {
			if event.HandleKey(key.CodeEscape, 0) {
				w.Close()
				return
			}
		}
		w.Row(70).Dynamic(1)
		w.LabelWrap("Save changes before leaving this work item?")
		w.Row(30).Dynamic(3)
		if w.ButtonText("Keep editing") {
			w.Close()
		}
		if w.ButtonText("Discard") {
			next()
			w.Close()
		}
		if primary(w, "Save", a.p) {
			d.afterSave = next
			a.saveDetail(v)
			w.Close()
		}
	})
}
func (a *App) disposeDetail(d *detailView) {
	if d == nil {
		return
	}
	if d.cancel != nil {
		d.cancel()
	}
	if d.collectionCancel != nil {
		d.collectionCancel()
	}
	d.collectionGeneration++
	d.cancelCollectionEditors()
	if d.referencePicker != nil {
		d.referencePicker.close()
	}
	d.collectionLayout, d.collectionLayoutPending, d.taskLayout = nil, nil, nil
	for _, r := range d.Rich {
		if r != nil {
			r.doc.DiscardHistory()
		}
	}
	if d.CommentRich != nil {
		d.CommentRich.doc.DiscardHistory()
	}
}
func (a *App) openArtifact(v *rallyView, o rally.Object) {
	if v.PendingCards[o.String("_ref")] {
		a.toast = "Wait for this work item to finish moving"
		return
	}
	a.leaveDetail(v, func() { a.openArtifactNow(v, o, true) })
}
func (a *App) openArtifactNow(v *rallyView, o rally.Object, remember bool) {
	if a.rallyClient == nil || v.Closed {
		return
	}
	if v.Detail != nil {
		if remember && v.Detail.inlineField == "" {
			v.DetailHistory = append(v.DetailHistory, v.Detail.Original.Clone())
			v.historyRevision++
		}
		a.disposeDetail(v.Detail)
	}
	kind := v.Spec.Kind
	if k := o.String("_type"); k != "" {
		kind = k
	}
	if k, ok := rally.CanonicalKind(kind); ok {
		kind = k
	}
	d := makeDetail(o, kind, false)
	d.Loading = true
	d.SchemaLoading = true
	v.Detail = d
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	d.cancel = cancel
	c := a.rallyClient
	workspace := a.prefs.RallyWorkspace
	a.work(func() {
		defer cancel()
		full, err := c.Get(ctx, o.String("_ref"))
		if err != nil {
			a.post(func() {
				if v.Detail == d && !v.Closed {
					d.Loading = false
					d.SchemaLoading = false
					d.Error = err.Error()
				}
			})
			return
		}
		nd := makeDetail(full, kind, false)
		nd.SchemaLoading = true
		nd.cancel = cancel
		a.post(func() {
			if v.Detail == d && !v.Closed {
				nd.Tab = d.Tab
				if nd.Tab == "Details" || nd.Tab == "More fields" {
					nd.Refreshed = time.Now()
				}
				v.Detail = nd
				if nd.Tab != "Details" && nd.Tab != "More fields" {
					a.loadCollection(nd)
				}
			}
		})
		fields, err := c.Fields(ctx, kind, workspace)
		var states []rally.Object
		if err == nil {
			states, err = c.Workflow(ctx, kind, workspace, fields)
		}
		a.post(func() {
			if v.Detail != nd || v.Closed {
				return
			}
			nd.SchemaLoading = false
			if err != nil {
				nd.SchemaError = "Field metadata unavailable: " + err.Error()
				return
			}
			mergeSchemaEditors(nd, fields)
			nd.setStates(states)
		})
	}, func() { d.Loading = false; d.SchemaLoading = false; d.Error = errWorkQueueFull.Error(); cancel() })
}
func (a *App) newArtifact(v *rallyView) {
	a.newArtifactInState(v, "")
}
func (a *App) newArtifactInState(v *rallyView, state string) {
	a.leaveDetail(v, func() {
		a.disposeDetail(v.Detail)
		o := rally.Object{"Project": map[string]any{"_ref": a.prefs.RallyProject}, "Workspace": map[string]any{"_ref": a.prefs.RallyWorkspace}}
		if a.rallyUser != nil {
			o["Owner"] = map[string]any(a.rallyUser.Clone())
		}
		if state != "" {
			if value, ok := v.stateValue(state); ok {
				o[rally.StateField(v.Spec.Kind)] = value
			}
		}
		a.prepareTimeboxNames(v)
		if ref := timeboxDefaultRef(a.iterations, a.prefs.RallyProject, v.Timebox, v.TimeboxName); ref != "" {
			o["Iteration"] = map[string]any{"_ref": ref}
		}
		if ref := timeboxDefaultRef(a.releases, a.prefs.RallyProject, v.ReleaseTimebox, v.ReleaseName); ref != "" {
			o["Release"] = map[string]any{"_ref": ref}
		}
		a.startNewDetail(v, o, v.Spec.Kind)
	})
}

func (a *App) startNewDetail(v *rallyView, o rally.Object, kind string) {
	d := makeDetail(o, kind, true)
	if a.rallyUser == nil {
		d.ownerDefaultEditor = d.Editors["Owner"]
		d.ownerDefaultRevision = d.ownerDefaultEditor.TextRevision()
	}
	v.Detail = d
	d.SchemaLoading = true
	if a.rallyClient == nil {
		d.SchemaError = "Connect Rally to load field metadata"
		d.SchemaLoading = false
		return
	}
	c, workspace := a.rallyClient, a.prefs.RallyWorkspace
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	d.cancel = cancel
	a.work(func() {
		defer cancel()
		fields, err := c.Fields(ctx, d.Kind, workspace)
		var states []rally.Object
		if err == nil {
			states, err = c.Workflow(ctx, d.Kind, workspace, fields)
		}
		a.post(func() {
			if v.Detail != d || v.Closed {
				return
			}
			d.SchemaLoading = false
			if a.rallyClient != c {
				d.SchemaError = "Rally connection changed. Reopen the form to load field metadata."
				return
			}
			if err != nil {
				d.SchemaError = err.Error()
				return
			}
			mergeSchemaEditors(d, fields)
			d.setStates(states)
			if d.Kind == "Task" && text(d.Editors["State"]) == "" {
				if f, ok := d.schemaField("State"); ok && !f.ReadOnly && slices.Contains(f.AllowedValues, "Defined") {
					d.Original["State"] = "Defined"
					setText(d.Editors["State"], "Defined")
				}
			}
		})
	}, func() { d.SchemaLoading = false; d.SchemaError = errWorkQueueFull.Error(); cancel() })
}

func (a *App) drawDetail(w *desktop.Window, v *rallyView) {
	d := v.Detail
	if d.selection != nil {
		a.drawSelectionEditor(w, v)
		return
	}
	w.Row(34).Static(100, max(150, w.LayoutAvailableWidth()-410), 100, 100, 100)
	if w.ButtonText("← Back") {
		a.closeRallyDetail(v, true)
		return
	}
	w.Label(fallback(d.Original.ID(), "Create "+rallyKindLabel(d.Kind)), "LC")
	if w.ButtonText("Open web") {
		if ref := d.Original.String("_ref"); ref != "" {
			kind := strings.ToLower(d.Kind)
			if kind == "hierarchicalrequirement" {
				kind = "userstory"
			}
			a.openURL(a.prefs.RallyEndpoint + "/#/detail/" + kind + "/" + d.Original.String("ObjectID"))
		}
	}
	if !d.Saving && !d.Loading && !d.SchemaLoading && d.SchemaError == "" {
		if primary(w, "Save", a.p) {
			a.saveDetail(v)
		}
	} else {
		w.Label("Loading / saving…", "LC")
	}
	if w.ButtonText("Delete…") && !d.New {
		a.confirm("Delete "+d.Original.ID()+"?", "This deletes the work item from Rally.", func() { a.deleteArtifact(v, d.Original) })
	}
	a.drawDetailTabs(w, d)
	if d.Error != "" {
		w.Row(45).Dynamic(1)
		w.LabelWrap(d.Error)
	}
	if d.SchemaError != "" {
		muted(w, d.SchemaError, a.p)
	}
	a.drawDetailRecovery(w, v, d)
	if d.dirty() {
		muted(w, "Unsaved changes", a.p)
	}
	if d.Loading {
		muted(w, "Loading work item…", a.p)
		return
	}
	if d.SchemaLoading {
		muted(w, "Loading field metadata…", a.p)
	}
	w.Row(max(150, w.LayoutAvailableHeight()-8)).Dynamic(1)
	flags := desktop.WindowNoHScrollbar
	if d.Tab == "Tasks" {
		flags = 0 // Keep every task column accessible at narrow widths.
	}
	if body := w.GroupBegin("detail-body", flags); body != nil {
		if d.Tab == "Details" || d.Tab == "More fields" {
			a.detailFields(body, d)
		} else {
			a.detailCollection(body, v, d)
		}
		body.GroupEnd()
	}
}
func (a *App) field(w *desktop.Window, name string, ed *desktop.TextEditor, multiline bool) {
	if ed == nil {
		return
	}
	switch name {
	case "PlanEstimate":
		name = "Plan estimate"
	case "BlockedReason":
		name = "Blocked reason"
	case "ToDo":
		name = "To do"
	case "AcceptanceCriteria":
		name = "Acceptance criteria"
	}
	title(w, name, a.p)
	height := 30
	if multiline {
		height = 110
	}
	w.Row(height).Dynamic(1)
	ed.Edit(w)
}
func (d *detailView) syncRich() {
	for _, r := range d.Rich {
		if r != nil {
			r.sync()
		}
	}
}
func (d *detailView) dirty() bool {
	if d.inlineField != "" {
		return d.inlineDirty()
	}
	if s := d.selection; s != nil {
		if s.Complete {
			return false
		}
		for _, on := range s.Enabled {
			if on {
				return true
			}
		}
		return false
	}
	if d.CommentRich != nil && strings.TrimSpace(d.CommentRich.html()) != "" {
		return true
	}
	d.syncRich()
	for k, r := range d.Rich {
		if r != nil && r.html() != d.Original.String(k) {
			return true
		}
	}
	for k, e := range d.Editors {
		original := d.originalValue(k)
		if k == "Blocked" || k == "Ready" {
			original = strconv.FormatBool(d.Original.Bool(k))
		}
		if text(e) != original {
			return true
		}
	}
	return false
}
func (d *detailView) changes() (rally.Object, error) {
	return d.changesFor(d.values())
}

// changesFor validates only the supplied fields, shared by full and cell editors.
func (d *detailView) changesFor(values map[string]string) (rally.Object, error) {
	result := rally.Object{}
fields:
	for k, value := range values {
		original := d.originalValue(k)
		if k == "Blocked" || k == "Ready" {
			original = strconv.FormatBool(d.Original.Bool(k))
		}
		// Existing values are a comparison baseline for updates. On creation,
		// populated original fields are defaults that still need to be sent.
		if value == original && (!d.New || d.Original[k] == nil || value == "") {
			continue
		}
		var v any = value
		if k == "PlanEstimate" || k == "Estimate" || k == "ToDo" || k == "Actuals" {
			if strings.TrimSpace(value) == "" {
				v = nil
			} else {
				n, e := strconv.ParseFloat(value, 64)
				if e != nil || n < 0 || math.IsNaN(n) || math.IsInf(n, 0) {
					return nil, fmt.Errorf("%s must be a non-negative number", k)
				}
				v = n
			}
		}
		if k == "Blocked" || k == "Ready" {
			v = value == "true"
		}
		if d.referenceField(k) {
			if value == "" {
				v = nil
			} else if !strings.Contains(value, "/") {
				return nil, fmt.Errorf("select a valid %s reference", k)
			}
		}
		if k == "DisplayColor" && value == "" {
			v = nil
		}
		for _, f := range d.Fields {
			if f.Name == k {
				if f.ReadOnly {
					continue fields
				}
				if len(f.AllowedValues) > 0 && value != "" && !contains(f.AllowedValues, value) {
					return nil, fmt.Errorf("%s must be one of %s", k, strings.Join(f.AllowedValues, ", "))
				}
				switch f.AttributeType {
				case "COLLECTION":
					refs, err := decodeCollectionRefs(value)
					if err != nil {
						return nil, fmt.Errorf("%s: %w", detailCaption(f), err)
					}
					if f.Required && len(refs) == 0 {
						return nil, fmt.Errorf("%s is required", detailCaption(f))
					}
					objects := make([]any, 0, len(refs))
					for _, ref := range refs {
						objects = append(objects, map[string]any{"_ref": ref})
					}
					v = objects
				case "BOOLEAN":
					v = value == "true"
				case "OBJECT":
					if value == "" {
						v = nil
					}
				case "INTEGER", "DECIMAL", "QUANTITY":
					if value == "" {
						v = nil
					} else {
						n, e := strconv.ParseFloat(value, 64)
						if e != nil || math.IsNaN(n) || math.IsInf(n, 0) {
							return nil, fmt.Errorf("%s requires a finite number", k)
						}
						if f.AttributeType == "INTEGER" {
							if _, err := strconv.ParseInt(value, 10, 64); err != nil {
								return nil, fmt.Errorf("%s requires a whole number", k)
							}
						}
						v = n
					}
				}
			}
		}
		if k != "FormattedID" {
			result[k] = v
		}
	}
	if name, exists := values["Name"]; exists && strings.TrimSpace(name) == "" {
		return nil, fmt.Errorf("Name is required")
	}
	for _, f := range d.Fields {
		if f.Required && !f.ReadOnly {
			if value, exists := values[f.Name]; exists && strings.TrimSpace(value) == "" {
				return nil, fmt.Errorf("%s is required", f.DisplayName)
			}
		}
	}
	return result, nil
}
func (d *detailView) values() map[string]string {
	d.syncRich()
	values := map[string]string{}
	for k, e := range d.Editors {
		values[k] = text(e)
	}
	for k, r := range d.Rich {
		if r != nil {
			values[k] = r.html()
		}
	}
	return values
}

// A successful mutation may return only a subset of the object. Retain the
// acknowledged values for omitted fields; explicit server values, including
// null, take precedence. A same-size collection summary is only metadata, so
// retain its acknowledged membership. Keep old revision stamps when none are returned
// so a subsequent save still checks the reviewed revision rather than bypassing it.
func (d *detailView) savedSnapshot(before, saved, changes rally.Object) rally.Object {
	result := before.Clone()
	for name, value := range changes {
		if ref, ok := value.(string); ok && d.referenceField(name) && ref != "" {
			if ref == before.Ref(name) {
				continue
			} else {
				reference := map[string]any{"_ref": ref}
				if label := d.referenceLabels[name+"\x00"+ref]; label != "" {
					reference["_refObjectName"] = label
				}
				result[name] = reference
			}
		} else {
			result[name] = value
		}
	}
	for name, value := range saved.Clone() {
		if f, ok := d.schemaField(name); ok && editableCollection(f) {
			if _, complete := collectionRefsValue(value); !complete {
				if known, ready := collectionRefsValue(result[name]); ready {
					if summary, ok := value.(map[string]any); ok {
						if count, err := strconv.Atoi(fmt.Sprint(summary["Count"])); err == nil {
							if refs, err := decodeCollectionRefs(known); err == nil && len(refs) == count {
								continue
							}
						}
					}
				}
			}
		}
		result[name] = value
	}
	return result
}

func (d *detailView) acceptSave(saved rally.Object, submitted map[string]string) {
	d.syncRich()
	baseline := makeDetail(saved, d.Kind, false)
	mergeSchemaEditors(baseline, d.Fields)
	for k, value := range d.values() {
		if value != submitted[k] {
			continue
		}
		if next := baseline.Rich[k]; next != nil {
			if previous := d.Rich[k]; previous != nil {
				previous.doc.DiscardHistory()
			}
			d.Rich[k] = next
		} else if next := baseline.Editors[k]; next != nil && d.Editors[k] != nil {
			setText(d.Editors[k], text(next))
		}
	}
	d.Original = saved.Clone()
	d.snapshotRevision++
	d.New = false
	d.Error = ""
	d.Conflict = false
	for _, f := range d.Fields {
		if editableCollection(f) && d.Editors[f.Name] != nil {
			if _, complete := collectionRefsValue(saved[f.Name]); !complete {
				d.Conflict = true
				d.Error = "Rally returned a different collection size. Reload item to compare before saving again."
			}
		}
	}
}
func (a *App) saveDetail(v *rallyView) {
	if v.Detail != nil && v.Detail.selection != nil {
		_ = a.prepareSelection(v)
		return
	}
	d := v.Detail
	if d == nil || d.Loading || d.Saving || d.Pending || d.SchemaLoading || d.SchemaError != "" || v.Closed || a.rallyClient == nil {
		return
	}
	if d.inlineField != "" && (v.Mutating || len(v.PendingCards) > 0) {
		d.Error = "Wait for pending Rally updates before saving this edit."
		return
	}
	if d.collectionsLoading() {
		d.Error = "Wait for the selected properties to finish loading before saving."
		return
	}
	if len(d.fieldConflicts) > 0 || d.Conflict {
		d.Error = "Reload and review the changed fields before saving. Your edits are retained."
		return
	}
	var fields rally.Object
	var err error
	if d.inlineField != "" {
		fields, err = a.inlineChanges(d)
	} else {
		fields, err = d.changes()
	}
	if err != nil {
		d.Error = err.Error()
		return
	}
	for _, f := range d.Fields {
		if !editableCollection(f) || fields[f.Name] == nil {
			continue
		}
		for _, o := range collectionObjects(fields[f.Name]) {
			kind, valid := a.rallyClient.ReferenceKind(o.String("_ref"))
			if !valid || kind != referenceKinds(d, f)[0] {
				d.Error = "Select a valid " + detailCaption(f) + " item from this Rally connection."
				return
			}
		}
	}
	if len(fields) == 0 && !d.New {
		if d.CommentRich != nil && strings.TrimSpace(d.CommentRich.html()) != "" {
			a.addComment(v, d)
			return
		}
		if next := d.afterSave; next != nil {
			d.afterSave = nil
			next()
		} else {
			a.toast = "No changes to save"
			a.finishInline(v, d)
		}
		return
	}
	submitted := d.values()
	submittedComment := d.CommentRich.html()
	original := d.Original.Clone()
	isNew, kind := d.New, d.Kind
	if isNew {
		fields["Name"] = text(d.Editors["Name"])
		if value := text(d.Editors["Project"]); value != "" {
			fields["Project"] = value
		}
		if ref := original.Ref("Workspace"); ref != "" {
			fields["Workspace"] = ref
		}
	}
	d.Saving = true
	c := a.rallyClient
	a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		var saved rally.Object
		var err error
		if isNew {
			saved, err = c.Create(ctx, kind, fields)
		} else {
			saved, err = c.UpdateIfUnchanged(ctx, original, kind, fields)
		}
		a.post(func() {
			d.Saving = false
			if v.Closed || v.Detail != d {
				return
			}
			if c != a.rallyClient {
				d.Error = "Rally connection changed. Reload to confirm whether this item was saved. Local edits are retained."
				if isNew {
					d.Error = "The connection changed during creation. Check the previous Rally connection before retrying; your draft is retained."
				}
				return
			}
			if err != nil {
				d.Error = err.Error()
				var conflict *rally.ConflictError
				var api *rally.APIError
				d.Conflict = !isNew && (errors.As(err, &conflict) || errors.As(err, &api) && (api.Status == 409 || api.Status == 412))
				d.snapshotRevision++
				return
			}
			saved = d.savedSnapshot(original, saved, fields)
			d.acceptSave(saved, submitted)
			if d.inlineField != "" {
				v.replaceBoardObject(saved.String("_ref"), saved)
				if v.Selected[saved.String("_ref")] {
					v.selectItem(saved, true)
				}
			}
			a.toast = "Saved " + saved.ID()
			a.refreshRallyItems(v)
			if d.CommentRich != nil && strings.TrimSpace(submittedComment) != "" && d.CommentRich.html() == submittedComment {
				a.addComment(v, d)
				return
			}
			if next := d.afterSave; next != nil && !d.dirty() {
				d.afterSave = nil
				next()
			} else {
				a.finishInline(v, d)
			}
		})
	}, func() { d.Saving = false; d.Error = errWorkQueueFull.Error() })
}
func (a *App) addComment(v *rallyView, d *detailView) {
	if d.Pending || d.New || a.rallyClient == nil {
		return
	}
	comment := d.CommentRich.html()
	if strings.TrimSpace(comment) == "" {
		return
	}
	d.Pending = true
	c := a.rallyClient
	ref := d.Original.String("_ref")
	workspace := a.prefs.RallyWorkspace
	a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		_, err := c.Create(ctx, "ConversationPost", rally.Object{"Artifact": ref, "Text": comment, "Workspace": workspace})
		a.post(func() {
			d.Pending = false
			if v.Closed || v.Detail != d {
				return
			}
			if err != nil {
				d.Error = err.Error()
				return
			}
			if d.CommentRich.html() == comment {
				d.CommentRich = newRichEditor("")
			}
			a.loadCollection(d)
			if next := d.afterSave; next != nil && !d.dirty() {
				d.afterSave = nil
				next()
			}
		})
	}, func() { d.Pending = false; d.Error = errWorkQueueFull.Error() })
}
func (a *App) loadCollection(d *detailView) { a.loadCollectionPage(d, false) }
func (a *App) loadCollectionPage(d *detailView, more bool) {
	start := 1
	if more {
		start = max(1, d.CollectionNext)
	}
	a.loadCollectionAt(d, start)
}
func (a *App) loadCollectionAt(d *detailView, start int) {
	if d.New || a.rallyClient == nil {
		return
	}
	if d.collectionCancel != nil {
		d.collectionCancel()
	}
	d.collectionGeneration++
	generation := d.collectionGeneration
	c, tab := a.rallyClient, d.Tab
	ref := d.Original.String("_ref")
	collectionRef := d.Original.Ref(detailCollectionField(d.Kind, tab))
	d.CollectionLoading = true
	d.Error = ""
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	d.collectionCancel = cancel
	scope := a.prefs.RallyWorkspace
	a.work(func() {
		defer cancel()
		q := rally.Query{PageSize: 128, Start: start, Workspace: scope}
		if tab == "Tasks" {
			q.Fetch = "ObjectID,FormattedID,Name,State,Estimate,ToDo,Actuals,Owner,WorkProduct,LastUpdateDate,VersionId"
		}
		var page rally.Page
		var err error
		switch tab {
		case "Discussions":
			q.Expression = rally.Eq("Artifact", ref)
			q.Order = "CreationDate DESC"
			page, err = c.Query(ctx, "ConversationPost", q)
		case "Revisions":
			var history rally.Object
			history, err = c.Get(ctx, collectionRef)
			if err == nil {
				page, err = c.Collection(ctx, history.Ref("Revisions"), q)
			}
		default:
			if collectionRef != "" {
				page, err = c.Collection(ctx, collectionRef, q)
			}
		}
		a.post(func() {
			if d.collectionGeneration != generation || d.Tab != tab {
				return
			}
			d.CollectionLoading = false
			d.collectionCancel = nil
			if a.rallyClient != c {
				d.Error = "Rally connection changed; reload this collection"
				return
			}
			if err != nil {
				d.Error = err.Error()
				return
			}
			d.Items = page.Results
			d.Refreshed = time.Now()
			d.collectionTab = tab
			d.collectionLayout = nil
			d.collectionLayoutPending = nil
			d.taskLayout = nil
			if d.collectionCounts == nil {
				d.collectionCounts = map[string]int{}
			}
			d.collectionCounts[tab] = page.Total
			d.CollectionStart = start
			d.CollectionTotal = page.Total
			d.CollectionNext = start + len(page.Results)
			d.CollectionMore = d.CollectionNext <= page.Total && len(page.Results) > 0
		})
	}, func() { d.CollectionLoading = false; d.Error = errWorkQueueFull.Error(); cancel() })
}
func (a *App) detailCollection(w *desktop.Window, v *rallyView, d *detailView) {
	if d.Tab == "Discussions" {
		d.CommentRich.ensureEditor()
		d.CommentRich.editor.Placeholder = "Add a comment..."
		a.richField(w, "Add to discussion", d.CommentRich, 140)
		w.Row(28).Static(140)
		if primary(w, "Add comment", a.p) && strings.TrimSpace(string(d.CommentRich.doc.Text)) != "" {
			a.addComment(v, d)
		}
	}
	if d.Tab == "Attachments" {
		w.Row(30).Static(180)
		if w.ButtonText("Add attachment…") && !d.Pending {
			a.choosePath(false, false, func(path string) {
				if d.Pending || v.Detail != d || v.Closed || a.rallyClient == nil {
					return
				}
				d.Pending = true
				c := a.rallyClient
				a.writeWork(func() {
					var err error
					info, e := os.Stat(path)
					if e != nil {
						err = e
					} else if info.Size() > 5<<20 {
						err = fmt.Errorf("Attachment exceeds 5 MiB")
					} else {
						data, e := os.ReadFile(path)
						if e != nil {
							err = e
						} else {
							ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
							defer cancel()
							_, err = c.Upload(ctx, d.Original.String("_ref"), filepath.Base(path), http.DetectContentType(data), data)
						}
					}
					a.post(func() {
						d.Pending = false
						if v.Detail != d || v.Closed {
							return
						}
						if err != nil {
							d.Error = err.Error()
						} else {
							a.loadCollection(d)
						}
					})
				}, func() { d.Pending = false; d.Error = errWorkQueueFull.Error() })
			})
		}
	}
	if d.Tab == "Tasks" {
		w.Row(28).Static(140)
		if enabledButton(w, "Add task…", !d.Pending && !d.CollectionLoading && a.rallyClient != nil, false, a.p) {
			a.newDetailTask(v, d)
		}
		if d.Pending {
			muted(w, "Removing task…", a.p)
		}
	}
	items := d.Items
	if d.collectionTab != "" && d.collectionTab != d.Tab {
		items = nil
	}
	if d.Tab == "Tasks" {
		a.drawDetailTasks(w, v, d, items)
	} else if d.Tab == "Discussions" || d.Tab == "Revisions" {
		a.drawCollectionText(w, v, d, items)
	} else {
		spacing := w.WindowStyle().Spacing.Y
		stride := int(33*w.Master().Style().Scaling) + spacing
		first, last := sidebarVisible(w.LayoutNextRowY(), w.Bounds.Y, w.Bounds.Y+w.Bounds.H, stride, len(items))
		sidebarSkip(w, first, stride, spacing)
		for _, o := range items[first:last] {
			w.Row(33).Dynamic(1)
			if w.ButtonText(o.ID() + "  " + o.String("Name")) {
				if d.Tab == "Attachments" {
					a.downloadAttachment(o)
				} else {
					a.openArtifact(v, o)
				}
			}
		}
		sidebarSkip(w, len(items)-last, stride, spacing)
	}
	if d.CollectionLoading {
		muted(w, "Loading…", a.p)
	}
	if !d.CollectionLoading && (d.collectionTab == "" || d.collectionTab == d.Tab) && (d.CollectionMore || d.CollectionStart > 1) {
		w.Row(30).Dynamic(3)
		if d.CollectionStart > 1 && w.ButtonText("Previous page") {
			a.loadCollectionAt(d, max(1, d.CollectionStart-128))
		} else {
			w.Spacing(1)
		}
		w.Label(fmt.Sprintf("%d–%d of %d", d.CollectionStart, d.CollectionStart+len(d.Items)-1, d.CollectionTotal), "LC")
		if d.CollectionMore && w.ButtonText("Next page") {
			a.loadCollectionPage(d, true)
		} else {
			w.Spacing(1)
		}
	}
	if len(items) == 0 && !d.CollectionLoading {
		muted(w, "No "+strings.ToLower(d.Tab)+" on this work item.", a.p)
	}
}
func (a *App) selectionPlan(v *rallyView, operation string, fields rally.Object) assistant.Plan {
	p := assistant.Plan{Summary: "Selected work items"}
	refs := make([]string, 0, len(v.SelectedItems))
	for ref := range v.SelectedItems {
		refs = append(refs, ref)
	}
	sort.Strings(refs)
	for _, ref := range refs {
		o := v.SelectedItems[ref]
		if v.Selected[ref] {
			p.Changes = append(p.Changes, assistant.Change{Operation: operation, Kind: v.objectKind(o), Ref: ref, Fields: fields.Clone(), Before: o.Clone()})
		}
	}
	return p
}
func (a *App) deleteArtifact(v *rallyView, o rally.Object) {
	a.applyPlan(v, assistant.Plan{Summary: "Delete " + o.ID(), Changes: []assistant.Change{{Operation: "delete", Kind: v.objectKind(o), Ref: o.String("_ref"), Before: o.Clone()}}})
}
func planResultVerb(p assistant.Plan) string {
	if len(p.Changes) == 0 {
		return "Updated"
	}
	operation := p.Changes[0].Operation
	for _, change := range p.Changes {
		if change.Operation != operation {
			return "Changed"
		}
	}
	switch operation {
	case "delete":
		return "Deleted"
	case "create":
		return "Created"
	default:
		return "Updated"
	}
}
func (a *App) applyPlan(v *rallyView, p assistant.Plan) {
	c := a.rallyClient
	if c == nil || v.Mutating {
		return
	}
	if len(v.PendingCards) > 0 {
		a.toast = "Wait for board moves to finish before changing these items"
		return
	}
	v.Mutating = true
	a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 2*time.Minute)
		defer cancel()
		n, e := assistant.Apply(ctx, c, p)
		a.post(func() {
			v.Mutating = false
			if e != nil {
				a.toast = fmt.Sprintf("%s %d of %d work items: %v. Refresh before retrying.", planResultVerb(p), n, len(p.Changes), e)
			} else {
				a.toast = fmt.Sprintf("%s %d work items", planResultVerb(p), n)
				if n == 1 && len(p.Changes) == 1 {
					a.toast = planResultVerb(p) + " " + fallback(p.Changes[0].Before.ID(), "work item")
				}
				clear(v.Selected)
				clear(v.SelectedItems)
				v.selectionRevision++
				if len(p.Changes) == 1 && p.Changes[0].Operation == "delete" && v.Detail != nil && v.Detail.Original.String("_ref") == p.Changes[0].Ref {
					a.disposeDetail(v.Detail)
					v.Detail = nil
				}
			}
			a.refreshRallyItems(v)
		})
	}, func() { v.Mutating = false })
}

func mergeSchemaEditors(d *detailView, fields []rally.Field) {
	d.snapshotRevision++
	d.Fields = fields
	for _, f := range fields {
		if editableCollection(f) {
			if value, complete := collectionRefsValue(d.Original[f.Name]); complete && d.Editors[f.Name] == nil {
				d.Editors[f.Name] = textEditor(value, false)
				d.rememberCollectionLabels(f.Name, collectionObjects(d.Original[f.Name]))
			}
			continue
		}
		if f.AttributeType == "OBJECT" && d.Editors[f.Name] != nil {
			ref := d.Original.Ref(f.Name)
			if ref != "" && text(d.Editors[f.Name]) == d.Original.String(f.Name) && text(d.Editors[f.Name]) != ref {
				setText(d.Editors[f.Name], ref)
			}
		}
		if !f.ReadOnly && f.AttributeType == "TEXT" {
			if d.Rich[f.Name] == nil {
				value := d.Original.String(f.Name)
				if ed := d.Editors[f.Name]; ed != nil {
					value = text(ed)
				}
				d.Rich[f.Name] = newRichEditor(value)
			}
			delete(d.Editors, f.Name)
			continue
		}
		if f.ReadOnly || f.AttributeType == "COLLECTION" || d.Editors[f.Name] != nil {
			continue
		}
		value := d.originalValue(f.Name)
		if f.AttributeType == "OBJECT" {
			value = d.Original.Ref(f.Name)
		}
		d.Editors[f.Name] = textEditor(value, f.AttributeType == "TEXT")
	}
}

func (d *detailView) setStates(states []rally.Object) {
	d.States = states
	d.snapshotRevision++
}
