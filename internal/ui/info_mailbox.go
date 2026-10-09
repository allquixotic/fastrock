package ui

import (
	"fmt"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func (a *App) setMessagesAllowed(c *workspace.Conversation, allow bool) {
	c.NoMessages = !allow
	if !allow {
		a.declineConsents(c.ID, "The target conversation turned off messaging")
	}
	a.publishThreads()
}
func (a *App) recordMail(id string, message mailMessage) {
	if id == "" {
		return
	}
	if a.mailbox == nil {
		a.mailbox = map[string][]mailMessage{}
	}
	messages := append(a.mailbox[id], message)
	a.mailbox[id] = messages[max(0, len(messages)-50):]
}
func (a *App) mailPeer(c *workspace.Conversation, m mailMessage) (string, string, string) {
	direction, peer := "From", m.From
	if m.From == c.ID {
		direction, peer = "To", m.To
	}
	name := peer
	if other := a.state.Chats[peer]; other != nil {
		name = other.Title
	}
	return direction, peer, name
}
func (a *App) drawMailbox(w *nucular.Window, c *workspace.Conversation) {
	messages := a.mailbox[c.ID]
	for i := len(messages) - 1; i >= 0; i-- {
		m := messages[i]
		direction, peer, name := a.mailPeer(c, m)
		w.Row(27).Dynamic(1)
		if w.ButtonText(direction + " " + cut(name, 50)) {
			a.openInfoPeer(peer, name, "")
		}
		if m.At > 0 {
			muted(w, time.Unix(m.At, 0).Local().Format("Jan 2 · 15:04"), a.p)
		}
		// The preview is bounded; opening the message preserves its complete text.
		w.Row(50).Dynamic(1)
		w.LabelWrap(cut(m.Text, 250))
		if len([]rune(m.Text)) > 250 {
			w.Row(25).Dynamic(1)
			if w.ButtonText("Read full message") {
				a.openText(fmt.Sprintf("Message %s %s", direction, name), m.Text)
			}
		}
	}
}
