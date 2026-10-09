package ui

import (
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/key"
)

type boardFocusEntry struct {
	card               *boardCard
	group, lane, index int
}

func (v *rallyView) boardFocusEntries() []boardFocusEntry {
	v.prepareCards()
	v.prepareBoardLayout(v.filtered())
	var entries []boardFocusEntry
	for gi, group := range v.boardGroups {
		for li, lane := range group.lanes {
			if v.CollapsedLanes[lane.state] {
				continue
			}
			for i, c := range lane.cards {
				entries = append(entries, boardFocusEntry{c, gi, li, i})
			}
		}
	}
	return entries
}

func (v *rallyView) currentBoardFocus(entries []boardFocusEntry) (boardFocusEntry, bool) {
	if !v.cardFocusActive || len(entries) == 0 {
		return boardFocusEntry{}, false
	}
	for i, entry := range entries {
		if entry.card.ref == v.focusCard {
			v.focusCardIndex = i
			return entry, true
		}
	}
	// A refresh can remove or filter the focused item. Keep focus at the nearest
	// remaining position instead of accidentally opening the old object.
	i := min(max(0, v.focusCardIndex), len(entries)-1)
	v.focusCard, v.focusCardIndex, v.revealCard = entries[i].card.ref, i, true
	return entries[i], true
}

func (v *rallyView) moveCardFocus(direction int) {
	entries := v.boardFocusEntries()
	if len(entries) == 0 {
		v.cardFocusActive = false
		v.focusSearch = true
		return
	}
	index := 0
	if direction < 0 {
		index = len(entries) - 1
	}
	if _, ok := v.currentBoardFocus(entries); ok && !v.Search.Active && !v.Query.Active {
		index = v.focusCardIndex + direction
		if index < 0 || index >= len(entries) {
			v.cardFocusActive, v.focusSearch = false, true
			return
		}
	}
	v.Search.Active, v.Query.Active = false, false
	v.focusCard, v.focusCardIndex = entries[index].card.ref, index
	v.cardFocusActive, v.revealCard, v.focusSearch = true, true, false
}

func (a *App) currentRally() *rallyView {
	if a.state == nil {
		return nil
	}
	if tab := a.state.Current(); tab != nil && tab.Kind == workspace.Rally {
		return a.rallyViews[tab.ID]
	}
	return nil
}

func (a *App) rallyShortcutApplies(id string, code key.Code, mods key.Modifiers) bool {
	v := a.currentRally()
	if v == nil || v.Closed {
		return false
	}
	switch id {
	case "rally-team-board":
		return true
	case "rally-close-detail":
		return v.Detail != nil
	case "rally-search":
		return v.Detail == nil && (code != key.CodeSlash || mods != 0 || !v.Search.Active && !v.Query.Active)
	case "rally-next-card", "rally-prev-card":
		return v.Detail == nil && v.Mode == "board"
	case "rally-open-card":
		if v.Detail != nil || v.Mode != "board" || v.Search.Active || v.Query.Active {
			return false
		}
		_, ok := v.currentBoardFocus(v.boardFocusEntries())
		return ok
	}
	return false
}

func (a *App) runRallyAction(id string) bool {
	if !strings.HasPrefix(id, "rally-") {
		return false
	}
	v := a.currentRally()
	if v == nil || v.Closed {
		return true
	}
	switch id {
	case "rally-team-board":
		a.openRally("teamboard")
	case "rally-search":
		if v.Detail == nil {
			v.cardFocusActive, v.focusSearch = false, true
		}
	case "rally-close-detail":
		a.closeRallyDetail(v, false)
	case "rally-next-card":
		v.moveCardFocus(1)
	case "rally-prev-card":
		v.moveCardFocus(-1)
	case "rally-open-card":
		if entry, ok := v.currentBoardFocus(v.boardFocusEntries()); ok && v.Detail == nil {
			a.openArtifact(v, entry.card.object)
		}
	}
	return true
}

func (a *App) closeRallyDetail(v *rallyView, history bool) {
	d := v.Detail
	if d == nil {
		return
	}
	if d.Saving {
		a.toast = "Wait for this work item to finish saving"
		return
	}
	a.leaveDetail(v, func() {
		if v.Detail != d {
			return
		}
		a.disposeDetail(d)
		v.Detail = nil
		if history && len(v.DetailHistory) > 0 {
			n := len(v.DetailHistory) - 1
			o := v.DetailHistory[n]
			v.DetailHistory = v.DetailHistory[:n]
			v.historyRevision++
			a.openArtifactNow(v, o, false)
		} else {
			v.DetailHistory = nil
			v.historyRevision++
			v.revealCard = v.cardFocusActive
		}
	})
}

func actionsOverlap(a, b string) bool {
	return !(a == "rally-close-detail" && b == "escape" || b == "rally-close-detail" && a == "escape")
}

func validateActionShortcut(id string, code key.Code, mods key.Modifiers) string {
	if id == "rally-search" && code == key.CodeSlash && mods == 0 {
		return ""
	}
	return validateShortcut(code, mods)
}

// Revealing a focus target uses actual pixel geometry. It works for both the
// outer grouped board and inner card lane, including scaled row heights.
func revealBoardRange(w *desktop.Window, top, bottom int) bool {
	clip := w.Commands().Clip
	viewTop := max(w.Bounds.Y, clip.Y)
	viewBottom := min(w.Bounds.Y+w.Bounds.H, clip.Y+clip.H)
	if viewBottom <= viewTop {
		return false
	}
	bottom = min(bottom, top+viewBottom-viewTop)
	if top < viewTop {
		w.Scrollbar.Y = max(0, w.Scrollbar.Y+top-viewTop)
		w.Master().Changed()
		return false
	}
	if bottom > viewBottom {
		w.Scrollbar.Y += bottom - viewBottom
		w.Master().Changed()
		return false
	}
	return true
}
