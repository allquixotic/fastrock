package ui

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/workspace"
)

type historyCaller interface {
	Call(context.Context, string, any, any) error
}

var errHistoryLimit = errors.New("conversation text exceeds the size limit")

// Both export and the text viewer read the server's chronological history,
// independent of whether a chat is open or which transcript pages are resident.
func writeChatHistory(ctx context.Context, client historyCaller, id, title string, out io.Writer, limit int) error {
	written := 0
	write := func(value string) error {
		remaining := max(0, limit-written)
		clipped := len(value) > remaining
		if clipped {
			end := remaining
			for end > 0 && !utf8.RuneStart(value[end]) {
				end--
			}
			value = value[:end]
		}
		n, err := io.WriteString(out, value)
		written += n
		if err != nil {
			return err
		}
		if n != len(value) {
			return io.ErrShortWrite
		}
		if clipped {
			return errHistoryLimit
		}
		return nil
	}
	if err := write("# Codex conversation\n\n" + title + "\n\n"); err != nil {
		return err
	}
	cursor := ""
	seen := map[string]bool{}
	for range 10000 {
		params := map[string]any{"threadId": id, "sortDirection": "asc", "limit": 100}
		if cursor != "" {
			params["cursor"] = cursor
		}
		var page itemPage
		if err := client.Call(ctx, "thread/items/list", params, &page); err != nil {
			return err
		}
		for _, entry := range page.Data {
			block := displayBlock(formatItem(entry.Item))
			role := "Activity"
			if block.Kind == "crossTabMessage" {
				role = block.Role
			}
			switch block.Role {
			case "you":
				role = "User"
			case "assistant":
				role = "Assistant"
			}
			if err := write(fmt.Sprintf("## %s\n\n%s\n\n", role, block.Text)); err != nil {
				return err
			}
		}
		if page.Next == "" {
			return nil
		}
		if seen[page.Next] {
			return errors.New("history cursor did not advance")
		}
		seen[page.Next] = true
		cursor = page.Next
	}
	return errors.New("history exceeded the export page limit")
}

func (a *App) viewChatText(c *workspace.Conversation) {
	client := a.client
	if client == nil {
		a.toast = "Reconnect Codex before reading conversation history"
		return
	}
	id := a.state.Open(workspace.File, c.Title, "history:"+c.ID, "")
	if old := a.files[id]; old != nil {
		if old.Loading {
			return
		}
		old.dispose()
	}
	ctx, cancel := context.WithTimeout(a.ctx, 5*time.Minute)
	v := &fileView{Path: c.Title, Virtual: true, Wrap: true, Find: textEditor("", false), Loading: true, ReadCancel: cancel}
	a.files[id] = v
	thread, title := c.ID, c.Title
	a.work(func() {
		defer cancel()
		var buffer strings.Builder
		err := writeChatHistory(ctx, client, thread, title, &buffer, maxFileBytes-256)
		truncated := errors.Is(err, errHistoryLimit)
		value := buffer.String()
		if truncated {
			value = "Preview limited to the first 4 MiB. Use Export Markdown from the conversation tab for full history.\n\n" + value
			err = nil
		}
		var editor = loadedFileEditor(value)
		a.post(func() {
			if a.files[id] != v || v.Closed {
				return
			}
			v.Loading = false
			v.ReadCancel = nil
			if client != a.client {
				v.Error = "Codex reconnected; choose View as text again to read current history"
				return
			}
			if err != nil {
				v.Error = "Could not read conversation history: " + err.Error()
				return
			}
			v.Editor = editor
			v.Loaded = true
			v.LoadedText = value
			v.LimitReached = truncated
		})
	}, func() { cancel(); v.ReadCancel = nil; v.Loading = false; v.Error = errWorkQueueFull.Error() })
}
