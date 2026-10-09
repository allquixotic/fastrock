package codex

import (
	"encoding/xml"
	"fmt"
	"strings"
)

// WrapAgentMessage keeps the literal payload separate from its transport
// metadata. The delivery ID correlates queue consumption and reply observation.
func WrapAgentMessage(from, title, message string, reply bool, id string) string {
	escape := func(s string) string {
		var b strings.Builder
		_ = xml.EscapeText(&b, []byte(s))
		return b.String()
	}
	short := []rune(strings.Join(strings.Fields(title), " "))
	if len(short) > 60 {
		short = append(short[:60], '…')
	}
	return fmt.Sprintf("Message from the agent in tab %q:\n<codex_gui_message>\n  <from_thread_id>%s</from_thread_id>\n  <from_title>%s</from_title>\n  <reply_expected>%t</reply_expected>\n  <delivery_id>%s</delivery_id>\n  <message>%s</message>\n</codex_gui_message>", string(short), escape(from), escape(title), reply, escape(id), escape(message))
}
