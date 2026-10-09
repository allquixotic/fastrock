package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
)

const maxPropertyReferences = 10000

type detailCollectionEditor struct {
	loading bool
	err     string
	cancel  context.CancelFunc
}

func editableCollection(f rally.Field) bool {
	return !f.ReadOnly && f.AttributeType == "COLLECTION" && (f.Name == "Tags" || f.Name == "Milestones")
}

func collectionObjects(value any) []rally.Object {
	var objects []rally.Object
	switch values := value.(type) {
	case []rally.Object:
		return values
	case []any:
		for _, v := range values {
			switch o := v.(type) {
			case map[string]any:
				objects = append(objects, rally.Object(o))
			case rally.Object:
				objects = append(objects, o)
			}
		}
	}
	return objects
}

func encodeCollectionRefs(refs []string) string {
	refs = slices.Clone(refs)
	sort.Strings(refs)
	refs = slices.Compact(refs)
	if refs == nil {
		refs = []string{}
	}
	data, _ := json.Marshal(refs)
	return string(data)
}

func decodeCollectionRefs(value string) ([]string, error) {
	var refs []string
	if err := json.Unmarshal([]byte(value), &refs); err != nil {
		return nil, fmt.Errorf("reload this collection before editing it")
	}
	if len(refs) > maxPropertyReferences {
		return nil, fmt.Errorf("too many selected references")
	}
	for _, ref := range refs {
		if !strings.Contains(ref, "/") {
			return nil, fmt.Errorf("select an existing item from the picker")
		}
	}
	return refs, nil
}

func collectionRefsValue(value any) (string, bool) {
	switch v := value.(type) {
	case nil:
		return "[]", true
	case map[string]any:
		count, err := strconv.Atoi(fmt.Sprint(v["Count"]))
		if err == nil && count == 0 {
			return "[]", true
		}
		return "", false
	case []any:
		if len(collectionObjects(v)) != len(v) {
			return "", false
		}
	case []rally.Object:
	default:
		return "", false
	}
	var refs []string
	for _, o := range collectionObjects(value) {
		if o.String("_ref") == "" {
			return "", false
		}
		refs = append(refs, o.String("_ref"))
	}
	return encodeCollectionRefs(refs), true
}

func (d *detailView) rememberCollectionLabels(name string, objects []rally.Object) {
	if d.referenceLabels == nil {
		d.referenceLabels = map[string]string{}
	}
	retained := map[string]bool{}
	if refs, err := decodeCollectionRefs(text(d.Editors[name])); err == nil {
		for _, ref := range refs {
			retained[ref] = true
		}
	}
	for _, o := range objects {
		retained[o.String("_ref")] = true
		d.referenceLabels[name+"\x00"+o.String("_ref")] = referenceLabel(o)
	}
	prefix := name + "\x00"
	for key := range d.referenceLabels {
		if strings.HasPrefix(key, prefix) && !retained[strings.TrimPrefix(key, prefix)] {
			delete(d.referenceLabels, key)
		}
	}
}

func (d *detailView) cancelCollectionEditors() {
	for _, e := range d.collectionEditors {
		if e.cancel != nil {
			e.cancel()
		}
	}
	d.collectionEditors = nil
}

func (d *detailView) collectionsLoading() bool {
	for _, e := range d.collectionEditors {
		if e.loading {
			return true
		}
	}
	return false
}

func readPropertyCollection(ctx context.Context, c *rally.Client, object rally.Object, f rally.Field) ([]rally.Object, error) {
	kind := referenceKinds(nil, f)[0]
	seen := map[string]bool{}
	validate := func(objects []rally.Object) error {
		if len(objects) > maxPropertyReferences {
			return fmt.Errorf("collection is too large to edit safely")
		}
		for _, o := range objects {
			actual, ok := c.ReferenceKind(o.String("_ref"))
			if !ok || actual != kind {
				return fmt.Errorf("collection contains an invalid %s reference", kind)
			}
			if seen[o.String("_ref")] {
				return fmt.Errorf("collection repeated an item; retry")
			}
			seen[o.String("_ref")] = true
		}
		return nil
	}
	if _, complete := collectionRefsValue(object[f.Name]); complete {
		objects := cloneObjects(collectionObjects(object[f.Name]))
		return objects, validate(objects)
	}
	ref := object.Ref(f.Name)
	if ref == "" {
		return nil, fmt.Errorf("collection reference is missing")
	}
	var result []rally.Object
	total := -1
	for start := 1; ; {
		fetch := "ObjectID,Name,FormattedID"
		if kind == "Tag" {
			fetch = "ObjectID,Name"
		}
		page, err := c.Collection(ctx, ref, rally.Query{Start: start, PageSize: 200, Fetch: fetch, Order: "ObjectID ASC"})
		if err != nil {
			return nil, err
		}
		if page.Total > maxPropertyReferences || len(result)+len(page.Results) > maxPropertyReferences {
			return nil, fmt.Errorf("collection is too large to edit safely (%d items)", page.Total)
		}
		if total >= 0 && total != page.Total {
			return nil, fmt.Errorf("collection changed while loading; retry")
		}
		total = page.Total
		if err := validate(page.Results); err != nil {
			return nil, err
		}
		result = append(result, page.Results...)
		if len(result) >= page.Total {
			return result, nil
		}
		if len(page.Results) == 0 {
			return nil, fmt.Errorf("collection ended before all items were loaded")
		}
		start += len(page.Results)
	}
}

func (a *App) loadPropertyCollection(d *detailView, f rally.Field) {
	if !editableCollection(f) || a.rallyClient == nil || d.Saving || d.Loading {
		return
	}
	if d.collectionEditors == nil {
		d.collectionEditors = map[string]*detailCollectionEditor{}
	}
	if old := d.collectionEditors[f.Name]; old != nil && old.loading {
		return
	}
	ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
	e := &detailCollectionEditor{loading: true, cancel: cancel}
	d.collectionEditors[f.Name] = e
	c, original := a.rallyClient, d.Original.Clone()
	a.work(func() {
		defer cancel()
		objects, err := readPropertyCollection(ctx, c, original, f)
		a.post(func() {
			if d.collectionEditors[f.Name] != e {
				return
			}
			e.loading = false
			if a.rallyClient != c {
				e.err = "Rally connection changed. Retry this collection."
				return
			}
			if err != nil {
				e.err = err.Error()
				return
			}
			d.Original[f.Name] = collectionObjectValues(objects)
			d.rememberCollectionLabels(f.Name, objects)
			value, _ := collectionRefsValue(d.Original[f.Name])
			d.Editors[f.Name] = textEditor(value, false)
			d.snapshotRevision++
		})
	}, func() { cancel(); e.loading = false; e.err = errWorkQueueFull.Error() })
}

func collectionObjectValues(objects []rally.Object) []any {
	result := make([]any, 0, len(objects))
	for _, o := range objects {
		result = append(result, map[string]any(o.Clone()))
	}
	return result
}

func (a *App) detailReferenceCollection(w *desktop.Window, d *detailView, f rally.Field) {
	title(w, detailCaption(f), a.p)
	ed := d.Editors[f.Name]
	if ed == nil {
		w.Row(28).Dynamic(2)
		w.Label(fmt.Sprintf("%d items", d.Original.Count(f.Name)), "LC")
		e := d.collectionEditors[f.Name]
		if enabledButton(w, "Edit…", a.rallyClient != nil && !d.Saving && (e == nil || !e.loading), false, a.p) {
			a.loadPropertyCollection(d, f)
		}
		if e != nil {
			if e.loading {
				muted(w, "Loading complete selection…", a.p)
			}
			if e.err != "" {
				muted(w, e.err, a.p)
			}
		}
		return
	}
	refs, err := decodeCollectionRefs(text(ed))
	if err != nil {
		muted(w, err.Error(), a.p)
		return
	}
	w.Row(28).Dynamic(2)
	w.Label(fmt.Sprintf("%d selected", len(refs)), "LC")
	if enabledButton(w, "Add…", a.rallyClient != nil && !d.Saving && len(refs) < maxPropertyReferences, false, a.p) {
		a.openReferencePicker(d, f)
	}
	if len(refs) == 0 {
		return
	}
	scale := w.Master().Style().Scaling
	w.RowScaled(min(len(refs), 4)*int(30*scale) + int(12*scale)).Dynamic(1)
	if list := w.GroupBegin("selected-"+f.Name, desktop.WindowNoHScrollbar); list != nil {
		spacing, stride := list.WindowStyle().Spacing.Y, int(28*scale)+list.WindowStyle().Spacing.Y
		first, last := sidebarVisible(list.LayoutNextRowY(), list.Bounds.Y, list.Bounds.Y+list.Bounds.H, stride, len(refs))
		sidebarSkip(list, first, stride, spacing)
		for _, ref := range refs[first:last] {
			list.Row(28).Ratio(.82, .18)
			label := fallback(d.referenceLabels[f.Name+"\x00"+ref], "Selected item")
			list.Label(cut(label, 32), "LC")
			if list.Input().Mouse.HoveringRect(list.LastWidgetBounds) {
				list.Tooltip(label)
			}
			if enabledButton(list, "×", !d.Saving, false, a.p) {
				setText(ed, encodeCollectionRefs(slices.DeleteFunc(slices.Clone(refs), func(s string) bool { return s == ref })))
				delete(d.referenceLabels, f.Name+"\x00"+ref)
				d.snapshotRevision++
			}
			if list.Input().Mouse.HoveringRect(list.LastWidgetBounds) {
				list.Tooltip("Remove from this work item")
			}
		}
		sidebarSkip(list, len(refs)-last, stride, spacing)
		list.GroupEnd()
	}
}
