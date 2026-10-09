package ui

import (
	"encoding/xml"
	"io"
	"strings"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type agentEnvelope struct {
	XMLName       xml.Name `xml:"codex_gui_message"`
	FromID        string   `xml:"from_thread_id"`
	FromTitle     string   `xml:"from_title"`
	ReplyExpected bool     `xml:"reply_expected"`
	DeliveryID    string   `xml:"delivery_id"`
	Message       string   `xml:"message"`
	// Legacy Fastrock messages put the sender in an attribute and the message
	// directly in the root element. Keep their history readable too.
	LegacyFrom    string `xml:"sender_thread_id,attr"`
	LegacyMessage string `xml:",chardata"`
}

func oneLineTitle(title string) string { return cut(strings.Join(strings.Fields(title), " "), 60) }
func wrapAgentMessage(from, title, message string, reply bool) string {
	return codex.WrapAgentMessage(from, title, message, reply, workspace.NewID("delivery"))
}
func parseAgentMessage(text string) (agentEnvelope, bool) {
	var value agentEnvelope
	if len(text) > 64<<10 {
		return value, false
	}
	body := strings.TrimSpace(text)
	if strings.HasPrefix(body, "Message from the agent in tab ") {
		line, rest, ok := strings.Cut(body, "\n")
		if !ok || !strings.HasSuffix(line, ":") {
			return value, false
		}
		body = rest
	}
	if !strings.HasPrefix(body, "<codex_gui_message") {
		return value, false
	}
	decoder := xml.NewDecoder(strings.NewReader(body))
	if err := decoder.Decode(&value); err != nil {
		return value, false
	}
	// Do not silently hide text following an apparent envelope.
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return value, false
		}
		if ch, ok := token.(xml.CharData); !ok || strings.TrimSpace(string(ch)) != "" {
			return value, false
		}
	}
	if value.FromID == "" && value.LegacyFrom != "" {
		value.FromID = value.LegacyFrom
		value.Message = strings.TrimSpace(value.LegacyMessage)
	}
	if value.FromID == "" {
		return value, false
	}
	if value.FromTitle == "" {
		value.FromTitle = value.FromID
	}
	return value, true
}
func displayBlock(block workspace.Block) workspace.Block {
	if block.Kind != "userMessage" {
		return block
	}
	envelope, ok := parseAgentMessage(block.Text)
	if !ok {
		return block
	}
	block.Kind = "crossTabMessage"
	block.Role = "Message from tab “" + oneLineTitle(envelope.FromTitle) + "”"
	block.Text = envelope.Message
	if envelope.ReplyExpected {
		block.Text = "Reply expected\n\n" + block.Text
	}
	return block
}
