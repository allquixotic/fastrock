//go:build nucular_headless

package ui

import (
	"fmt"
	"image"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"golang.org/x/mobile/event/mouse"
)

func TestV67BoardDisplayState(t *testing.T) {
	if !savedViewEqual(settings.SavedView{}, settings.SavedView{Display: settings.DefaultBoardDisplay().Copy()}) {
		t.Fatal("legacy view falsely modified by new default")
	}
	a, v := keyboardBoard(t, 3)
	want := settings.BoardDisplay{Density: "Compact", ColorBy: "Owner", WIPLimit: 4, AgeDays: 0}
	query := a.rallyQuery(v)
	a.setBoardDisplay(v, want)
	if a.rallyQuery(v) != query {
		t.Fatal("display changed the server query")
	}
	saved := v.savedView("Mine")
	v.Display.ColorBy = "Priority"
	if *saved.Display != want || *a.prefs.RallyDisplay != want {
		t.Fatal("display snapshot shares mutable state")
	}
	v.applySavedView(saved)
	if v.Display != want {
		t.Fatal("saved display lost")
	}
	v.applySavedView(settings.SavedView{Name: "Legacy", Page: "teamboard"})
	if v.Display != want {
		t.Fatal("legacy view erased display preference")
	}
	v.applySavedView(settings.SavedView{})
	if v.Display != settings.DefaultBoardDisplay() {
		t.Fatal("Standard View retained custom display settings")
	}
	v.Display = want
	v.beginBoardSettings()
	setText(v.DisplayDraft.Age, "unfinished")
	tab := *a.state.Current()
	first := a.checkpointDocument(tab)
	if first.Rally.DisplayDraft.Age != "unfinished" {
		t.Fatal("draft not checkpointed")
	}
	setText(v.DisplayDraft.Age, "9")
	v.boardStride, v.boardHeader = 222, 75
	v.LaneScroll = map[string]int{"board--Defined": 600}
	next := a.checkpointDocument(tab)
	if first.Rally == next.Rally || first.Rally.DisplayDraft.Age != "unfinished" {
		t.Fatal("checkpoint was stale or mutated")
	}
	b := transferFixture()
	if err := b.installTransfer(transferJSON(t, next)); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[tab.ID]
	if restored.Display != want || text(restored.DisplayDraft.Age) != "9" || restored.boardStride != 222 || restored.boardHeader != 75 {
		t.Fatal("transfer lost display/draft/geometry")
	}
	restored.updateBoardStride(150, 65)
	if restored.RestoreLaneScroll["board--Defined"] != boardScrollAtStride(600, 222, 150, 75, 65) {
		t.Fatal("transferred scroll did not adapt to geometry")
	}
	legacy := transferJSON(t, next)
	legacy.Rally.Display, legacy.Rally.DisplayDraft = nil, nil
	b.prefs.RallyDisplay = want.Copy()
	if err := b.installTransfer(legacy); err != nil || b.rallyViews[tab.ID].Display != want {
		t.Fatal("old session ignored current preferences", err)
	}
	v.beginBoardSettings()
	setText(v.DisplayDraft.WIP, "-1")
	if a.applyBoardSettings(v) || v.Display != want || v.DisplayDraft.Error == "" {
		t.Fatal("invalid WIP committed")
	}
	setText(v.DisplayDraft.WIP, "2")
	setText(v.DisplayDraft.Age, "1.5")
	if a.applyBoardSettings(v) || v.Display != want {
		t.Fatal("fractional age committed")
	}
	setText(v.DisplayDraft.Age, "0")
	if !a.applyBoardSettings(v) || v.DisplayDraft != nil || v.Display.WIPLimit != 2 || v.Display.AgeDays != 0 {
		t.Fatal("valid settings not applied")
	}
}

func TestV67BoardWIPIsAdvisory(t *testing.T) {
	a, v, server := boardMutationFixture(t)
	v.Display.WIPLimit = 1
	v.prepareCards()
	source, target := v.Items[0], v.Items[1]
	a.dropBoardCard(v, v.cards[source.String("_ref")], boardDrop{State: "Accepted", Group: boardDestination(v, target), Target: target.String("_ref")})
	drain(t, a, func() bool { return len(v.PendingCards) == 0 })
	server.mu.Lock()
	defer server.mu.Unlock()
	if len(server.writes) != 1 || server.writes[0]["ScheduleState"] != "Accepted" {
		t.Fatal("WIP prevented an otherwise valid move")
	}
}

func TestV67BoardCardPresentation(t *testing.T) {
	p := colors(false)
	item := rally.Object{"DisplayColor": "#105cab", "Priority": "High", "Owner": map[string]any{"_ref": "/user/7", "_refObjectName": "Sam"}}
	c := &boardCard{object: item, kind: "Defect", owner: "Sam"}
	if c.displayColor("Work Item", p) != hex(0x105cab) {
		t.Fatal("work item color not used")
	}
	ownerColor, priorityColor := c.displayColor("Owner", p), c.displayColor("Priority", p)
	copy := &boardCard{object: item.Clone(), kind: "Defect", owner: "Renamed owner"}
	copy.object["Owner"].(map[string]any)["_refObjectName"] = "Renamed owner"
	if copy.displayColor("Owner", p) != ownerColor || copy.displayColor("Priority", p) != priorityColor {
		t.Fatal("unstable color identity")
	}
	c.object["DisplayColor"] = "Pink"
	if c.workItemColor(p) != hex(0xdf1a7b) {
		t.Fatal("named color unsupported")
	}
	c.object["DisplayColor"] = "not a color"
	if c.workItemColor(p) != stableBoardColor("Defect", p.Accent) {
		t.Fatal("invalid color fallback")
	}
	now := time.Date(2026, 10, 9, 12, 0, 0, 0, time.UTC)
	for _, tc := range []struct {
		elapsed   time.Duration
		threshold int
		want      bool
	}{
		{72 * time.Hour, 3, true}, {72*time.Hour - time.Second, 3, false}, {90 * time.Hour, 0, false}, {-time.Hour, 1, false},
	} {
		_, got := boardAge(now.Add(-tc.elapsed), tc.threshold, now)
		if got != tc.want {
			t.Fatalf("age %+v got %v", tc, got)
		}
	}
	if _, aged := boardAge(time.Time{}, 3, now); aged {
		t.Fatal("missing date marked stale")
	}
	lane := boardLane{state: "Defined", cards: make([]*boardCard, 3)}
	if label, over := boardLaneLabel(lane, 2); label != "Defined   3/2" || !over {
		t.Fatal(label, over)
	}
	if _, over := boardLaneLabel(lane, 3); over {
		t.Fatal("at-limit lane warned")
	}
	if label, over := boardLaneLabel(lane, 0); label != "Defined   3/∞" || over {
		t.Fatal(label, over)
	}

	a, v := keyboardBoard(t, 1)
	v.Display.WIPLimit = 0
	v.Items[0]["Name"] = strings.Repeat("A long Unicode title 世界 ", 30)
	v.Items[0]["LastUpdateDate"] = time.Now().Add(-10 * 24 * time.Hour).Format(time.RFC3339Nano)
	v.Items[0]["DisplayColor"] = "#105cab"
	v.prepareCards()
	card := v.cards[v.Items[0].String("_ref")]
	for _, density := range []string{"Comfortable", "Compact"} {
		v.Display.Density = density
		h := nucular.NewHeadlessHarness(0, image.Pt(400, 600), func(w *nucular.Window) {
			m := boardMetrics(density, 1, w.Master().Style().Font)
			w.RowScaled(m.Height).Static(250)
			a.drawBoardCard(w, v, card)
		})
		h.Master().SetStyle(makeStyle(a.p, 13))
		h.Frame(false)
		m := boardMetrics(density, 1, h.Master().Style().Font)
		if len(card.lines) != m.TitleLines || !strings.HasSuffix(card.lines[len(card.lines)-1], "…") {
			t.Fatal("title not truncated", density, card.lines)
		}
		for _, line := range card.lines {
			if nucular.FontWidth(card.face, line) > card.width {
				t.Fatal("title overflow")
			}
		}
		age, accent := false, false
		for _, cmd := range h.Commands() {
			if cmd.Kind == command.TextCmd && cmd.Text.String == "10d" {
				age = true
			}
			if cmd.Kind == command.RectFilledCmd && cmd.RectFilled.Color == hex(0x105cab) {
				accent = true
			}
		}
		if !age || !accent {
			t.Fatal("age/color absent from card", density, age, accent)
		}
	}
}

func TestV67BoardDensityGeometryAndFocus(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		for _, size := range []int{13, 24} {
			t.Run(fmt.Sprintf("%g/%d", scale, size), func(t *testing.T) {
				a, v := keyboardBoard(t, 200)
				v.focusCard, v.cardFocusActive, v.revealCard = v.Items[50].String("_ref"), true, true
				var cmds []command.Command
				h := nucular.NewHeadlessHarness(0, image.Pt(int(900*scale), int(700*scale)), func(w *nucular.Window) {
					a.drawTeamBoard(w, v, v.filtered())
					cmds = append(cmds[:0], w.Commands().Commands...)
				})
				style := makeStyle(a.p, size)
				style.Scale(scale)
				h.Master().SetStyle(style)
				for range 4 {
					h.Frame(false)
				}
				oldStride, oldHeader := v.boardStride, v.boardHeader
				oldScroll := v.LaneScroll["board--Defined"]
				v.Display.Density = "Compact"
				for range 3 {
					h.Frame(false)
				}
				if v.boardStride >= oldStride || v.focusCard != v.Items[50].String("_ref") || v.revealCard {
					t.Fatal("density changed focus or failed to shrink")
				}
				want := boardScrollAtStride(oldScroll, oldStride, v.boardStride, oldHeader, v.boardHeader)
				if absInt(want-v.LaneScroll["board--Defined"]) > 2 {
					t.Fatal("density lost scroll anchor", oldScroll, want, v.LaneScroll)
				}
				m := boardMetrics("Compact", scale, h.Master().Style().Font)
				if m.TitleY+m.TitleLines*m.Line > m.OwnerY || m.OwnerY+m.OwnerH > m.IterationY || m.IterationY+m.Line > m.FooterY || m.FooterY+m.FooterH > m.Height-m.StatusH {
					t.Fatalf("overlapping card metrics: %+v", m)
				}
				ids := 0
				for _, cmd := range cmds {
					if cmd.Kind == command.TextCmd && strings.HasPrefix(cmd.Text.String, "US0") {
						ids++
					}
				}
				if ids == 0 || ids > 10 {
					t.Fatal("unbounded/empty visible cards", ids)
				}
				if len(v.cards) != 200 {
					t.Fatal("density discarded cards")
				}
			})
		}
	}
}

func TestV67BoardSettingsControls(t *testing.T) {
	a, v := keyboardBoard(t, 3)
	var click image.Point
	var clicking bool
	h := nucular.NewHeadlessHarness(0, image.Pt(700, 500), func(w *nucular.Window) {
		m := &w.Input().Mouse
		m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
		m.Buttons[mouse.ButtonLeft].Clicked = clicking
		a.drawBoardDisplayControls(w, v)
	})
	h.Master().SetStyle(makeStyle(a.p, 13))
	press := func(label string) {
		t.Helper()
		clicking = false
		h.Frame(false)
		found := false
		for _, cmd := range h.Commands() {
			if cmd.Kind == command.TextCmd && cmd.Text.String == label {
				click = image.Pt(cmd.Rect.X+cmd.Rect.W/2, cmd.Rect.Y+cmd.Rect.H/2)
				found = true
				break
			}
		}
		if !found {
			t.Fatal("missing control", label)
		}
		clicking = true
		h.Frame(false)
		clicking = false
		h.Frame(false)
	}
	press("Compact")
	if v.Display.Density != "Compact" {
		t.Fatal("density button inert")
	}
	press("Page settings")
	if v.DisplayDraft == nil {
		t.Fatal("settings button inert")
	}
	before := v.Display
	setText(v.DisplayDraft.WIP, "5")
	press("Cancel")
	if v.DisplayDraft != nil || v.Display != before {
		t.Fatal("cancel committed draft")
	}
	press("Page settings")
	setText(v.DisplayDraft.WIP, "5")
	setText(v.DisplayDraft.Age, "7")
	v.DisplayDraft.ColorBy = "Priority"
	press("Apply")
	if v.Display.WIPLimit != 5 || v.Display.AgeDays != 7 || v.Display.ColorBy != "Priority" || v.DisplayDraft != nil {
		t.Fatal("Apply button inert")
	}
	if !reflect.DeepEqual(*a.prefs.RallyDisplay, v.Display) {
		t.Fatal("UI did not persist settings")
	}
}

func TestV67ListDensity(t *testing.T) {
	a, v := keyboardBoard(t, 2)
	v.Mode, v.Columns = "list", []string{"Name"}
	heights := map[string]int{}
	for _, density := range []string{"Comfortable", "Compact"} {
		v.Display.Density = density
		h := nucular.NewHeadlessHarness(0, image.Pt(600, 400), func(w *nucular.Window) {
			a.drawRallyModes(w, v)
			a.table(w, v, v.Items)
		})
		h.Master().SetStyle(makeStyle(a.p, 13))
		h.Frame(false)
		first, second := 0, 0
		for _, cmd := range h.Commands() {
			if cmd.Kind != command.TextCmd {
				continue
			}
			if cmd.Text.String == "Story 0" {
				first = cmd.Rect.Y
			}
			if cmd.Text.String == "Story 1" {
				second = cmd.Rect.Y
			}
			if cmd.Text.String == "Page settings" {
				t.Fatal("board settings shown for list")
			}
		}
		if first == 0 || second <= first {
			t.Fatal("missing list rows")
		}
		heights[density] = second - first
	}
	if heights["Compact"] >= heights["Comfortable"] {
		t.Fatal("list density inert", heights)
	}
}

func TestV67BoardSettingsFitScales(t *testing.T) {
	for _, scale := range []float64{1, 1.25, 1.5, 2} {
		for _, width := range []int{320, 900} {
			t.Run(fmt.Sprintf("%g/%d", scale, width), func(t *testing.T) {
				a, v := keyboardBoard(t, 1)
				v.beginBoardSettings()
				h := nucular.NewHeadlessHarness(0, image.Pt(int(float64(width)*scale), int(600*scale)), func(w *nucular.Window) { a.drawBoardDisplayControls(w, v) })
				style := makeStyle(a.p, 13)
				style.Scale(scale)
				h.Master().SetStyle(style)
				h.Frame(false)
				seen := map[string]bool{}
				clip := image.Rect(0, 0, int(float64(width)*scale), int(600*scale))
				for _, cmd := range h.Commands() {
					if cmd.Kind == command.ScissorCmd {
						clip = image.Rect(cmd.Rect.X, cmd.Rect.Y, cmd.Rect.X+cmd.Rect.W, cmd.Rect.Y+cmd.Rect.H)
						continue
					}
					if cmd.Kind != command.TextCmd {
						continue
					}
					visible := image.Rect(cmd.Rect.X, cmd.Rect.Y, cmd.Rect.X+cmd.Rect.W, cmd.Rect.Y+cmd.Rect.H).Intersect(clip)
					if visible.Empty() {
						continue
					}
					seen[cmd.Text.String] = true
					if visible.Min.X < 0 || visible.Max.X > int(float64(width)*scale) {
						t.Fatal("control exceeds viewport", cmd.Text.String, cmd.Rect)
					}
				}
				for _, label := range []string{"Comfortable", "Compact", "Page settings", "Apply", "Cancel"} {
					if !seen[label] {
						t.Fatal("control clipped", label)
					}
				}
			})
		}
	}
}
