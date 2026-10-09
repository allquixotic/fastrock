package ui

import (
	"context"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/workspace"
)

// A picker owns its request lifetime. Immutable metadata can be shared with the
// sidebar, but its query, cancellation and results stay independent.
type conversationPicker struct {
	query                             string
	root                              *sidebarNode
	rows                              []*sidebarRow
	generation, failedGeneration      uint64
	requested, working, ready, closed bool
	cancel                            context.CancelFunc
	err                               string
}

func (p *conversationPicker) close() {
	p.closed = true
	if p.cancel != nil {
		p.cancel()
	}
	p.rows, p.root = nil, nil
}

func (p *conversationPicker) retry() {
	p.requested = false
}

func (a *App) prepareConversationPicker(p *conversationPicker, query string) {
	if p.closed {
		return
	}
	a.sidebarFolders() // Admit bounded metadata batches even with sidebar hidden.
	c := &a.sidebarCache
	if !p.requested || p.query != query || p.root != c.root {
		p.requested, p.ready, p.query, p.root = true, false, query, c.root
		p.generation++
		p.err = ""
		if p.cancel != nil {
			p.cancel()
		}
	}
	if !c.indexed || c.scan != nil || len(c.dirty) > 0 || p.ready || p.working || p.failedGeneration == p.generation {
		return
	}
	ctx := a.ctx
	if ctx == nil {
		ctx = context.Background()
	}
	ctx, cancel := context.WithCancel(ctx)
	p.cancel, p.working = cancel, true
	root, generation := p.root, p.generation
	a.work(func() {
		defer cancel()
		rows, _, err := querySidebar(ctx, root, query, false, nil)
		a.post(func() {
			p.working = false
			if p.closed || generation != p.generation {
				return
			}
			if err != nil {
				p.err, p.failedGeneration = err.Error(), generation
				return
			}
			p.rows, p.ready = rows, true
		})
	}, func() {
		p.working = false
		if !p.closed && generation == p.generation {
			p.err, p.failedGeneration = errWorkQueueFull.Error(), generation
		}
	})
}

func (a *App) chooseConversation(pick func(*workspace.Conversation)) {
	ed := textEditor("", false)
	ed.Placeholder = "Search conversations"
	p := &conversationPicker{}
	a.window.PopupOpen("Choose conversation", desktop.WindowTitle|desktop.WindowClosable, a.modalBounds(600, 500), false, func(w *desktop.Window) {
		a.drawConversationPicker(w, p, ed, pick)
	})
}

func (a *App) drawConversationPicker(w *desktop.Window, p *conversationPicker, ed *desktop.TextEditor, pick func(*workspace.Conversation)) {
	w.OnClose(p.close)
	w.Row(30).Dynamic(1)
	ed.Edit(w)
	a.prepareConversationPicker(p, text(ed))
	if p.err != "" {
		muted(w, "Could not prepare conversations: "+p.err, a.p)
		w.Row(26).Dynamic(1)
		if w.ButtonText("Retry") {
			p.retry()
		}
	} else if !p.ready {
		muted(w, "Preparing conversations…", a.p)
	} else if len(p.rows) == 0 {
		muted(w, "No conversations match your search", a.p)
	}
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	stride := int(30*w.Master().Style().Scaling) + spacing
	start, end := sidebarVisible(w.LayoutNextRowY(), w.Bounds.Y, w.Bounds.Y+w.Bounds.H, stride, len(p.rows))
	sidebarSkip(w, start, stride, spacing)
	for _, row := range p.rows[start:end] {
		w.Row(30).Dynamic(1)
		if w.ButtonText(row.Title) {
			if c := a.state.Chats[row.ID]; c != nil {
				pick(c)
				w.Close()
			}
		}
	}
	sidebarSkip(w, len(p.rows)-end, stride, spacing)
}
