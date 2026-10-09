package ui

import (
	"fmt"
	"image"
	"sort"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"
	"github.com/aarzilli/nucular/style"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"golang.org/x/mobile/event/mouse"
)

type boardLane struct {
	id, state, label string
	cards            []*boardCard
}
type boardGroup struct {
	key, name string
	value     any
	lanes     []boardLane
}

type boardCard struct {
	object                                                           rally.Object
	kind                                                             string
	ref, id, title, owner, initials, iteration, points, tasks, state string
	blocked, ready                                                   bool
	width, titleLines                                                int
	face                                                             font.Face
	updated                                                          time.Time
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
	v.stateOptions = append([]string{"All states"}, v.stateNames()...)
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
		estimate := "PlanEstimate"
		kind := v.objectKind(o)
		if kind == "Task" {
			estimate = "Estimate"
		}
		points := "—"
		if o[estimate] != nil {
			points = fmt.Sprintf("%g", o.Number(estimate))
		}
		tasks := fmt.Sprintf("%d tasks", o.Count("Tasks"))
		if o.Count("Tasks") == 1 {
			tasks = "1 task"
		}
		if status := o.String("TaskStatus"); status != "" {
			tasks += " · " + status
		}
		if o["TaskRemainingTotal"] != nil {
			tasks += fmt.Sprintf(" · %gh", o.Number("TaskRemainingTotal"))
		}
		if n := o.Count("Discussion"); n > 0 {
			tasks += fmt.Sprintf(" · %d comments", n)
		}
		if owner == "Unassigned" {
			initials = "—"
		}
		c := &boardCard{object: o, kind: kind, ref: o.String("_ref"), id: o.ID(), title: o.String("Name"), owner: owner, initials: initials, iteration: fallback(o.String("Iteration"), "Unscheduled"), points: points, tasks: tasks, state: fallback(o.String(v.stateField()), "Unspecified"), blocked: o.Bool("Blocked"), ready: o.Bool("Ready")}
		c.updated, _ = time.Parse(time.RFC3339Nano, o.String("LastUpdateDate"))
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
	metadata := make(map[string]boardGroup)
	for _, o := range items {
		key, name, value := boardGroupIdentity(o, v.Group)
		metadata[key] = boardGroup{key: key, name: name, value: value}
		grouped[key] = append(grouped[key], v.cards[o.String("_ref")])
	}
	if v.Group != "" && v.Group != "None" {
		groups = nil
		for key := range grouped {
			groups = append(groups, key)
		}
		sort.Slice(groups, func(i, j int) bool {
			if metadata[groups[i]].name == metadata[groups[j]].name {
				return groups[i] < groups[j]
			}
			return metadata[groups[i]].name < metadata[groups[j]].name
		})
	}
	v.boardGroups = make([]boardGroup, len(groups))
	v.largestLane = 0
	for gi, g := range groups {
		bg := metadata[g]
		bg.lanes = make([]boardLane, len(columns))
		indices := make(map[string]int, len(columns))
		for ci, c := range columns {
			bg.lanes[ci] = boardLane{id: "board-" + bg.key + "-" + c, state: c}
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
			v.largestLane = max(v.largestLane, len(l.cards))
		}
		v.boardGroups[gi] = bg
	}
	v.applyBoardPlacements()
	if v.cardFocusActive {
		v.currentBoardFocus(v.boardFocusEntries())
	}
}
func (a *App) drawTeamBoard(w *nucular.Window, v *rallyView, items []rally.Object) {
	v.prepareCards()
	v.prepareBoardLayout(items)
	scale := w.Master().Style().Scaling
	m := boardMetrics(v.Display.Density, scale, w.Master().Style().Font)
	// Header/add rows and spacing precede the virtualized card range.
	header := int(55*scale) + 2*m.Gap
	v.updateBoardStride(m.Height+m.Gap, header)
	if len(items) == 0 && !v.Loading {
		muted(w, "No work items match the current scope and filters.", a.p)
		w.Row(28).Static(130)
		if w.ButtonText("Clear filters") {
			v.applySavedView(settings.SavedView{})
			a.refreshRally(v)
		}
		return
	}
	var target *boardDrop
	released := w.Input().Mouse.Released(mouse.ButtonLeft)
	largestLane := v.largestLane
	if v.CollapsedLanes == nil {
		v.CollapsedLanes = map[string]bool{}
	}
	firstGroup, lastGroup := 0, len(v.boardGroups)
	groupStride := 0
	var focus boardFocusEntry
	focusPending := false
	if v.revealCard {
		focus, focusPending = v.currentBoardFocus(v.boardFocusEntries())
	}
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	if len(v.boardGroups) > 1 {
		groupStride = int(420*w.Master().Style().Scaling) + int(float64(titleHeight(w))*w.Master().Style().Scaling) + 2*spacing
		if focusPending {
			top := w.LayoutNextRowY() + focus.group*groupStride
			revealBoardRange(w, top, top+min(groupStride, w.Bounds.H))
		}
		firstGroup, lastGroup = sidebarVisible(w.LayoutNextRowY(), w.Bounds.Y, w.Bounds.Y+w.Bounds.H, groupStride, len(v.boardGroups))
		sidebarSkip(w, firstGroup, groupStride, spacing)
	}
	for groupIndex, group := range v.boardGroups[firstGroup:lastGroup] {
		if group.name != "" {
			title(w, group.name, a.p)
		}
		height := max(int(260*w.Master().Style().Scaling), w.LayoutAvailableHeight()-8)
		if len(v.boardGroups) > 1 {
			height = int(420 * w.Master().Style().Scaling)
		}
		// Keep useful card widths on narrow windows; the parent exposes horizontal scrolling.
		if len(v.boardWidths) != len(group.lanes) {
			v.boardWidths = make([]int, len(group.lanes))
		}
		for i := range v.boardWidths {
			v.boardWidths[i] = max(int(220*w.Master().Style().Scaling), (w.LayoutAvailableWidth()-8*(len(group.lanes)-1))/len(group.lanes))
			if v.CollapsedLanes[group.lanes[i].state] {
				v.boardWidths[i] = int(45 * w.Master().Style().Scaling)
			}
		}
		w.RowScaled(height).StaticScaled(v.boardWidths...)
		for laneIndex, lane := range group.lanes {
			column := lane.state
			focusLane := focusPending && focus.group == firstGroup+groupIndex && focus.lane == laneIndex
			if focusLane {
				bounds := w.WidgetBounds()
				left, right := bounds.X, bounds.X+min(bounds.W, w.Bounds.W)
				if left < w.Bounds.X {
					w.Scrollbar.X = max(0, w.Scrollbar.X+left-w.Bounds.X)
					w.Master().Changed()
				} else if right > w.Bounds.X+w.Bounds.W {
					w.Scrollbar.X += right - (w.Bounds.X + w.Bounds.W)
					w.Master().Changed()
				}
			}
			if v.CollapsedLanes[column] {
				if compact := w.GroupBegin(lane.id+"-collapsed", nucular.WindowNoScrollbar); compact != nil {
					compact.Row(30).Dynamic(1)
					if compact.ButtonText("›") {
						v.CollapsedLanes[column] = false
					}
					for _, r := range lane.state {
						compact.Row(18).Dynamic(1)
						compact.Label(string(r), "CC")
					}
					compact.Row(24).Dynamic(1)
					compact.Label(fmt.Sprint(len(lane.cards)), "CC")
					compact.GroupEnd()
				}
				continue
			}
			old := w.Master().Style().GroupWindow
			gs := old
			gs.FixedBackground = style.MakeItemColor(a.p.Sunken)
			gs.Padding = image.Pt(m.Padding, m.Padding)
			gs.Spacing = image.Pt(m.Gap, m.Gap)
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
				target = &boardDrop{State: column, Group: group}
			}
			col.Row(30).Dynamic(1)
			b, out := col.Custom(col.CustomState())
			if out != nil {
				if dropTarget {
					out.FillRect(b, 4, a.p.Selected)
				}
				label, overLimit := boardLaneLabel(lane, v.Display.WIPLimit)
				fg := a.p.Text
				if overLimit {
					out.FillRect(b, 3, a.p.WarningSoft)
					fg = a.p.Warning
				}
				labelAt(out, inset(b, 2, 0), label, col.Master().Style().Font, fg)
				if col.Input().Mouse.HoveringRect(b) {
					col.Tooltip("Loaded matching cards / WIP limit. Click to collapse.")
				}
				if col.Input().Mouse.Clicked(mouse.ButtonLeft, b) {
					v.CollapsedLanes[column] = true
					v.currentBoardFocus(v.boardFocusEntries())
				}
			}
			col.Row(25).Dynamic(1)
			if col.ButtonText("+ Add in " + column) {
				a.newArtifactInState(v, column)
			}
			// Layout only rows intersecting the viewport plus one overscan row.
			stride := m.Height + gs.Spacing.Y
			if delta := v.scrollAdjustment[column]; delta != 0 {
				col.Scrollbar.Y = max(0, col.Scrollbar.Y-delta*stride)
				delete(v.scrollAdjustment, column)
			}
			if focusLane {
				top := col.LayoutNextRowY() + focus.index*stride
				if revealBoardRange(col, top, top+stride-gs.Spacing.Y) {
					v.revealCard = false
				}
			}
			first := max(0, col.Scrollbar.Y/stride-1)
			last := min(len(lane.cards), first+col.Bounds.H/stride+4)
			first = min(first, len(lane.cards))
			if first > 0 {
				col.RowScaled(first*stride - gs.Spacing.Y).Dynamic(1)
				col.Spacing(1)
			}
			for _, c := range lane.cards[first:last] {
				col.RowScaled(m.Height).Dynamic(1)
				cardBounds := col.WidgetBounds()
				if dropTarget && target != nil && c.ref != v.dragCard && col.Input().Mouse.HoveringRect(cardBounds) {
					target.Target = c.ref
					target.Below = col.Input().Mouse.Pos.Y >= cardBounds.Y+cardBounds.H/2
					if v.Descending {
						target.Below = !target.Below
					}
				}
				a.drawBoardCard(col, v, c)
			}
			if last < len(lane.cards) {
				col.RowScaled((len(lane.cards)-last)*stride - gs.Spacing.Y).Dynamic(1)
				col.Spacing(1)
			}
			if largestLane < height/stride+2 || (col.Scrollbar.Y > 0 || col.Input().Mouse.HoveringRect(col.Bounds)) && col.Scrollbar.Y+col.Bounds.H >= len(lane.cards)*stride-2*stride {
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
	if groupStride > 0 {
		sidebarSkip(w, len(v.boardGroups)-lastGroup, groupStride, spacing)
	}
	if released && v.cardDragging {
		if target != nil {
			if c := v.cards[v.dragCard]; c != nil {
				a.dropBoardCard(v, c, *target)
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
	scale := w.Master().Style().Scaling
	sz := func(n int) int { return max(1, int(float64(n)*scale)) }
	m := boardMetrics(v.Display.Density, scale, face)
	age, aged := boardAge(c.updated, v.Display.AgeDays, time.Now())
	hover := in.Mouse.HoveringRect(b)
	if hover {
		tip := rallyKindLabel(c.kind) + " · " + c.title + "\nColor: " + v.Display.ColorBy
		if !c.updated.IsZero() {
			tip += fmt.Sprintf("\nLast updated %d days ago", age)
		}
		w.Tooltip(tip)
	}
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
	if v.cardFocusActive && v.focusCard == c.ref && !v.Search.Active && !v.Query.Active {
		out.FillRect(inset(b, 3, 3), 3, a.p.Accent)
		out.FillRect(inset(b, 5, 5), 2, fill)
	}
	// Color is a separate accent; selection and keyboard focus remain visible.
	out.FillRect(rect.Rect{X: b.X + sz(1), Y: b.Y + sz(6), W: sz(4), H: b.H - sz(12)}, uint16(sz(2)), c.displayColor(v.Display.ColorBy, a.p))
	x, y, width := b.X+m.Padding, b.Y+m.Padding, b.W-2*m.Padding
	badge := rect.Rect{X: x, Y: y, W: sz(27), H: m.Header}
	out.FillRect(badge, uint16(sz(3)), a.p.Selected)
	labelAt(out, inset(badge, sz(2), 0), artifactBadge(c.kind), face, c.workItemColor(a.p))
	check := rect.Rect{X: b.X + b.W - m.Padding - sz(14), Y: y + sz(2), W: sz(14), H: sz(14)}
	labelRight := check.X - sz(6)
	if aged {
		label := fmt.Sprintf("%dd", age)
		ageWidth := nucular.FontWidth(face, label) + sz(8)
		r := rect.Rect{X: labelRight - ageWidth, Y: y, W: ageWidth, H: m.Header}
		out.FillRect(r, uint16(sz(3)), a.p.WarningSoft)
		labelAt(out, inset(r, sz(4), 0), label, face, a.p.Warning)
		labelRight = r.X - sz(4)
	}
	idr := rect.Rect{X: x + sz(33), Y: y, W: max(0, labelRight-x-sz(33)), H: m.Header}
	labelAt(out, idr, c.id, face, a.p.Accent)
	out.FillRect(check, uint16(sz(3)), a.p.Border)
	out.FillRect(inset(check, sz(1), sz(1)), uint16(sz(2)), fill)
	if v.Selected[c.ref] {
		out.FillRect(inset(check, sz(3), sz(3)), uint16(sz(1)), a.p.Accent)
	}
	if in.Mouse.Clicked(mouse.ButtonLeft, check) {
		v.focusCard, v.cardFocusActive = c.ref, true
		v.selectItem(c.object, !v.Selected[c.ref])
		return
	}
	y = b.Y + m.TitleY
	if c.width != width || c.face != face || c.titleLines != m.TitleLines {
		c.width, c.face, c.titleLines = width, face, m.TitleLines
		c.lines = nucular.WrapText(face, c.title, width)
		if len(c.lines) > m.TitleLines {
			c.lines = c.lines[:m.TitleLines]
			c.lines[m.TitleLines-1] = ellipsizeBoardTitle(face, c.lines[m.TitleLines-1], width)
		}
	}
	for i, line := range c.lines {
		labelAt(out, rect.Rect{X: x, Y: y + i*m.Line, W: width, H: m.Line}, line, face, a.p.Text)
	}
	y = b.Y + m.OwnerY
	if contains(v.CardFields, "Owner") {
		out.FillCircle(rect.Rect{X: x, Y: y, W: m.OwnerH, H: m.OwnerH}, a.p.Selected)
		labelAt(out, rect.Rect{X: x + sz(2), Y: y, W: m.OwnerH, H: m.OwnerH}, c.initials, face, a.p.Muted)
		labelAt(out, rect.Rect{X: x + m.OwnerH + sz(8), Y: y, W: max(0, width-m.OwnerH-sz(8)), H: m.OwnerH}, c.owner, face, a.p.Muted)
	}
	if contains(v.CardFields, "Iteration") {
		labelAt(out, rect.Rect{X: x, Y: b.Y + m.IterationY, W: width, H: m.Line}, c.iteration, face, a.p.Muted)
	}
	if contains(v.CardFields, "Tasks") {
		labelAt(out, rect.Rect{X: x, Y: b.Y + m.FooterY, W: max(0, width-sz(50)), H: m.FooterH}, c.tasks, face, a.p.Faint)
	}
	pr := rect.Rect{X: b.X + b.W - m.Padding - sz(30), Y: b.Y + m.FooterY, W: sz(30), H: m.FooterH}
	if contains(v.CardFields, "PlanEstimate") {
		out.FillRect(pr, uint16(sz(10)), a.p.Selected)
		labelAt(out, inset(pr, sz(6), 0), c.points, face, a.p.Accent)
	}
	if c.blocked || c.ready {
		fg, bg, label := a.p.Success, a.p.Selected, "Ready"
		if c.blocked {
			fg, label = a.p.Danger, "Blocked"
		}
		r := rect.Rect{X: b.X + 1, Y: b.Y + b.H - m.StatusH - 1, W: b.W - 2, H: m.StatusH}
		out.FillRect(r, 3, bg)
		labelAt(out, inset(r, 10, 0), label, face, fg)
	}
	if v.PendingCards[c.ref] {
		pending := rect.Rect{X: b.X + 1, Y: b.Y + b.H - m.StatusH - 1, W: b.W - 2, H: m.StatusH}
		out.FillRect(pending, 3, a.p.WarningSoft)
		labelAt(out, inset(pending, 10, 0), "Saving…", face, a.p.Warning)
	}
	if !v.PendingCards[c.ref] && in.Mouse.IsClickDownInRect(mouse.ButtonLeft, b, true) {
		v.focusCard, v.cardFocusActive = c.ref, true
		v.dragCard = c.ref
		v.dragCardX = in.Mouse.Pos.X
		v.dragCardY = in.Mouse.Pos.Y
	}
	if v.dragCard == c.ref && in.Mouse.Down(mouse.ButtonLeft) && (absInt(in.Mouse.Pos.X-v.dragCardX) > sz(6) || absInt(in.Mouse.Pos.Y-v.dragCardY) > sz(6)) {
		v.cardDragging = true
	}
	if in.Mouse.Clicked(mouse.ButtonLeft, b) && !v.cardDragging && !v.PendingCards[c.ref] {
		a.openArtifact(v, c.object)
		v.dragCard = ""
	}
}
