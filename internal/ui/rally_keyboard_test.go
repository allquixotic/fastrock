//go:build fltk_headless

package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

func keyboardBoard(t *testing.T, n int) (*App, *rallyView) {
	t.Helper()
	a := transferFixture()
	a.ctx, a.cancel = context.WithCancel(context.Background())
	t.Cleanup(a.cancel)
	a.updates = make(chan func(), 128)
	a.p, a.prefs.FontSize = colors(false), 13
	a.navigationFace = typeFace(13, regularFont)
	id := a.state.Open(workspace.Rally, "Team Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	v.Items = make([]rally.Object, n)
	for i := range v.Items {
		v.Items[i] = rally.Object{"_ref": fmt.Sprintf("/hierarchicalrequirement/%d", i), "FormattedID": fmt.Sprintf("US%04d", i), "Name": fmt.Sprintf("Story %d", i), "ScheduleState": "Defined", "DragAndDropRank": fmt.Sprintf("%06d", i)}
	}
	a.rallyViews[id] = v
	return a, v
}

func TestV44RallyActionsRespectContextAndRemapping(t *testing.T) {
	a, v := keyboardBoard(t, 3)
	h := desktop.NewHeadlessHarness(0, image.Pt(900, 700), a.shortcuts)
	a.window = h.Master()
	h.Key(key.CodeSlash, 0)
	h.Frame(false)
	if !v.focusSearch {
		t.Fatal("slash did not focus Rally search")
	}
	v.focusSearch = false
	v.Search.Active = true
	h.Key(key.CodeSlash, 0)
	h.Frame(false)
	if v.focusSearch {
		t.Fatal("slash intercepted an active text editor")
	}
	h.Key(key.CodeTab, 0)
	h.Frame(false)
	if !v.cardFocusActive || v.Search.Active || v.focusCard != v.Items[0].String("_ref") {
		t.Fatal("Tab did not enter the first work item", v.focusCard)
	}
	h.Key(key.CodeTab, 0)
	h.Frame(false)
	if v.focusCard != v.Items[1].String("_ref") {
		t.Fatal("Tab did not advance the focused item")
	}
	h.Key(key.CodeTab, key.ModShift)
	h.Frame(false)
	if v.focusCard != v.Items[0].String("_ref") {
		t.Fatal("Shift+Tab did not move backward")
	}
	h.Key(key.CodeTab, key.ModShift)
	h.Frame(false)
	if v.cardFocusActive || !v.focusSearch {
		t.Fatal("focus did not return to search at the board boundary")
	}
	v.focusSearch = false
	a.prefs.Keymap = map[string]settings.KeyBinding{"rally-search": {Code: int(key.CodeF6)}}
	h.Key(key.CodeSlash, 0)
	h.Frame(false)
	if v.focusSearch {
		t.Fatal("replaced default shortcut still ran")
	}
	h.Key(key.CodeF6, 0)
	h.Frame(false)
	if !v.focusSearch {
		t.Fatal("configured shortcut did not run")
	}
	if validateActionShortcut("rally-search", key.CodeSlash, 0) != "" || validateActionShortcut("new-tab", key.CodeSlash, 0) == "" || actionsOverlap("escape", "rally-close-detail") {
		t.Fatal("scoped shortcut validation or overlap is wrong")
	}
	v.focusSearch = false
	v.Query.Active = true
	h.Key(key.CodeF6, 0)
	h.Frame(false)
	if !v.focusSearch {
		t.Fatal("non-typing search shortcut was blocked by query editing")
	}
	v.focusSearch, v.Query.Active = false, false
	a.state.Open(workspace.New, "New", "", "")
	h.Key(key.CodeF6, 0)
	h.Frame(false)
	if v.focusSearch {
		t.Fatal("Rally shortcut ran in a different document")
	}
}

func TestV44FocusedCardOpensAndDirtyEscapeKeepsWork(t *testing.T) {
	a, v := keyboardBoard(t, 2)
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": []any{}}})
	}))
	t.Cleanup(s.Close)
	a.rallyClient, _ = rally.New(s.URL, "fixture", nil)
	h := desktop.NewHeadlessHarness(0, image.Pt(900, 700), a.shortcuts)
	a.window = h.Master()
	keyboardFrame(h, key.CodeTab, 0)
	keyboardFrame(h, key.CodeReturnEnter, 0)
	if v.Detail == nil || v.Detail.Original.ID() != "US0000" {
		t.Fatal("Enter did not open the focused card")
	}
	// Replace the loading draft with a deterministic clean/dirty detail fixture.
	a.disposeDetail(v.Detail)
	d := makeDetail(v.Items[0], "HierarchicalRequirement", false)
	v.Detail = d
	setText(d.Editors["Name"], "Unsaved correction")
	keyboardFrame(h, key.CodeEscape, 0)
	if v.Detail != d || !d.dirty() {
		t.Fatal("Escape discarded a dirty work item")
	}
	deadline := time.Now().Add(time.Second)
	for {
		prompt := false
		h.Master().Lock()
		h.Frame(false)
		labels := map[string]bool{}
		for _, cmd := range h.Commands() {
			if cmd.Kind == command.TextCmd {
				labels[cmd.Text.String] = true
			}
		}
		prompt = labels["Unsaved work item"] && labels["Keep editing"] && labels["Discard"] && labels["Save"]
		h.Master().Unlock()
		if prompt {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("dirty Escape did not offer save/discard/keep editing")
		}
		time.Sleep(time.Millisecond)
	}
	keyboardFrame(h, key.CodeEscape, 0)
	if v.Detail != d || !d.dirty() {
		t.Fatal("dismissed prompt discarded edits")
	}
	setText(d.Editors["Name"], d.Original.String("Name"))
	d.Saving = true
	keyboardFrame(h, key.CodeEscape, 0)
	if v.Detail != d {
		t.Fatal("Escape closed an in-flight save")
	}
	d.Saving = false
	keyboardFrame(h, key.CodeEscape, 0)
	if v.Detail != nil {
		t.Fatal("Escape did not close the clean detail")
	}
	other := a.state.Open(workspace.Rally, "Backlog", "", "backlog")
	a.rallyViews[other] = newRallyView(rally.FindPage("backlog"))
	keyboardFrame(h, key.CodeB, key.ModAlt)
	if a.state.Current().Page != "teamboard" {
		t.Fatal("Alt+B did not activate Team Board")
	}
}

func TestV44BoardFocusSurvivesRefreshAndSkipsHiddenItems(t *testing.T) {
	_, v := keyboardBoard(t, 4)
	v.moveCardFocus(1)
	v.moveCardFocus(1)
	ref := v.focusCard
	v.Items = cloneObjects(v.Items)
	v.Generation++
	entry, ok := v.currentBoardFocus(v.boardFocusEntries())
	if !ok || entry.card.ref != ref {
		t.Fatal("refresh lost focused identity")
	}
	v.Items = append(v.Items[:1:1], v.Items[2:]...)
	v.Generation++
	entry, ok = v.currentBoardFocus(v.boardFocusEntries())
	if !ok || entry.card.ref != v.Items[1].String("_ref") || entry.card.ref == ref {
		t.Fatal("removed card remained a keyboard target")
	}
	v.CollapsedLanes = map[string]bool{"Defined": true}
	if _, ok := v.currentBoardFocus(v.boardFocusEntries()); ok {
		t.Fatal("collapsed cards remained keyboard targets")
	}
	v.moveCardFocus(1)
	if v.cardFocusActive || !v.focusSearch {
		t.Fatal("empty board did not return focus to search")
	}
}

func TestV44KeyboardFocusRevealsVirtualCards(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a, v := keyboardBoard(t, 25)
			v.focusCard = v.Items[24].String("_ref")
			v.focusCardIndex, v.cardFocusActive, v.revealCard = 24, true, true
			var commands []command.Command
			h := desktop.NewHeadlessHarness(0, image.Pt(1100, 700), func(w *desktop.Window) {
				a.drawTeamBoard(w, v, v.filtered())
				commands = append(commands[:0], w.Commands().Commands...)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			a.window = h.Master()
			for range 4 {
				h.Frame(false)
			}
			visible := false
			for _, cmd := range commands {
				if cmd.Kind == command.TextCmd && cmd.Text.String == "US0024" && cmd.Rect.Y >= 0 && cmd.Rect.Y+cmd.Rect.H <= 700 {
					visible = true
				}
			}
			if !visible || v.revealCard {
				t.Fatal("focused virtual card was not revealed", scale, v.revealCard, v.LaneScroll)
			}
		})
	}
}

// PopupOpen installs windows asynchronously under the master lock.
func keyboardFrame(h *desktop.HeadlessHarness, code key.Code, mods key.Modifiers) {
	h.Master().Lock()
	defer h.Master().Unlock()
	h.Key(code, mods)
	h.Frame(false)
}

func TestV44SearchFocusHandsOffToBoard(t *testing.T) {
	a, v := keyboardBoard(t, 3)
	a.rallyClient, _ = rally.New("http://127.0.0.1:1", "fixture", nil)
	v.signature = a.rallySignature(v)
	v.pendingSignature = v.signature
	h := desktop.NewHeadlessHarness(0, image.Pt(1200, 1000), func(w *desktop.Window) {
		a.shortcuts(w)
		a.drawRally(w, v)
	})
	a.window = h.Master()
	h.KeyRune(key.CodeSlash, 0, '/')
	h.Frame(false)
	h.Frame(false)
	if !v.Search.Active || v.cardFocusActive || text(v.Search) != "" {
		t.Fatal("search shortcut did not acquire focus without typing a slash")
	}
	h.Key(key.CodeTab, 0)
	h.Frame(false)
	h.Frame(false)
	if v.Search.Active || !v.cardFocusActive || v.focusCard != v.Items[0].String("_ref") {
		t.Fatal("search retained focus after Tab")
	}
	h.Key(key.CodeTab, key.ModShift)
	h.Frame(false)
	h.Frame(false)
	if !v.Search.Active || v.cardFocusActive {
		t.Fatal("reverse Tab did not return to search")
	}
	h.KeyRune(key.CodeSlash, 0, '/')
	h.Frame(false)
	if text(v.Search) != "/" {
		t.Fatal("ordinary slash typing was intercepted")
	}
	if v.searchTimer != nil {
		v.searchTimer.Stop()
	}
}

func TestV44BoardFocusCheckpointAndTransfer(t *testing.T) {
	a, v := keyboardBoard(t, 3)
	tab := *a.state.Current()
	before := a.checkpointDocument(tab)
	v.moveCardFocus(1)
	v.moveCardFocus(1)
	after := a.checkpointDocument(tab)
	if before.Rally == after.Rally || before.Rally.CardFocusActive || !after.Rally.CardFocusActive || after.Rally.FocusCard != v.focusCard || after.Rally.FocusCardIndex != 1 {
		t.Fatal("focus change was not checkpointed")
	}
	b := transferFixture()
	if err := b.installTransfer(after); err != nil {
		t.Fatal(err)
	}
	restored := b.rallyViews[tab.ID]
	if restored == nil || !restored.cardFocusActive || restored.focusCard != v.focusCard || restored.focusCardIndex != 1 {
		t.Fatal("transfer lost focused card")
	}
	restored.boardFocusEntries()
	if !restored.cardFocusActive || restored.focusCard != v.focusCard {
		t.Fatal("loading empty board discarded transferred focus")
	}
}

func TestV44KeyboardFocusRevealsGroupedAndHorizontalCards(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a, v := keyboardBoard(t, 18)
			v.Group = "Owner"
			for i, item := range v.Items {
				item["Owner"] = map[string]any{"_refObjectName": fmt.Sprintf("Owner %02d", i/3)}
				item["ScheduleState"] = []string{"Defined", "In-Progress", "Completed"}[i%3]
			}
			v.focusCard = v.Items[17].String("_ref")
			v.focusCardIndex, v.cardFocusActive, v.revealCard = 17, true, true
			var commands []command.Command
			h := desktop.NewHeadlessHarness(0, image.Pt(500, 700), func(w *desktop.Window) {
				a.drawTeamBoard(w, v, v.filtered())
				commands = append(commands[:0], w.Commands().Commands...)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			a.window = h.Master()
			for range 8 {
				h.Frame(false)
			}
			visible := false
			for _, c := range commands {
				if c.Kind == command.TextCmd && c.Text.String == "US0017" && c.Rect.X >= 0 && c.Rect.X+c.Rect.W <= 500 && c.Rect.Y >= 0 && c.Rect.Y+c.Rect.H <= 700 {
					visible = true
				}
			}
			if !visible || v.revealCard {
				t.Fatal("grouped/horizontal focus was not revealed", scale, v.revealCard, v.LaneScroll)
			}
		})
	}
}
