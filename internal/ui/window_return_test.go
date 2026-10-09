//go:build fltk_headless

package ui

import (
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func returnFixture(t *testing.T, withDestination bool) (*App, *App) {
	t.Helper()
	source := reviewFixture(t)
	b, err := codex.NewBroker(source.client)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(b.Close)
	client, err := codex.Dial(source.ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(client.Close)
	source.client = client
	source.connection.Ticket = "original-popout"
	source.files = map[string]*fileView{}
	source.rallyViews = map[string]*rallyView{}
	source.popping = map[string]bool{}
	source.transfers = map[string]string{}
	t.Cleanup(source.stopTransferTimers)
	if !withDestination {
		return source, nil
	}
	destination := transferFixture()
	destination.ctx = source.ctx
	destination.updates = make(chan func(), 128)
	destination.p = colors(false)
	destination.client, err = codex.Dial(source.ctx, b.Address(), b.Token())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(destination.client.Close)
	if err := destination.client.Call(source.ctx, "fastrock/name", map[string]string{"name": "Fastrock"}, nil); err != nil {
		t.Fatal(err)
	}
	return source, destination
}

func pumpTransferWindows(t *testing.T, done func() bool, apps ...*App) {
	t.Helper()
	deadline := time.After(5 * time.Second)
	ticker := time.NewTicker(time.Millisecond)
	defer ticker.Stop()
	for !done() {
		for _, a := range apps {
		updates:
			for range 64 {
				select {
				case f := <-a.updates:
					f()
				default:
					break updates
				}
			}
		events:
			for range 64 {
				select {
				case event, ok := <-a.client.Events:
					if !ok {
						break events
					}
					a.transferEvent(event)
				default:
					break events
				}
			}
			if ticket := a.readyTicket; ticket != "" {
				a.readyTicket = ""
				a.rpc("fastrock/ready", map[string]string{"ticket": ticket}, nil)
			}
		}
		select {
		case <-deadline:
			t.Fatal("window transfer did not finish")
		case <-ticker.C:
		}
	}
}

func TestV28ClosedPopoutReturnsDocumentsImmediately(t *testing.T) {
	source, dest := returnFixture(t, true)
	c := &workspace.Conversation{ID: "chat", Title: "Chat", Draft: "unsent draft", Outbox: []workspace.Draft{{ID: "pending", Text: "uncertain send", Status: "unconfirmed"}}, Queue: []workspace.Draft{{ID: "queued", Text: "queued draft"}}, QueuePaused: true}
	source.state.Chats[c.ID] = c
	source.state.Open(workspace.Chat, c.Title, c.ID, "")
	source.rememberDraft(c.ID)
	if err := source.client.Call(source.ctx, "fastrock/own", map[string]string{"threadId": c.ID}, nil); err != nil {
		t.Fatal(err)
	}
	fileID := source.state.Open(workspace.File, "Patch", "virtual:patch", "")
	source.files[fileID] = &fileView{Path: "virtual:patch", Virtual: true, Editor: textEditor("file contents", true)}
	boardID := source.state.Open(workspace.Rally, "Board", "", "teamboard")
	v := newRallyView(rally.FindPage("teamboard"))
	v.Detail = makeDetail(rally.Object{"Name": "saved"}, "HierarchicalRequirement", false)
	setText(v.Detail.Editors["Name"], "unsaved edit")
	source.rallyViews[boardID] = v
	// The real close guard obtains the user's decision about dirty/running work
	// first. Exercise the handoff after that decision without displaying UI.
	if source.beginWindowClose() {
		t.Fatal("closed before handoff")
	}
	pumpTransferWindows(t, func() bool { return source.allowWindowClose }, source, dest)
	if len(source.state.Tabs) != 0 || len(dest.state.Tabs) != 3 || source.windowReturn != nil {
		t.Fatal("documents not moved before close")
	}
	got := dest.state.Chats["chat"]
	if got == nil || got.Draft != "unsent draft" || len(got.Outbox) != 1 || len(got.Queue) != 1 || !got.QueuePaused {
		t.Fatal("chat state lost")
	}
	var fileOK, boardOK bool
	for _, v := range dest.files {
		fileOK = fileOK || text(v.Editor) == "file contents"
	}
	for _, v := range dest.rallyViews {
		boardOK = boardOK || v.Detail != nil && text(v.Detail.Editors["Name"]) == "unsaved edit"
	}
	if !fileOK || !boardOK {
		t.Fatal("document content lost")
	}
	if _, exists := source.sessionSnapshot().Chats["chat"]; exists {
		t.Fatal("moved draft persisted a second time")
	}
}

func TestV28ReturnFailureKeepsWindowAndRemainingDocuments(t *testing.T) {
	source, _ := returnFixture(t, true)
	id := source.state.Open(workspace.File, "File", "virtual:file", "")
	source.files[id] = &fileView{Path: "virtual:file", Virtual: true, Editor: textEditor("retained", true)}
	source.state.Open(workspace.File, "Other", "other", "")
	if source.canCloseWindow() {
		t.Fatal("closed before returning")
	}
	// Leave the destination unresponsive, then cancel as the watchdog would.
	drain(t, source, func() bool { return source.windowReturn != nil && source.windowReturn.ticket != "" })
	ticket := source.windowReturn.ticket
	source.abortTransfer(id, ticket, errors.New("destination unavailable"))
	drain(t, source, func() bool { return source.windowReturn == nil })
	if source.allowWindowClose || len(source.state.Tabs) != 2 || text(source.files[id].Editor) != "retained" || source.popping[id] {
		t.Fatal("failure discarded source or closed window")
	}
	if !strings.Contains(source.toast, "destination unavailable") {
		t.Fatal(source.toast)
	}
}

func TestV28LastWindowClosesWithLocalRecovery(t *testing.T) {
	source, _ := returnFixture(t, false)
	source.state.Open(workspace.File, "Local", "local", "")
	if source.canCloseWindow() {
		t.Fatal("window discovery skipped")
	}
	drain(t, source, func() bool { return source.allowWindowClose })
	if len(source.state.Tabs) != 1 || source.windowReturn != nil {
		t.Fatal("last-window recovery state lost")
	}
}

func TestV28ReturnDestinationPrefersMain(t *testing.T) {
	if got := returnDestination([]returnWindow{{"z", "Pop-out"}, {"b", "Fastrock"}, {"a", "Pop-out"}}); got != "b" {
		t.Fatal(got)
	}
	if got := returnDestination([]returnWindow{{"z", "Pop-out"}, {"a", "Pop-out"}}); got != "a" {
		t.Fatal(got)
	}
}

func TestV28ReturnNeverOverwritesDestinationDraft(t *testing.T) {
	source, dest := returnFixture(t, true)
	c := &workspace.Conversation{ID: "same", Title: "Chat", Draft: "source draft"}
	source.state.Chats[c.ID] = c
	source.state.Open(workspace.Chat, c.Title, c.ID, "")
	dest.state.Chats[c.ID] = &workspace.Conversation{ID: c.ID, Draft: "destination draft"}
	if source.beginWindowClose() {
		t.Fatal("closed before handoff")
	}
	pumpTransferWindows(t, func() bool { return source.windowReturn == nil }, source, dest)
	if source.allowWindowClose || len(source.state.Tabs) != 1 || source.state.Chats[c.ID].Draft != "source draft" || dest.state.Chats[c.ID].Draft != "destination draft" {
		t.Fatal("conflicting drafts were lost")
	}
}

func TestV28ReturningDocumentsCannotBeClosedAgain(t *testing.T) {
	a := transferFixture()
	id := a.state.Open(workspace.File, "Retained", "file", "")
	a.windowReturn = &windowReturn{}
	a.closeTab(id)
	a.closeTabs([]string{id})
	if len(a.state.Tabs) != 1 {
		t.Fatal("returning document was removed before acknowledgement")
	}
}
