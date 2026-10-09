package ui

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/platform"
	"maps"
	"os"
	"os/exec"
	"slices"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type Connection struct {
	Notice                 string
	Client                 *codex.Client
	Address, Token, Ticket string
}
type editorPosition struct{ Cursor, Start, End, ScrollX, ScrollY int }

func position(e *nucular.TextEditor) editorPosition {
	if e == nil {
		return editorPosition{}
	}
	return editorPosition{Cursor: e.Cursor, Start: e.SelectStart, End: e.SelectEnd, ScrollX: e.Scrollbar.X, ScrollY: e.Scrollbar.Y}
}
func (p editorPosition) apply(e *nucular.TextEditor) {
	if e != nil {
		e.Cursor = max(0, min(p.Cursor, len(e.Buffer)))
		e.SelectStart = max(0, min(p.Start, len(e.Buffer)))
		e.SelectEnd = max(0, min(p.End, len(e.Buffer)))
		e.Scrollbar.X, e.Scrollbar.Y = max(0, p.ScrollX), max(0, p.ScrollY)
	}
}

type fileTransfer struct {
	Stamp               fileStamp
	FileNotice          string
	LimitReached        bool
	Lossy               bool
	DiffCollapsed       []string
	DiffSelection       diffSelection
	Diff                bool
	BasePath            string
	Text, Find          string
	Virtual, Wrap, More bool
	TextLoaded          bool
	Offset              int64
	Position            editorPosition
	ScrollX, ScrollY    int
}
type chatTransfer struct {
	Position           editorPosition
	Follow             bool
	Scroll, ShowBlocks int
	Expanded           map[string]bool
	QueuePage          int
}
type tabTransfer struct {
	SchemaVersion int `json:"schemaVersion,omitempty"`
	Mailbox       []mailMessage
	File          *fileTransfer
	ChatView      *chatTransfer
	Text          string
	Tab           workspace.Tab
	Chat          *workspace.Conversation
	Rally         *rallyTransfer
}
type rallyTransfer struct {
	TimeboxName, ReleaseTimebox, ReleaseName           string
	Display                                            *settings.BoardDisplay  `json:",omitempty"`
	DisplayDraft                                       *boardDisplayDraftState `json:",omitempty"`
	BoardStride, BoardHeader                           int
	QueryApplied                                       *string                `json:",omitempty"`
	StructuredFilters                                  []settings.RallyFilter `json:",omitempty"`
	FilterDraft                                        *rallyFilterDraftState `json:",omitempty"`
	CurrentIteration                                   bool
	ResidentStart, ResidentCount, Total                int
	Filters, ExitAgreements, Rules, ShowFields, AIView bool
	CardFields                                         []string
	CollapsedLanes                                     map[string]bool
	ViewName                                           string
	Widgets                                            bool
	Sort                                               string
	Descending                                         bool
	Selected                                           map[string]rally.Object
	History                                            []rally.Object
	Mode, Group, Timebox, Query, Search, Owner, State  string
	OnlyBlocked, OnlyReady                             bool
	Columns                                            []string
	LaneScroll                                         map[string]int
	FocusCard                                          string
	FocusCardIndex                                     int
	CardFocusActive                                    bool
	ListPage                                           int
	Detail                                             *detailTransfer
}
type detailTransfer struct {
	InlineField, InlineScope string         `json:",omitempty"`
	Selection                *selectionEdit `json:",omitempty"`
	Conflict                 bool
	FieldConflicts           []detailFieldConflict
	ReferenceLabels          map[string]string
	Positions                map[string]editorPosition
	RichViews                map[string]richPresentation
	CommentView              richPresentation
	Fields                   []rally.Field
	States                   []rally.Object
	Rich                     map[string]richtext.Saved
	CommentDocument          richtext.Saved
	Original                 rally.Object
	Kind, Tab, Comment       string
	New                      bool
	PendingDefaultOwner      bool
	Values                   map[string]string
}

func (a *App) tabSnapshot(t workspace.Tab) tabTransfer {
	return a.tabSnapshotWithContent(t, true)
}
func (a *App) tabSnapshotWithContent(t workspace.Tab, includeContent bool) tabTransfer {
	p := tabTransfer{SchemaVersion: transferSchemaVersion, Tab: t}
	if v := a.files[t.ID]; v != nil && (v.Editor != nil || v.Loaded || v.DiffSource != "") {
		pos := v.filePosition()
		p.File = &fileTransfer{Diff: len(v.Diff) > 0, BasePath: v.BasePath, TextLoaded: includeContent, Find: text(v.Find), Virtual: v.Virtual, Wrap: v.Wrap, More: v.More, Offset: v.Offset, Position: pos, ScrollX: pos.ScrollX, ScrollY: pos.ScrollY, Stamp: v.Stamp, FileNotice: v.FileNotice, LimitReached: v.LimitReached, Lossy: v.Lossy}
		if includeContent {
			p.File.Text = v.fileText()
		}
		if v.DiffSource != "" {
			p.File.Diff = true
			p.File.ScrollX, p.File.ScrollY = v.DiffScroll.X, v.DiffScroll.Y
			p.File.DiffSelection = v.DiffSelection
			for _, f := range v.Diff {
				if f.Collapsed {
					p.File.DiffCollapsed = append(p.File.DiffCollapsed, f.displayPath())
				}
			}
			if includeContent {
				p.File.Text = v.DiffSource
			}
		}
	}
	if c := a.state.Chats[t.Target]; c != nil {
		copy := *c
		copy.Blocks = append([]workspace.Block(nil), c.Blocks...)
		copy.Agents = slices.Clone(c.Agents)
		copy.Queue = cloneQueue(c.Queue)
		copy.Outbox = cloneQueue(c.Outbox)
		copy.QueueDraft.Attachments = append([]string(nil), c.QueueDraft.Attachments...)
		copy.DraftAttachments = append([]string(nil), c.DraftAttachments...)
		if v := a.chats[c.ID]; v != nil {
			copy.Draft = text(v.Editor)
			p.ChatView = &chatTransfer{Position: position(v.Editor), Follow: v.Follow, Scroll: v.Scroll, ShowBlocks: v.ShowBlocks, Expanded: maps.Clone(v.Expanded), QueuePage: v.QueuePage}
			copy.DraftAttachments = append([]string(nil), v.Attachments...)
		}
		p.Chat = &copy
		p.Mailbox = append([]mailMessage(nil), a.mailbox[c.ID]...)
	}
	if v := a.rallyViews[t.ID]; v != nil {
		r := &rallyTransfer{Mode: v.Mode, Group: v.Group, Timebox: v.Timebox, TimeboxName: v.TimeboxName, ReleaseTimebox: v.ReleaseTimebox, ReleaseName: v.ReleaseName, Query: text(v.Query), Search: text(v.Search), Owner: v.OwnerFilter, State: v.StateFilter, OnlyBlocked: v.OnlyBlocked, OnlyReady: v.OnlyReady, Columns: append([]string(nil), v.Columns...)}
		applied := v.QueryApplied
		r.QueryApplied = &applied
		r.Display = v.Display.Copy()
		r.DisplayDraft = v.DisplayDraft.snapshot()
		r.BoardStride, r.BoardHeader = v.boardStride, v.boardHeader
		r.StructuredFilters = slices.Clone(v.StructuredFilters)
		r.FilterDraft = v.FilterDraft.snapshot()
		r.CurrentIteration = v.CurrentIteration
		r.ListPage = v.Page
		r.ResidentStart, r.ResidentCount, r.Total = v.Start, max(len(v.Items), v.RestoreCount), v.Total
		r.Filters, r.ExitAgreements, r.Rules, r.ShowFields, r.AIView = v.Filters, v.ExitAgreements, v.Rules, v.ShowFields, v.AIView
		r.CardFields = slices.Clone(v.CardFields)
		r.CollapsedLanes = maps.Clone(v.CollapsedLanes)
		r.ViewName, r.Widgets = v.ViewName, v.Widgets
		r.FocusCard, r.FocusCardIndex, r.CardFocusActive = v.focusCard, v.focusCardIndex, v.cardFocusActive
		r.Sort, r.Descending = v.Sort, v.Descending
		r.Selected = make(map[string]rally.Object, len(v.SelectedItems))
		for ref, o := range v.SelectedItems {
			r.Selected[ref] = o.Clone()
		}
		r.History = cloneObjects(v.DetailHistory)
		r.LaneScroll = make(map[string]int, len(v.LaneScroll))
		for k, n := range v.LaneScroll {
			r.LaneScroll[k] = n
		}
		if d := v.Detail; d != nil {
			data := &detailTransfer{Original: d.Original.Clone(), Kind: d.Kind, Tab: d.Tab, New: d.New, Values: map[string]string{}, Rich: map[string]richtext.Saved{}}
			data.InlineField, data.InlineScope = d.inlineField, d.inlineScope
			data.Selection = d.selection.clone()
			data.PendingDefaultOwner = d.pendingDefaultOwner()
			data.Conflict, data.FieldConflicts = d.Conflict, slices.Clone(d.fieldConflicts)
			data.ReferenceLabels = maps.Clone(d.referenceLabels)
			data.Fields = cloneTransferFields(d.Fields)
			data.States = cloneObjects(d.States)
			for k, e := range d.Editors {
				data.Values[k] = text(e)
			}
			for k, r := range d.Rich {
				r.sync()
				data.Rich[k] = r.doc.Save()
			}
			if d.CommentRich != nil {
				d.CommentRich.sync()
				data.CommentDocument = d.CommentRich.doc.Save()
			}
			data.capturePresentation(d)
			r.Detail = data
		}
		p.Rally = r
	}
	return p
}
func (a *App) installTransfer(p tabTransfer) error {
	if err := validateTransfer(p); err != nil {
		return err
	}
	if p.Chat != nil && a.transferPending && a.incomingTab == "" {
		for _, existing := range a.state.Tabs {
			if existing.Kind == workspace.Chat && existing.Target == p.Chat.ID {
				return fmt.Errorf("conversation is already open in this window; close that tab before moving it here")
			}
		}
		if c := a.state.Chats[p.Chat.ID]; c != nil && (c.Draft != "" || len(c.DraftAttachments) > 0 || len(c.Queue) > 0 || len(c.Outbox) > 0 || c.QueueDraft.Text != "" || len(c.QueueDraft.Attachments) > 0) {
			return fmt.Errorf("this window has an unsent draft for the conversation; keep or discard it before moving the tab here")
		}
	}
	t := p.Tab
	id := t.ID
	if a.transferPending && a.incomingTicket != "" {
		id = "window-" + a.incomingTicket
		a.incomingTab = id
	}
	if id == "" {
		id = a.state.Open(t.Kind, t.Title, t.Target, t.Page)
	} else {
		found := false
		for _, existing := range a.state.Tabs {
			if existing.ID == id {
				found = true
				break
			}
		}
		if !found {
			t.ID = id
			a.state.Tabs = append(a.state.Tabs, t)
		}
		a.state.Active = id
	}
	if c := p.Chat; c != nil {
		if a.mailbox == nil {
			a.mailbox = map[string][]mailMessage{}
		}
		a.mailbox[c.ID] = p.Mailbox
		delete(a.detached, c.ID)
		a.invalidateSidebar(c.ID)
		a.state.Chats[c.ID] = c
		v := newChatView()
		setText(v.Editor, c.Draft)
		v.Attachments = c.DraftAttachments
		if view := p.ChatView; view != nil {
			view.Position.apply(v.Editor)
			v.Follow = view.Follow
			v.Scroll = view.Scroll
			v.RestoreScroll = true
			v.ShowBlocks = max(200, view.ShowBlocks)
			v.Expanded = maps.Clone(view.Expanded)
			if v.Expanded == nil {
				v.Expanded = map[string]bool{}
			}
			v.QueuePage = max(0, view.QueuePage)
		}
		a.chats[c.ID] = v

	}
	if r := p.Rally; r != nil {
		if old := a.rallyViews[id]; old != nil {
			old.Closed = true
			if m := old.SavedViewManager; m != nil && m.cancel != nil {
				m.cancel()
			}
			old.FilterDraft.closePicker()
			if old.cancel != nil {
				old.cancel()
			}
		}
		v := newRallyView(rally.FindPage(t.Page))
		v.Display = settings.DisplayOrDefault(a.prefs.RallyDisplay)
		if r.Display != nil {
			v.Display = settings.DisplayOrDefault(r.Display)
		}
		v.DisplayDraft = r.DisplayDraft.restore()
		v.boardStride, v.boardHeader = r.BoardStride, r.BoardHeader
		v.Mode, v.Group, v.Timebox = r.Mode, r.Group, r.Timebox
		v.CurrentIteration = r.CurrentIteration
		v.TimeboxName, v.ReleaseTimebox, v.ReleaseName = r.TimeboxName, r.ReleaseTimebox, r.ReleaseName
		v.migrateTimeboxes()
		if r.CardFields != nil {
			v.CardFields = slices.Clone(r.CardFields)
		}
		v.CollapsedLanes = maps.Clone(r.CollapsedLanes)
		v.ViewName, v.Widgets = r.ViewName, r.Widgets
		v.focusCard, v.focusCardIndex, v.cardFocusActive = r.FocusCard, r.FocusCardIndex, r.CardFocusActive
		v.Filters, v.ExitAgreements, v.Rules, v.ShowFields, v.AIView = r.Filters, r.ExitAgreements, r.Rules, r.ShowFields, r.AIView
		setText(v.Query, r.Query)
		setText(v.Search, r.Search)
		v.OwnerFilter, v.StateFilter = r.Owner, r.State
		v.OnlyBlocked, v.OnlyReady = r.OnlyBlocked, r.OnlyReady
		v.Columns = r.Columns
		v.QueryApplied = r.Query
		if r.QueryApplied != nil {
			v.QueryApplied = *r.QueryApplied
		}
		v.StructuredFilters = slices.Clone(r.StructuredFilters)
		v.FilterDraft = r.FilterDraft.restore()
		v.Sort, v.Descending = fallback(r.Sort, "Rank"), r.Descending
		v.DetailHistory = r.History
		for _, o := range r.Selected {
			v.selectItem(o, true)
		}
		v.RestoreLaneScroll = r.LaneScroll
		v.Page = max(1, r.ListPage)
		v.Start, v.RestoreCount, v.Total = max(1, r.ResidentStart), max(0, min(r.ResidentCount, rallyWindowItems)), max(0, r.Total)
		if d := r.Detail; d != nil {
			v.Detail = makeDetail(d.Original, d.Kind, d.New)
			v.Detail.inlineField, v.Detail.inlineScope = d.InlineField, d.InlineScope
			v.Detail.selection = d.Selection.clone()
			v.Detail.referenceLabels = maps.Clone(d.ReferenceLabels)
			v.Detail.Conflict, v.Detail.fieldConflicts = d.Conflict, slices.Clone(d.FieldConflicts)
			mergeSchemaEditors(v.Detail, d.Fields)
			v.Detail.setStates(d.States)
			v.Detail.Tab = d.Tab
			for k, value := range d.Values {
				if v.Detail.Editors[k] == nil {
					v.Detail.Editors[k] = textEditor(value, false)
				} else {
					setText(v.Detail.Editors[k], value)
				}
				if v.Detail.Rich[k] != nil {
					v.Detail.Rich[k] = newRichEditor(value)
				}
			}
			for k, saved := range d.Rich {
				r := newRichEditor("")
				r.doc = richtext.Restore(saved)
				if len(r.doc.Unsupported) > 0 {
					r.mode = "HTML"
				}
				v.Detail.Rich[k] = r
			}
			r := newRichEditor("")
			r.doc = richtext.Restore(d.CommentDocument)
			if len(r.doc.Unsupported) > 0 {
				r.mode = "HTML"
			}
			v.Detail.CommentRich = r
			d.restorePresentation(v.Detail)
			if d.InlineField != "" && v.Detail.Editors[d.InlineField] != nil {
				v.Detail.Editors[d.InlineField].Flags |= nucular.EditSigEnter
			}
			if d.PendingDefaultOwner && v.Detail.New && text(v.Detail.Editors["Owner"]) == "" {
				v.Detail.ownerDefaultEditor = v.Detail.Editors["Owner"]
				v.Detail.ownerDefaultRevision = v.Detail.ownerDefaultEditor.TextRevision()
				applyDefaultOwner(v.Detail, a.rallyUser)
			}
		}
		a.rallyViews[id] = v
		v.signature = a.rallySignature(v)
		if a.rallyClient != nil {
			a.refreshRallyItems(v)
			if v.Detail != nil && len(v.Detail.Fields) == 0 {
				a.rehydrateDetail(v)
			}
		}
	}
	if t.Kind == workspace.File {
		if f := p.File; f != nil {
			if old := a.files[id]; old != nil {
				old.dispose()
			}
			v := &fileView{Path: t.Target, Find: textEditor(f.Find, false), Virtual: f.Virtual, Wrap: f.Wrap, More: f.More, Offset: f.Offset, BasePath: f.BasePath, Stamp: f.Stamp, FileNotice: f.FileNotice, LimitReached: f.LimitReached, Lossy: f.Lossy}
			a.files[id] = v
			if f.Diff {
				v.DiffCollapsed = slices.Clone(f.DiffCollapsed)
				v.DiffSelection = f.DiffSelection
				v.DiffSelection.Dragging = false
				v.DiffScroll.X, v.DiffScroll.Y = max(0, f.ScrollX), max(0, f.ScrollY)
				v.DiffRestoreScroll = true
				a.prepareDiff(v, f.Text)
			} else {
				v.Editor = loadedFileEditor(f.Text)
				v.Loaded, v.LoadedText = f.TextLoaded, f.Text
				if !v.Wrap {
					v.Editor.Flags &^= nucular.EditNoHorizontalScroll | nucular.EditSoftWrap
				}
				f.Position.apply(v.Editor)
				v.Editor.Scrollbar.X, v.Editor.Scrollbar.Y = max(0, f.ScrollX), max(0, f.ScrollY)
				if !v.Virtual && v.Loaded {
					a.watchFile(v)
				}
			}
			if !f.Virtual && !f.TextLoaded {
				v.Editor = nil
				v.PendingPosition = &f.Position
				a.loadFilePage(v, false)
			}
		} else if p.Text != "" {
			a.openText(t.Title, p.Text)
		} else {
			a.openFile(t.Target)
		}
	}
	if t.Kind == workspace.Settings {
		a.openSettings()
	}
	return nil
}
func (a *App) popOut(t workspace.Tab) {
	if a.pendingRallyWrite(t.ID) {
		a.toast = "Wait for the Rally update to finish before moving this tab"
		return
	}
	if a.windowReturn != nil {
		return
	}
	if a.connection.Address == "" || a.client == nil {
		a.toast = "Codex is still connecting"
		return
	}
	if a.popping[t.ID] {
		return
	}
	a.popping[t.ID] = true
	payload := a.tabSnapshot(t)
	connection := a.connection
	a.rpcResult("fastrock/offer", map[string]any{"data": payload}, func(raw json.RawMessage) {
		var offer struct{ Ticket string }
		if json.Unmarshal(raw, &offer) != nil || offer.Ticket == "" {
			a.abortTransfer(t.ID, "", fmt.Errorf("open window: invalid transfer ticket"))
			return
		}
		a.transfers[offer.Ticket] = t.ID
		if a.transferChildren == nil {
			a.transferChildren = map[string]*transferChild{}
		}
		child := &transferChild{}
		a.transferChildren[offer.Ticket] = child
		a.watchTransfer(t.ID, offer.Ticket)
		a.controlWork(func() {
			binary, err := os.Executable()
			if err == nil {
				cmd := exec.Command(binary, "--popout", offer.Ticket)
				cmd.Env = append(windowEnvironment(), "FASTROCK_BROKER="+connection.Address, "FASTROCK_BROKER_TOKEN="+connection.Token)
				err = cmd.Start()
				if err == nil {
					child.attach(cmd.Process)
					go func() {
						_ = cmd.Wait()
						child.exited()
						a.post(func() {
							a.abortTransfer(t.ID, offer.Ticket, fmt.Errorf("destination window exited before accepting the tab; the source is still here"))
						})
					}()
				}
			}
			if err != nil {
				a.post(func() { a.abortTransfer(t.ID, offer.Ticket, fmt.Errorf("open window: %w", err)) })
			}
		}, func() { a.abortTransfer(t.ID, offer.Ticket, errWorkQueueFull) })
	}, func(err error) { a.abortTransfer(t.ID, "", err) })
}

func (a *App) transferEvent(m codex.Message) bool {
	if !strings.HasPrefix(m.Method, "fastrock/") {
		return false
	}
	switch m.Method {
	case "fastrock/deliveryPending", "fastrock/deliveryRequest", "fastrock/deliveryResult", "fastrock/deliveryResolved":
		a.deliveryEvent(m)
	case "fastrock/messagingDisabled":
		var disabled struct {
			ThreadIDs []string `json:"threadIds"`
		}
		if json.Unmarshal(m.Params, &disabled) == nil {
			for _, id := range disabled.ThreadIDs {
				a.declineConsents(id, "The target conversation turned off messaging")
			}
		}
	case "fastrock/delivery":
		var message mailMessage
		if json.Unmarshal(m.Params, &message) == nil && message.To != "" {
			a.recordMail(message.To, message)
		}
	case "fastrock/serverStopped":
		a.cancelRecaps()
		a.resetSettingsConnection()
		a.clearReplyWaits()
		a.resetInfoConnection()
		var p struct {
			Message    string
			Restarting bool
		}
		_ = json.Unmarshal(m.Params, &p)
		a.status, a.toast = p.Message, p.Message
		a.serverPaused = true
		a.serverGeneration++
		a.catalogGeneration++
		a.catalog.PolicyLoaded = false
		a.serverStarting = p.Restarting || p.Message == "Restarting Codex app-server…"
		a.serverError = ""
		if !a.serverStarting {
			a.serverError = p.Message
		}
		for id, v := range a.chats {
			if c := a.state.Chats[id]; c != nil {
				c.Draft = text(v.Editor)
				c.DraftAttachments = append([]string(nil), v.Attachments...)
				if c.Ephemeral {
					c.EphemeralLost = true
					continue
				}
			}
			delete(a.chats, id)
		}
		a.approvals = nil
		a.approvalDiffs = nil
		for _, c := range a.state.Chats {
			c.Status, c.TurnID = "idle", ""
			if c.Ephemeral {
				c.EphemeralLost = true
				c.Status = "ended"
			}
		}
		if a.assistant != nil {
			a.assistant.Busy = false
			a.assistant.ThreadID = ""
			a.assistant.TurnID = ""
		}
	case "fastrock/serverReady":
		a.resetInfoConnection()
		var p struct{ Version string }
		_ = json.Unmarshal(m.Params, &p)
		a.status = p.Version + " · Connected"
		a.serverPaused = false
		a.serverGeneration++
		a.serverStarting, a.restartPending = false, false
		a.serverError, a.restartNote, a.startedProvider = "", "", ""
		a.toast = "Codex restarted; unsent drafts retained"
		a.refreshCatalog()
		a.requestThreads(false, "")
		if t := a.state.Current(); t != nil && t.Kind == workspace.Chat {
			a.resumeThread(t.Target)
		}
	case "fastrock/tabRefresh":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		if id := a.transfers[p.Ticket]; id != "" {
			for _, t := range a.state.Tabs {
				if t.ID == id {
					payload := a.tabSnapshot(t)
					a.rpc("fastrock/finalize", map[string]any{"ticket": p.Ticket, "data": payload}, nil)
					break
				}
			}
		}
	case "fastrock/tabCancelled":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		if id := a.transfers[p.Ticket]; id != "" {
			returning := a.windowReturn != nil
			a.finishTransfer(p.Ticket, false)
			if !returning {
				a.toast = "Window transfer was cancelled; the tab is still here"
			}
		}
		if a.incomingTicket == p.Ticket {
			a.cancelIncoming()
		}
	case "fastrock/finalized":
		var p struct {
			Ticket string
			Data   tabTransfer
		}
		if json.Unmarshal(m.Params, &p) == nil && p.Ticket == a.incomingTicket {
			if err := a.installTransfer(p.Data); err != nil {
				a.rpc("fastrock/cancel", map[string]string{"ticket": p.Ticket}, nil)
				a.cancelIncoming()
				a.report(err)
				return true
			}
			a.rpcResult("fastrock/applied", map[string]string{"ticket": p.Ticket}, func(_ json.RawMessage) {
				a.finishIncoming()
			}, func(err error) { a.resolveIncoming(p.Ticket, err) })
		}

	case "fastrock/preferences":
		var state codex.PreferencesState
		if json.Unmarshal(m.Params, &state) == nil {
			a.applyPreferences(state, nil)
		}
	case "fastrock/threadMoved":
		var p struct{ ThreadID string }
		_ = json.Unmarshal(m.Params, &p)
		if a.detached == nil {
			a.detached = map[string]bool{}
		}
		a.detached[p.ThreadID] = true
		for _, t := range append([]workspace.Tab(nil), a.state.Tabs...) {
			if t.Kind == workspace.Chat && t.Target == p.ThreadID {
				a.closeTransferredTab(t.ID)
			}
		}
		a.releaseTransferredConversation(p.ThreadID)
		for i := len(a.approvals) - 1; i >= 0; i-- {
			params := codex.Decode(a.approvals[i].Message.Params)
			if str(params, "threadId") == p.ThreadID {
				a.approvals = append(a.approvals[:i], a.approvals[i+1:]...)
			}
		}
	case "fastrock/tabAvailable":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		a.claimTab(p.Ticket)
	case "fastrock/tabClaimed":
		var p struct{ Ticket string }
		_ = json.Unmarshal(m.Params, &p)
		if id := a.transfers[p.Ticket]; id != "" {
			a.finishTransfer(p.Ticket, true)
		}
	}
	return true
}
func (a *App) claimTab(ticket string) {
	if a.windowReturn != nil || a.transferPending || a.readyTicket != "" {
		a.toast = "Finish the current window transfer before moving another tab"
		return
	}
	client := a.client
	if client == nil {
		a.report(fmt.Errorf("window service is disconnected; could not receive tab"))
		return
	}
	a.transferPending = true
	a.incomingTicket = ticket
	a.work(func() {
		var payload tabTransfer
		ctx, cancel := context.WithTimeout(a.ctx, 15*time.Second)
		defer cancel()
		err := client.Call(ctx, "fastrock/claim", map[string]string{"ticket": ticket}, &payload)
		a.post(func() {
			if err != nil {
				a.transferPending = false
				a.incomingTicket = ""
				a.report(err)
				a.closeAbandonedPopout()
			} else {
				if err := a.installTransfer(payload); err != nil {
					a.rpc("fastrock/cancel", map[string]string{"ticket": ticket}, nil)
					a.cancelIncoming()
					a.report(err)
					return
				}
				a.transferPending = true
				if payload.Chat != nil {
					a.transferBuffer = map[string][]codex.Message{payload.Chat.ID: nil}
				}
				a.readyTicket = ticket
			}
		})
	}, func() { a.transferPending = false; a.incomingTicket = "" })
}
func (a *App) moveTab(t workspace.Tab) {
	if a.pendingRallyWrite(t.ID) {
		a.toast = "Wait for the Rally update to finish before moving this tab"
		return
	}
	if a.windowReturn != nil {
		return
	}
	a.rpc("fastrock/windows", map[string]any{}, func(raw json.RawMessage) {
		var list struct{ Windows []struct{ ID, Name string } }
		_ = json.Unmarshal(raw, &list)
		a.window.PopupOpen("Move tab to window", nucular.WindowTitle|nucular.WindowClosable, a.modalBounds(600, 500), false, func(w *nucular.Window) {
			if len(list.Windows) == 0 {
				muted(w, "No other Fastrock windows are open.", a.p)
			}
			for _, other := range list.Windows {
				w.Row(30).Dynamic(1)
				if w.ButtonText(fallback(other.Name, other.ID[:8])) {
					if a.pendingRallyWrite(t.ID) {
						a.toast = "Wait for the Rally update to finish before moving this tab"
						return
					}
					a.popping[t.ID] = true
					payload := a.tabSnapshot(t)
					destination := other.ID
					a.rpcResult("fastrock/offer", map[string]any{"data": payload}, func(raw json.RawMessage) {
						var offer struct{ Ticket string }
						_ = json.Unmarshal(raw, &offer)
						a.transfers[offer.Ticket] = t.ID
						a.rpcResult("fastrock/move", map[string]string{"ticket": offer.Ticket, "window": destination}, func(json.RawMessage) {
							a.watchTransfer(t.ID, offer.Ticket)
						}, func(err error) { a.abortTransfer(t.ID, offer.Ticket, err) })
					}, func(err error) { a.abortTransfer(t.ID, "", err) })
					w.Close()
				}
			}
		})
	})
}
func windowEnvironment() []string { return platform.ChildEnv() }

func (a *App) cancelIncoming() {
	if a.incomingTab != "" {
		a.closeTabNow(a.incomingTab)
	}
	a.transferPending = false
	a.incomingTicket, a.incomingTab, a.readyTicket = "", "", ""
	a.transferBuffer = nil
	a.toast = "Window transfer was cancelled; the source tab is intact"
	a.closeAbandonedPopout()
}
