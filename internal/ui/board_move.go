package ui

import (
	"context"
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
)

type boardPlacement struct {
	Ref      string
	Position rally.RankPosition
}
type boardWrite struct {
	Before, Preview, Changes rally.Object
	Position, Previous       *rally.RankPosition
	Label                    string
	Undo                     bool
}
type boardUndo struct {
	After, Fields rally.Object
	Position      *rally.RankPosition
	Label         string
}
type boardDrop struct {
	State  string
	Group  boardGroup
	Target string
	Below  bool
}

func boardGroupIdentity(o rally.Object, field string) (key, name string, value any) {
	if field == "" || field == "None" {
		return "", "", nil
	}
	value = o[field]
	name = fallback(o.String(field), "Unassigned")
	if value == nil || o.String(field) == "" {
		return "", "Unassigned", nil
	}
	if ref := o.Ref(field); strings.HasPrefix(ref, "/") || strings.HasPrefix(ref, "https://") {
		return "ref:" + ref, name, value
	}
	return fmt.Sprintf("%T:%v", value, value), name, value
}

func boardGroupChange(v *rallyView, group boardGroup) (any, any, error) {
	field := v.Group
	for _, f := range v.Fields {
		if f.Name != field {
			continue
		}
		if f.ReadOnly || f.AttributeType == "COLLECTION" || f.Required && group.value == nil {
			return nil, nil, fmt.Errorf("%s cannot be changed by a board drop", fallback(f.DisplayName, field))
		}
	}
	value := group.value
	switch field {
	case "Owner", "Iteration", "Release", "Feature", "Parent", "Project":
		if value == nil {
			return nil, nil, nil
		}
		o := rally.Object{field: value}
		ref := o.Ref(field)
		if !strings.HasPrefix(ref, "/") && !strings.HasPrefix(ref, "https://") {
			return nil, nil, fmt.Errorf("reload the %s reference before moving this item", field)
		}
		return ref, value, nil
	default:
		for _, f := range v.Fields {
			if f.Name == field && !f.ReadOnly && f.AttributeType != "OBJECT" && f.AttributeType != "COLLECTION" {
				return value, value, nil
			}
		}
	}
	return nil, nil, fmt.Errorf("moving between %s groups is unavailable for this field", field)
}

func (a *App) dropBoardCard(v *rallyView, c *boardCard, drop boardDrop) {
	value, ok := v.objectStateValue(c.object, drop.State)
	if !ok {
		a.report(fmt.Errorf("state metadata is unavailable; reload before moving this item"))
		return
	}
	changes := rally.Object{}
	preview := c.object.Clone()
	field := v.stateField()
	if c.state != drop.State {
		changes[field] = value
		preview[field] = value
	}
	if v.Group != "" && v.Group != "None" && v.Group != field {
		key, _, _ := boardGroupIdentity(c.object, v.Group)
		if key != drop.Group.key {
			metadataView := *v
			metadataView.Fields = v.metadataFor(c.object).Fields
			api, display, err := boardGroupChange(&metadataView, drop.Group)
			if err != nil {
				a.report(err)
				return
			}
			changes[v.Group] = api
			preview[v.Group] = display
		}
	}
	var position *rally.RankPosition
	if drop.Target != "" && drop.Target != c.ref {
		position = &rally.RankPosition{Ref: drop.Target, Below: drop.Below}
	}
	if len(changes) == 0 && position == nil {
		return
	}
	a.startBoardWrite(v, &boardWrite{Before: c.object.Clone(), Preview: preview, Changes: changes, Position: position, Label: "Moved " + c.id})
}

func priorBoardPosition(v *rallyView, ref string) *rally.RankPosition {
	items := slices.Clone(v.Items)
	sort.SliceStable(items, func(i, j int) bool { return compareRally(items[i], items[j], "Rank", v.Fields) < 0 })
	for i, o := range items {
		if o.String("_ref") == ref {
			if i+1 < len(items) {
				return &rally.RankPosition{Ref: items[i+1].String("_ref")}
			}
			if i > 0 {
				return &rally.RankPosition{Ref: items[i-1].String("_ref"), Below: true}
			}
		}
	}
	return nil
}
func (v *rallyView) replaceBoardObject(ref string, o rally.Object) {
	for i, old := range v.Items {
		if old.String("_ref") == ref {
			v.Items = slices.Clone(v.Items)
			v.Items[i] = o.Clone()
			break
		}
	}
	v.Generation++
	v.filterSource, v.cardSource = nil, nil
	v.boardPrepared = false
}
func (a *App) boardWriteBlocked(v *rallyView) bool {
	if v == nil || v.Closed || a.rallyClient == nil || v.Mutating {
		return true
	}
	for id, view := range a.rallyViews {
		if view == v && (a.popping[id] || a.windowReturn != nil || a.transferPending && a.incomingTab == id) {
			return true
		}
	}
	return false
}
func (a *App) startBoardWrite(v *rallyView, op *boardWrite) {
	ref := op.Before.String("_ref")
	if a.boardWriteBlocked(v) || v.PendingCards[ref] || op.Position != nil && v.PendingCards[op.Position.Ref] {
		a.toast = "Wait for the current work item operation to finish"
		return
	}
	kind, valid := a.rallyClient.ReferenceKind(ref)
	if !valid || kind != v.objectKind(op.Before) {
		a.report(fmt.Errorf("reload this work item before moving it; its type or reference is invalid"))
		return
	}
	if op.Before.String("LastUpdateDate") == "" && op.Before.String("VersionId") == "" {
		a.report(fmt.Errorf("reload this work item before moving it; its revision is unavailable"))
		return
	}
	// Freeze the existing read before publishing the optimistic object. A refresh
	// requested during the write resumes after all current writes settle.
	if v.cancel != nil {
		v.cancel()
		v.cancel = nil
		v.boardRefreshQueued = true
	}
	v.Loading = false
	if v.PendingCards == nil {
		v.PendingCards = map[string]bool{}
	}
	if v.boardWrites == nil {
		v.boardWrites = map[string]*boardWrite{}
	}
	v.PendingCards[ref] = true
	v.boardWrites[ref] = op
	op.Previous = priorBoardPosition(v, ref)
	if op.Position != nil {
		v.Sort, v.Descending = "Rank", false
		v.boardPlacements = append(v.boardPlacements, boardPlacement{ref, *op.Position})
	}
	v.boardUndo = nil
	v.replaceBoardObject(ref, op.Preview)
	client := a.rallyClient
	finish := func(result rally.Object, err error) {
		if v.boardWrites[ref] != op {
			return
		}
		delete(v.boardWrites, ref)
		delete(v.PendingCards, ref)
		if v.Closed {
			return
		}
		if a.rallyClient != client {
			err = fmt.Errorf("Rally connection changed while the move was pending")
		}
		if err != nil {
			v.replaceBoardObject(ref, op.Before)
			v.boardPlacements = slices.DeleteFunc(v.boardPlacements, func(p boardPlacement) bool { return p.Ref == ref })
			a.report(fmt.Errorf("move not confirmed; card restored locally: %w. Refresh to check Rally", err))
		} else {
			updated := op.Preview.Clone()
			for k, value := range result {
				updated[k] = value
			}
			v.replaceBoardObject(ref, updated)
			if v.Selected[ref] {
				v.SelectedItems[ref] = updated.Clone()
				v.selectionRevision++
			}
			if !op.Undo && (result.String("LastUpdateDate") != "" || result.String("VersionId") != "") {
				fields := rally.Object{}
				for k := range op.Changes {
					fields[k] = op.Before[k]
				}
				u := &boardUndo{After: updated.Clone(), Fields: fields, Label: op.Label}
				if op.Position != nil {
					u.Position = op.Previous
				}
				v.boardUndo = u
				a.actionNotice(op.Label, "Undo", func() { a.undoBoardWrite(v, u) })
			} else {
				a.toast = op.Label
			}
		}
		// Only successful refreshes retire committed placement hints.
		if len(v.boardPlacements) > 128 {
			retained := make([]boardPlacement, 0, 128+len(v.PendingCards))
			committed := 0
			for i := len(v.boardPlacements) - 1; i >= 0; i-- {
				p := v.boardPlacements[i]
				if v.PendingCards[p.Ref] || committed < 128 {
					retained = append(retained, p)
					if !v.PendingCards[p.Ref] {
						committed++
					}
				}
			}
			slices.Reverse(retained)
			v.boardPlacements = retained
		}
		if len(v.PendingCards) == 0 && v.boardRefreshQueued {
			v.boardRefreshQueued = false
			a.refreshRallyItems(v)
		}
	}
	if !a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		result, err := client.UpdatePositionIfUnchanged(ctx, op.Before, kind, op.Changes, op.Position)
		a.post(func() { finish(result, err) })
	}, func() { finish(nil, errWorkQueueFull) }) && a.ctx.Err() != nil {
		finish(nil, a.ctx.Err())
	}
}
func (a *App) undoBoardWrite(v *rallyView, u *boardUndo) {
	if v.boardUndo != u || len(v.PendingCards) > 0 || a.boardWriteBlocked(v) {
		a.toast = "This move can no longer be undone here"
		return
	}
	// A later local refresh or operation must not repurpose an older undo action.
	for _, o := range v.Items {
		if o.String("_ref") == u.After.String("_ref") {
			for _, key := range []string{"LastUpdateDate", "VersionId"} {
				if u.After.String(key) != "" && u.After.String(key) != o.String(key) {
					a.toast = "This work item changed; refresh before undoing"
					return
				}
			}
		}
	}
	preview := u.After.Clone()
	for k, value := range u.Fields {
		preview[k] = value
	}
	a.startBoardWrite(v, &boardWrite{Before: u.After.Clone(), Preview: preview, Changes: boardWireFields(u.Fields), Position: u.Position, Label: "Undid move", Undo: true})
}

func (v *rallyView) applyBoardPlacements() {
	for gi := range v.boardGroups {
		for li := range v.boardGroups[gi].lanes {
			lane := &v.boardGroups[gi].lanes[li]
			for _, placement := range v.boardPlacements {
				from, to := -1, -1
				for i, c := range lane.cards {
					if c.ref == placement.Ref {
						from = i
					}
					if c.ref == placement.Position.Ref {
						to = i
					}
				}
				if from < 0 || to < 0 || from == to {
					continue
				}
				card := lane.cards[from]
				lane.cards = slices.Delete(lane.cards, from, from+1)
				if from < to {
					to--
				}
				if placement.Position.Below {
					to++
				}
				lane.cards = slices.Insert(lane.cards, to, card)
			}
		}
	}
}

func boardWireFields(fields rally.Object) rally.Object {
	out := fields.Clone()
	for key, value := range fields {
		switch value.(type) {
		case map[string]any, rally.Object:
			if ref := fields.Ref(key); ref != "" {
				out[key] = ref
			}
		}
	}
	return out
}
func (a *App) pendingRallyWrite(id string) bool {
	v := a.rallyViews[id]
	return v != nil && (len(v.PendingCards) > 0 || v.Mutating || v.Detail != nil && v.Detail.Saving)
}
