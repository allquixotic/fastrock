package ui

import (
	"context"
	"fmt"
	"image"
	"sort"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/rect"
	"github.com/aarzilli/nucular/style"
	"github.com/allquixotic/fastrock/internal/rally"
	"golang.org/x/mobile/event/mouse"
)

type boardLane struct {
	id, state, label string
	cards            []*boardCard
}
type boardGroup struct {
	name  string
	lanes []boardLane
}

type boardCard struct {
	object                                                           rally.Object
	ref, id, title, owner, initials, iteration, points, tasks, state string
	blocked, ready                                                   bool
	width                                                            int
	lines                                                            []string
}

func (v *rallyView) prepareCards() {
	var source *rally.Object
	if len(v.Items) > 0 {
		source = &v.Items[0]
	}
	if source == v.cardSource && v.Generation == v.cardGeneration && v.cards != nil {
		return
	}
	v.cardSource = source
	v.cardGeneration = v.Generation
	v.cards = make(map[string]*boardCard, len(v.Items))
	v.ownerOptions = []string{"All owners"}
	v.stateOptions = append([]string{"All states"}, rally.States(v.Spec.Kind)...)
	owners := make(map[string]bool)
	states := make(map[string]bool)
	for _, s := range v.stateOptions {
		states[s] = true
	}
	for _, o := range v.Items {
		owner := fallback(o.String("Owner"), "Unassigned")
		initials := ""
		for _, word := range strings.Fields(owner) {
			for _, r := range word {
				initials += string(r)
				break
			}
			if len(initials) > 1 {
				break
			}
		}
		c := &boardCard{object: o, ref: o.String("_ref"), id: o.ID(), title: o.String("Name"), owner: owner, initials: initials, iteration: fallback(o.String("Iteration"), "Unscheduled"), points: fmt.Sprintf("%g", o.Number("PlanEstimate")), tasks: fmt.Sprintf("%d tasks", o.Count("Tasks")), state: fallback(o.String(rally.StateField(v.Spec.Kind)), rally.States(v.Spec.Kind)[0]), blocked: o.Bool("Blocked"), ready: o.Bool("Ready")}
		v.cards[c.ref] = c
		if !owners[c.owner] {
			owners[c.owner] = true
			v.ownerOptions = append(v.ownerOptions, c.owner)
		}
		if !states[c.state] {
			states[c.state] = true
			v.stateOptions = append(v.stateOptions, c.state)
		}
	}
	sort.Strings(v.ownerOptions[1:])
}
func (v *rallyView) prepareBoardLayout(items []rally.Object) {
	if v.boardPrepared && v.boardRevision == v.filterRevision && v.boardGrouping == v.Group {
		return
	}
	v.boardPrepared = true
	v.boardRevision = v.filterRevision
	v.boardGrouping = v.Group
	columns := append([]string(nil), v.stateOptions[1:]...)
	groups := []string{""}
	grouped := make(map[string][]*boardCard)
	for _, o := range items {
		g := ""
		if v.Group != "None" {
			g = fallback(o.String(v.Group), "Unassigned")
		}
		grouped[g] = append(grouped[g], v.cards[o.String("_ref")])
	}
	if v.Group != "None" {
		groups = nil
		for g := range grouped {
			groups = append(groups, g)
		}
		sort.Strings(groups)
	}
	v.boardGroups = make([]boardGroup, len(groups))
	for gi, g := range groups {
		bg := boardGroup{name: g, lanes: make([]boardLane, len(columns))}
		indices := make(map[string]int, len(columns))
		for ci, c := range columns {
			bg.lanes[ci] = boardLane{id: "board-" + g + "-" + c, state: c}
			indices[c] = ci
		}
		for _, card := range grouped[g] {
			if card != nil {
				if ci, ok := indices[card.state]; ok {
					bg.lanes[ci].cards = append(bg.lanes[ci].cards, card)
				}
			}
		}
		for ci := range bg.lanes {
			l := &bg.lanes[ci]
			l.label = fmt.Sprintf("%s   %d", l.state, len(l.cards))
		}
		v.boardGroups[gi] = bg
	}
}
func (a *App) drawTeamBoard(w *nucular.Window, v *rallyView, items []rally.Object) {
	v.prepareCards()
	v.prepareBoardLayout(items)
	target := ""
	released := w.Input().Mouse.Released(mouse.ButtonLeft)
	largestLane := 0
	for _, g := range v.boardGroups {
		for _, l := range g.lanes {
			largestLane = max(largestLane, len(l.cards))
		}
	}
	for _, group := range v.boardGroups {
		if group.name != "" {
			title(w, group.name, a.p)
		}
		height := max(260, w.LayoutAvailableHeight()-8)
		if len(v.boardGroups) > 1 {
			height = 420
		}
		// Keep useful card widths on narrow windows; the parent exposes horizontal scrolling.
		if len(v.boardWidths) != len(group.lanes) {
			v.boardWidths = make([]int, len(group.lanes))
		}
		for i := range v.boardWidths {
			v.boardWidths[i] = max(190, (w.LayoutAvailableWidth()-8*(len(group.lanes)-1))/len(group.lanes))
		}
		w.Row(height).Static(v.boardWidths...)
		for _, lane := range group.lanes {
			column := lane.state
			old := w.Master().Style().GroupWindow
			gs := old
			gs.FixedBackground = style.MakeItemColor(a.p.Sunken)
			gs.Padding = image.Pt(8, 8)
			gs.Spacing = image.Pt(8, 10)
			w.Master().Style().GroupWindow = gs
			col := w.GroupBegin(lane.id, nucular.WindowNoHScrollbar)
			if col == nil {
				w.Master().Style().GroupWindow = old
				continue
			}
			if n, ok := v.RestoreLaneScroll[lane.id]; ok {
				col.Scrollbar.Y = n
				delete(v.RestoreLaneScroll, lane.id)
			}
			bounds := col.Bounds
			dropTarget := v.cardDragging && w.Input().Mouse.HoveringRect(bounds)
			if dropTarget {
				target = column
			}
			col.Row(30).Dynamic(1)
			b, out := col.Custom(col.CustomState())
			if out != nil {
				if dropTarget {
					out.FillRect(b, 4, a.p.Selected)
				}
				labelAt(out, inset(b, 2, 0), lane.label, col.Master().Style().Font, stateColor(column, a.p))
			}
			if v.ExitAgreements {
				muted(col, "Review acceptance before advancing", a.p)
			}
			// Layout only rows intersecting the viewport plus one overscan row.
			const stride = 206
			if delta := v.scrollAdjustment[column]; delta != 0 {
				col.Scrollbar.Y = max(0, col.Scrollbar.Y-delta)
				delete(v.scrollAdjustment, column)
			}
			first := max(0, col.Scrollbar.Y/stride-1)
			last := min(len(lane.cards), first+col.Bounds.H/stride+4)
			first = min(first, len(lane.cards))
			if first > 0 {
				col.Row(first*stride - 10).Dynamic(1)
				col.Spacing(1)
			}
			for _, c := range lane.cards[first:last] {
				col.Row(196).Dynamic(1)
				a.drawBoardCard(col, v, c)
			}
			if last < len(lane.cards) {
				col.Row((len(lane.cards)-last)*stride - 10).Dynamic(1)
				col.Spacing(1)
			}
			if largestLane < height/stride+2 || col.Input().Mouse.HoveringRect(col.Bounds) && col.Input().Mouse.ScrollDelta < 0 && col.Scrollbar.Y+col.Bounds.H >= len(lane.cards)*stride-2*stride {
				a.needRallyPage(v)
			}
			if col.Scrollbar.Y == 0 && v.Start > 1 && col.Input().Mouse.ScrollDelta > 0 {
				a.previousRallyPage(v)
			}
			if v.LaneScroll == nil {
				v.LaneScroll = map[string]int{}
			}
			v.LaneScroll[lane.id] = col.Scrollbar.Y
			col.GroupEnd()
			w.Master().Style().GroupWindow = old
		}
	}
	if released && v.cardDragging {
		if target != "" {
			if c := v.cards[v.dragCard]; c != nil && c.state != target {
				a.moveBoardCard(v, c, target)
			}
		}
		v.cardDragging = false
		v.dragCard = ""
	}
}
func (a *App) drawBoardCard(w *nucular.Window, v *rallyView, c *boardCard) {
	b, out := w.Custom(w.CustomState())
	if out == nil {
		return
	}
	in := w.Input()
	face := w.Master().Style().Font
	hover := in.Mouse.HoveringRect(b)
	fill := a.p.Surface
	if hover {
		fill = a.p.Alt
	}
	border := a.p.Border
	if v.Selected[c.ref] || v.dragCard == c.ref {
		border = a.p.Accent
	}
	out.FillRect(b, 5, border)
	out.FillRect(inset(b, 1, 1), 4, fill)
	x, y, width := b.X+12, b.Y+10, b.W-24
	idr := rect.Rect{X: x, Y: y, W: width - 25, H: 20}
	labelAt(out, idr, c.id, face, a.p.Accent)
	check := rect.Rect{X: b.X + b.W - 27, Y: y + 2, W: 14, H: 14}
	out.FillRect(check, 3, a.p.Border)
	out.FillRect(inset(check, 1, 1), 2, fill)
	if v.Selected[c.ref] {
		out.FillRect(inset(check, 3, 3), 1, a.p.Accent)
	}
	if in.Mouse.Clicked(mouse.ButtonLeft, check) {
		v.Selected[c.ref] = !v.Selected[c.ref]
		return
	}
	y += 30
	if c.width != width {
		c.width = width
		c.lines = nucular.WrapText(face, c.title, width)
	}
	for i, line := range c.lines {
		if i == 3 {
			break
		}
		labelAt(out, rect.Rect{X: x, Y: y + i*18, W: width, H: 18}, line, face, a.p.Text)
	}
	y = b.Y + 99
	out.FillCircle(rect.Rect{X: x, Y: y, W: 22, H: 22}, a.p.Selected)
	labelAt(out, rect.Rect{X: x + 2, Y: y, W: 22, H: 22}, c.initials, face, a.p.Muted)
	labelAt(out, rect.Rect{X: x + 30, Y: y, W: width - 30, H: 22}, c.owner, face, a.p.Muted)
	labelAt(out, rect.Rect{X: x, Y: b.Y + 128, W: width, H: 20}, c.iteration, face, a.p.Muted)
	labelAt(out, rect.Rect{X: x, Y: b.Y + 151, W: width - 50, H: 22}, c.tasks, face, a.p.Faint)
	pr := rect.Rect{X: b.X + b.W - 44, Y: b.Y + 151, W: 30, H: 22}
	out.FillRect(pr, 10, a.p.Selected)
	labelAt(out, inset(pr, 8, 0), c.points, face, a.p.Accent)
	if c.blocked || c.ready {
		fg, bg, label := a.p.Success, a.p.Selected, "Ready"
		if c.blocked {
			fg, label = a.p.Danger, "Blocked"
		}
		r := rect.Rect{X: b.X + 1, Y: b.Y + b.H - 24, W: b.W - 2, H: 23}
		out.FillRect(r, 3, bg)
		labelAt(out, inset(r, 10, 0), label, face, fg)
	}
	if in.Mouse.IsClickDownInRect(mouse.ButtonLeft, b, true) {
		v.dragCard = c.ref
		v.dragCardX = in.Mouse.Pos.X
		v.dragCardY = in.Mouse.Pos.Y
	}
	if v.dragCard == c.ref && in.Mouse.Down(mouse.ButtonLeft) && (absInt(in.Mouse.Pos.X-v.dragCardX) > 6 || absInt(in.Mouse.Pos.Y-v.dragCardY) > 6) {
		v.cardDragging = true
	}
	if in.Mouse.Clicked(mouse.ButtonLeft, b) && !v.cardDragging {
		a.openArtifact(v, c.object)
		v.dragCard = ""
	}
}
func (a *App) moveBoardCard(v *rallyView, c *boardCard, state string) {
	client := a.rallyClient
	field := rally.StateField(v.Spec.Kind)
	a.status = "Moving " + c.id + " to " + state
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		current, err := client.Get(ctx, c.ref)
		if err == nil && c.object.String("LastUpdateDate") != "" && current.String("LastUpdateDate") != c.object.String("LastUpdateDate") {
			err = fmt.Errorf("%s changed; refresh before moving it", c.id)
		}
		if err == nil {
			_, err = client.Update(ctx, c.ref, v.Spec.Kind, rally.Object{field: state})
		}
		a.post(func() {
			if err != nil {
				a.report(err)
			} else {
				a.refreshRally(v)
			}
			a.status = "Connected"
		})
	})
}
