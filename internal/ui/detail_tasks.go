package ui

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/allquixotic/fastrock/internal/rally"
)

type detailTaskRow struct {
	ID, Name tableLinkLayout
	Height   int
}

type detailTaskLayout struct {
	Source              *rally.Object
	Length, Width       int
	Spacing, Generation int
	Scale               float64
	Face                font.Face
	Widths              []int
	Rows                []detailTaskRow
	Offsets             []int
}

func (d *detailView) taskTableLayout(w *nucular.Window, items []rally.Object) *detailTaskLayout {
	var source *rally.Object
	if len(items) > 0 {
		source = &items[0]
	}
	scale, face := w.Master().Style().Scaling, w.Master().Style().Font
	width, spacing := w.LayoutAvailableWidth(), w.WindowStyle().Spacing.Y
	l := d.taskLayout
	if l != nil && l.Source == source && l.Length == len(items) && l.Width == width && l.Spacing == spacing && l.Generation == d.collectionGeneration && l.Scale == scale && l.Face == face {
		return l
	}
	l = &detailTaskLayout{Source: source, Length: len(items), Width: width, Spacing: spacing, Generation: d.collectionGeneration, Scale: scale, Face: face, Offsets: []int{0}}
	for _, size := range []int{90, 260, 110, 80, 80, 80, 150, 40} {
		l.Widths = append(l.Widths, int(float64(size)*scale))
	}
	used := (len(l.Widths) - 1) * w.WindowStyle().Spacing.X
	for _, size := range l.Widths {
		used += size
	}
	l.Widths[1] += max(0, width-used)
	padding := int(6 * scale)
	for _, item := range items {
		row := detailTaskRow{Height: int(33 * scale)}
		row.ID.prepare(fallback(item.ID(), "—"), l.Widths[0]-2*padding, face)
		row.Name.prepare(fallback(item.String("Name"), "Unnamed task"), l.Widths[1]-2*padding, face)
		row.Height = max(row.Height, max(len(row.ID.lines), len(row.Name.lines))*face.Metrics().Height.Ceil()+2*padding)
		l.Rows = append(l.Rows, row)
		l.Offsets = append(l.Offsets, l.Offsets[len(l.Offsets)-1]+row.Height+spacing)
	}
	d.taskLayout = l
	return l
}

func (a *App) drawDetailTasks(w *nucular.Window, v *rallyView, d *detailView, items []rally.Object) {
	l := d.taskTableLayout(w, items)
	w.Row(30).StaticScaled(l.Widths...)
	for _, heading := range []string{"ID", "Name", "State", "Estimate", "To Do", "Actuals", "Owner", ""} {
		w.Label(heading, "LC")
	}
	top, clip := w.LayoutNextRowY(), w.Commands().Clip
	first := sort.Search(len(items), func(i int) bool { return top+l.Offsets[i+1] > clip.Y })
	last := max(first, sort.Search(len(items), func(i int) bool { return top+l.Offsets[i] >= clip.Y+clip.H }))
	collectionSkip(w, l.Offsets[first], l.Spacing)
	for i := first; i < last; i++ {
		item, row := items[i], &l.Rows[i]
		w.RowScaled(row.Height).StaticScaled(l.Widths...)
		open := tableLink(w, &row.ID, int(6*l.Scale), a.p)
		open = tableLink(w, &row.Name, int(6*l.Scale), a.p) || open
		w.LabelColored(item.String("State"), "LC", stateColor(item.String("State"), a.p))
		for _, field := range []string{"Estimate", "ToDo", "Actuals"} {
			w.Label(fallback(item.String(field), "—"), "RC")
		}
		owner := item.String("Owner")
		if nested, ok := item["Owner"].(map[string]any); ok {
			owner = referenceLabel(rally.Object(nested))
		}
		w.Label(cut(fallback(owner, "Unassigned"), 28), "LC")
		if w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
			w.Tooltip(fallback(owner, "Unassigned"))
		}
		if enabledButton(w, "×", !d.Pending && !d.CollectionLoading && a.rallyClient != nil, false, a.p) {
			selected := item.Clone()
			a.confirm("Delete task "+selected.ID()+"?", "Permanently delete this task from Rally: "+selected.String("Name"), func() { a.deleteDetailTask(v, d, selected) })
		}
		if w.Input().Mouse.HoveringRect(w.LastWidgetBounds) {
			w.Tooltip("Delete this task")
		}
		if open {
			selected := item.Clone()
			selected["_type"] = "Task"
			a.openArtifact(v, selected)
			return
		}
	}
	collectionSkip(w, l.Offsets[len(items)]-l.Offsets[last], l.Spacing)
}

func (a *App) newDetailTask(v *rallyView, parent *detailView) {
	if v.Detail != parent || v.Closed || parent.Pending || parent.Saving || parent.Loading || parent.New || a.rallyClient == nil || parent.Tab != "Tasks" {
		return
	}
	a.leaveDetail(v, func() {
		if v.Detail != parent || v.Closed {
			return
		}
		object := rally.Object{"WorkProduct": map[string]any{"_ref": parent.Original.String("_ref"), "_refObjectName": parent.Original.String("Name")}}
		for _, field := range []string{"Project", "Workspace"} {
			if ref := parent.Original.Ref(field); ref != "" {
				object[field] = map[string]any{"_ref": ref, "_refObjectName": parent.Original.String(field)}
			}
		}
		if object.Ref("Workspace") == "" && a.prefs.RallyWorkspace != "" {
			object["Workspace"] = map[string]any{"_ref": a.prefs.RallyWorkspace}
		}
		if a.rallyUser != nil {
			object["Owner"] = map[string]any(a.rallyUser.Clone())
		}
		v.DetailHistory = append(v.DetailHistory, parent.Original.Clone())
		v.historyRevision++
		a.disposeDetail(parent)
		a.startNewDetail(v, object, "Task")
	})
}

func (a *App) deleteDetailTask(v *rallyView, d *detailView, item rally.Object) {
	if v.Detail != d || v.Closed || d.Pending || d.Saving || d.Loading || d.CollectionLoading || d.New || a.rallyClient == nil || d.Tab != "Tasks" {
		return
	}
	c, ref, parentRef := a.rallyClient, item.String("_ref"), d.Original.String("_ref")
	kind, valid := c.ReferenceKind(ref)
	if !valid || kind != "Task" || !slices.ContainsFunc(d.Items, func(o rally.Object) bool { return o.String("_ref") == ref }) {
		d.Error = "This task is no longer in the displayed collection. Refresh before deleting it."
		return
	}
	item = item.Clone()
	d.Pending = true
	a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		current, err := c.Get(ctx, ref)
		if err == nil {
			if current.Ref("WorkProduct") != parentRef {
				err = fmt.Errorf("task moved to another work item; refresh before deleting it")
			}
			for _, field := range []string{"LastUpdateDate", "VersionId"} {
				if value := item.String(field); value != "" && value != current.String(field) {
					err = fmt.Errorf("task changed on Rally; refresh before deleting it")
				}
			}
		}
		if err == nil {
			err = c.Delete(ctx, ref)
		}
		a.post(func() {
			d.Pending = false
			if v.Detail != d || v.Closed {
				return
			}
			if a.rallyClient != c {
				d.Error = "Rally connection changed. Refresh to confirm whether the task was deleted."
				return
			}
			if err != nil {
				d.Error = "Task could not be deleted: " + err.Error()
				return
			}
			a.toast = "Deleted " + fallback(item.ID(), "task")
			delete(d.collectionCounts, "Tasks")
			if d.Tab == "Tasks" {
				a.loadCollectionAt(d, max(1, d.CollectionStart))
			}
		})
	}, func() { d.Pending = false; d.Error = errWorkQueueFull.Error() })
}
