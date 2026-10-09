package ui

import (
	"context"
	"fmt"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type itemPage struct {
	Data []struct {
		Item map[string]any `json:"item"`
	} `json:"data"`
	Next string `json:"nextCursor"`
}

func readItemPage(ctx context.Context, client *codex.Client, id, cursor string) (itemPage, error) {
	var page itemPage
	params := map[string]any{"threadId": id, "limit": 100, "sortDirection": "desc"}
	if cursor != "" {
		params["cursor"] = cursor
	}
	err := client.Call(ctx, "thread/items/list", params, &page)
	return page, err
}
func (a *App) prepareItemPage(page itemPage) []workspace.Block {
	c := &workspace.Conversation{}
	for i := len(page.Data) - 1; i >= 0; i-- {
		a.upsertItem(c, page.Data[i].Item)
	}
	return c.Blocks
}

func (a *App) olderMessages(c *workspace.Conversation, v *chatView) {
	if v.HistoryLoading || v.HistoryCursor == "" || a.client == nil {
		return
	}
	v.HistoryLoading = true
	client, cursor := a.client, v.HistoryCursor
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		page, err := readItemPage(ctx, client, c.ID, cursor)
		blocks := a.prepareItemPage(page)
		a.post(func() {
			if a.client != client || a.chats[c.ID] != v {
				return
			}
			v.HistoryLoading = false
			if err != nil {
				v.HistoryError = fmt.Sprintf("Could not load earlier messages: %v", err)
				return
			}
			v.HistoryError = ""
			v.HistoryCursor = page.Next
			if page.Next == cursor {
				v.HistoryCursor = ""
			}
			merged := mergeTranscript(blocks, c.Blocks)
			// Browsing older history keeps that end of the window. Newest history
			// remains available by reloading the latest page.
			bytes, end := 0, 0
			for end < len(merged) && end < workspace.MaxTranscriptBlocks && bytes+len(merged[end].Text) <= workspace.MaxTranscriptBytes {
				bytes += len(merged[end].Text)
				end++
			}
			c.Blocks = append([]workspace.Block(nil), merged[:end]...)
			v.HistoryWindowed = v.HistoryWindowed || end < len(merged)
			c.TrimTranscript()
			v.ShowBlocks = len(c.Blocks)
			v.Follow = false
			v.PreviousScroll = -1
		})
	}, func() { v.HistoryLoading = false; v.HistoryError = errWorkQueueFull.Error() })
}

func (a *App) latestMessages(c *workspace.Conversation, v *chatView) {
	if v.HistoryLoading || a.client == nil {
		return
	}
	v.HistoryLoading = true
	client := a.client
	before := make(map[string]bool, len(c.Blocks))
	for _, b := range c.Blocks {
		before[b.ID] = true
	}
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		page, err := readItemPage(ctx, client, c.ID, "")
		blocks := a.prepareItemPage(page)
		a.post(func() {
			if a.client != client || a.chats[c.ID] != v {
				return
			}
			v.HistoryLoading = false
			if err != nil {
				v.HistoryError = fmt.Sprintf("Could not load latest messages: %v", err)
				return
			}
			pageIDs := map[string]bool{}
			for _, b := range blocks {
				pageIDs[b.ID] = true
			}
			var live []workspace.Block
			for _, b := range c.Blocks {
				if !before[b.ID] || pageIDs[b.ID] {
					live = append(live, b)
				}
			}
			c.Blocks = mergeTranscript(blocks, live)
			c.TrimTranscript()
			v.HistoryCursor, v.HistoryError = page.Next, ""
			v.HistoryWindowed, v.Follow = false, true
			v.ShowBlocks = len(c.Blocks)
		})
	}, func() { v.HistoryLoading = false; v.HistoryError = errWorkQueueFull.Error() })
}
