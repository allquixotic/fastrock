package ui

import (
	"fmt"
	"image/color"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type statusDot struct {
	Color  color.RGBA
	Hollow bool
}

func (a *App) tabDot(tab workspace.Tab) statusDot {
	if tab.Kind != workspace.Chat {
		return statusDot{}
	}
	return a.conversationDot(a.state.Chats[tab.Target], true)
}

func (a *App) conversationDot(c *workspace.Conversation, open bool) statusDot {
	if c == nil {
		return statusDot{}
	}
	switch {
	case c.Status == "error":
		return statusDot{Color: a.p.Danger}
	case a.activeApprovalFor(c.ID):
		return statusDot{Color: a.p.Warning}
	case c.Status == "starting":
		return statusDot{Color: a.p.Faint}
	case c.Busy():
		return statusDot{Color: a.p.Accent}
	case c.Unread:
		return statusDot{Color: a.p.Success}
	case open:
		return statusDot{Color: a.p.BorderStrong, Hollow: true}
	default:
		return statusDot{}
	}
}

type openChatIndex struct {
	first    *workspace.Tab
	length   int
	revision uint64
	ids      map[string]bool
}

func (a *App) chatIsOpen(id string) bool {
	var first *workspace.Tab
	if len(a.state.Tabs) > 0 {
		first = &a.state.Tabs[0]
	}
	cache := &a.openChatIndex
	if cache.ids == nil || cache.first != first || cache.length != len(a.state.Tabs) || cache.revision != a.state.TabRevision {
		cache.first, cache.length, cache.revision = first, len(a.state.Tabs), a.state.TabRevision
		cache.ids = make(map[string]bool, len(a.state.Tabs))
		for _, tab := range a.state.Tabs {
			if tab.Kind == workspace.Chat {
				cache.ids[tab.Target] = true
			}
		}
	}
	return cache.ids[id]
}

func conversationAge(updated int64, now time.Time) string {
	if updated <= 0 {
		return ""
	}
	seconds := max(int64(0), now.Unix()-updated)
	for _, unit := range []struct {
		seconds int64
		suffix  string
	}{
		{365 * 24 * 60 * 60, "y"}, {30 * 24 * 60 * 60, "mo"}, {7 * 24 * 60 * 60, "w"},
		{24 * 60 * 60, "d"}, {60 * 60, "h"}, {60, "m"},
	} {
		if seconds >= unit.seconds {
			return fmt.Sprintf("%d%s", seconds/unit.seconds, unit.suffix)
		}
	}
	return "now"
}

func (a *App) drawTabOverflow(w *nucular.Window) bool {
	scale := w.Master().Style().Scaling
	row, spacing := int(30*scale), w.Master().Style().GroupWindow.Spacing.Y
	stride := row + spacing
	height := min(int(400*scale), max(row, w.LayoutAvailableHeight()), len(a.state.Tabs)*stride+int(12*scale))
	w.RowScaled(height).Dynamic(1)
	selected := false
	if group := w.GroupBegin("tab-overflow", nucular.WindowNoHScrollbar); group != nil {
		selected = a.drawTabOverflowList(group)
		group.GroupEnd()
	}
	return selected
}

func (a *App) drawTabOverflowList(group *nucular.Window) bool {
	row := int(30 * group.Master().Style().Scaling)
	spacing := group.Master().Style().GroupWindow.Spacing.Y
	stride := row + spacing
	group.RowScaled(row).Dynamic(1)
	top := group.WidgetBounds().Y
	first, last := sidebarVisible(top, group.Bounds.Y, group.Bounds.Y+group.Bounds.H, stride, len(a.state.Tabs))
	sidebarSkip(group, first, stride, spacing)
	selected := false
	for _, tab := range a.state.Tabs[first:last] {
		group.RowScaled(row).Dynamic(1)
		dot := a.tabDot(tab)
		if flatStatusRow(group, a.tabTitle(tab), "", tab.ID == a.state.Active, dot, 12, a.p) {
			a.state.Active = tab.ID
			selected = true
		}
	}
	sidebarSkip(group, len(a.state.Tabs)-last, stride, spacing)
	return selected
}
