package ui

import (
	"maps"
	"reflect"
	"slices"

	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/workspace"
)

type editorVersion struct {
	editor   *nucular.TextEditor
	revision uint64
}

func versionOf(e *nucular.TextEditor) editorVersion {
	if e == nil {
		return editorVersion{}
	}
	return editorVersion{e, e.TextRevision()}
}

type richVersion struct {
	document *richtext.Document
	revision uint64
}

func richVersionOf(r *richEditor) richVersion {
	if r == nil {
		return richVersion{}
	}
	r.sync()
	return richVersion{r.doc, r.doc.Revision}
}

type documentCheckpoint struct {
	observed                                           uint64
	value                                              tabTransfer
	view                                               *rallyView
	detail                                             *detailView
	detailRevision, selectionRevision, historyRevision uint64
	editors                                            map[string]editorVersion
	rich                                               map[string]richVersion
	comment                                            richVersion
}

func (c *documentCheckpoint) matches(a *App, t workspace.Tab) bool {
	if c.value.Tab != t {
		return false
	}
	if f := a.files[t.ID]; f != nil && (f.Editor != nil || f.Loaded) {
		x := c.value.File
		pos := f.filePosition()
		return x != nil && x.Diff == (len(f.Diff) > 0) && x.BasePath == f.BasePath && x.Find == text(f.Find) && x.Virtual == f.Virtual && x.Wrap == f.Wrap && x.More == f.More && x.Offset == f.Offset && x.Position == pos && x.ScrollX == pos.ScrollX && x.ScrollY == pos.ScrollY && x.FileNotice == f.FileNotice && x.LimitReached == f.LimitReached && x.Lossy == f.Lossy && x.Stamp.Valid == f.Stamp.Valid && x.Stamp.Size == f.Stamp.Size && x.Stamp.Modified.Equal(f.Stamp.Modified)
	}
	if c.value.File != nil {
		return false
	}
	v := a.rallyViews[t.ID]
	if v != c.view {
		return false
	}
	if v == nil {
		return true
	}
	x := c.value.Rally
	if x != nil && (x.Display == nil || *x.Display != v.Display || !v.DisplayDraft.matches(x.DisplayDraft) || x.BoardStride != v.boardStride || x.BoardHeader != v.boardHeader) {
		return false
	}
	if x != nil && (x.QueryApplied == nil || *x.QueryApplied != v.QueryApplied || !slices.Equal(x.StructuredFilters, v.StructuredFilters) || !v.FilterDraft.matches(x.FilterDraft)) {
		return false
	}
	if x != nil && (x.FocusCard != v.focusCard || x.FocusCardIndex != v.focusCardIndex || x.CardFocusActive != v.cardFocusActive) {
		return false
	}
	if x == nil || x.Mode != v.Mode || x.Group != v.Group || x.Timebox != v.Timebox || x.TimeboxName != v.TimeboxName || x.ReleaseTimebox != v.ReleaseTimebox || x.ReleaseName != v.ReleaseName || x.CurrentIteration != v.CurrentIteration || x.Query != text(v.Query) || x.Search != text(v.Search) || x.Owner != v.OwnerFilter || x.State != v.StateFilter || x.OnlyBlocked != v.OnlyBlocked || x.OnlyReady != v.OnlyReady || x.ListPage != v.Page || x.ViewName != v.ViewName || x.Widgets != v.Widgets || x.Sort != v.Sort || x.Descending != v.Descending || x.ResidentStart != v.Start || x.ResidentCount != max(len(v.Items), v.RestoreCount) || x.Total != v.Total || x.Filters != v.Filters || x.ExitAgreements != v.ExitAgreements || x.Rules != v.Rules || x.ShowFields != v.ShowFields || x.AIView != v.AIView || !slices.Equal(x.Columns, v.Columns) || !slices.Equal(x.CardFields, v.CardFields) || !maps.Equal(x.CollapsedLanes, v.CollapsedLanes) || !maps.Equal(x.LaneScroll, v.LaneScroll) || c.selectionRevision != v.selectionRevision || c.historyRevision != v.historyRevision {
		return false
	}
	d := v.Detail
	if d != c.detail {
		return false
	}
	if d == nil {
		return true
	}
	if d.snapshotRevision != c.detailRevision || x.Detail == nil || d.Tab != x.Detail.Tab || d.Kind != x.Detail.Kind || d.New != x.Detail.New || len(d.Editors) != len(c.editors) || len(d.Rich) != len(c.rich) {
		return false
	}
	for k, e := range d.Editors {
		if c.editors[k] != versionOf(e) {
			return false
		}
	}
	for k, r := range d.Rich {
		if c.rich[k] != richVersionOf(r) {
			return false
		}
	}
	return c.comment == richVersionOf(d.CommentRich)
}

func (a *App) checkpointDocument(t workspace.Tab) tabTransfer {
	if a.documentCheckpoints == nil {
		a.documentCheckpoints = map[string]*documentCheckpoint{}
	}
	if old := a.documentCheckpoints[t.ID]; old != nil && old.matches(a, t) {
		old.refreshPresentation()
		old.observed = a.checkpointEpoch
		return old.value
	}
	x := a.tabSnapshotWithContent(t, false)
	x.Chat = nil
	c := &documentCheckpoint{value: x, view: a.rallyViews[t.ID], observed: a.checkpointEpoch}
	if v := c.view; v != nil {
		c.selectionRevision, c.historyRevision = v.selectionRevision, v.historyRevision
		c.detail = v.Detail
		if d := v.Detail; d != nil {
			c.detailRevision = d.snapshotRevision
			c.editors = map[string]editorVersion{}
			for k, e := range d.Editors {
				c.editors[k] = versionOf(e)
			}
			c.rich = map[string]richVersion{}
			for k, r := range d.Rich {
				c.rich[k] = richVersionOf(r)
			}
			c.comment = richVersionOf(d.CommentRich)
		}
	}
	a.documentCheckpoints[t.ID] = c
	return x
}

// Compare only persisted fields. Private streaming buffers and transcript
// blocks are deliberately excluded, and unchanged strings remain shared.
var persistedChatFields = func() []int {
	t := reflect.TypeOf(workspace.Conversation{})
	var fields []int
	for i := 0; i < t.NumField(); i++ {
		field := t.Field(i)
		if field.PkgPath != "" || field.Name == "Blocks" {
			continue
		}
		fields = append(fields, i)
	}
	return fields
}()

func sameConversationCheckpoint(a, b *workspace.Conversation) bool {
	av, bv := reflect.ValueOf(a).Elem(), reflect.ValueOf(b).Elem()
	for _, i := range persistedChatFields {
		x, y := av.Field(i), bv.Field(i)
		switch x.Kind() {
		case reflect.String:
			if x.String() != y.String() {
				return false
			}
		case reflect.Bool:
			if x.Bool() != y.Bool() {
				return false
			}
		case reflect.Int, reflect.Int64:
			if x.Int() != y.Int() {
				return false
			}
		case reflect.Uint64:
			if x.Uint() != y.Uint() {
				return false
			}
		default:
			if !reflect.DeepEqual(x.Interface(), y.Interface()) {
				return false
			}
		}
	}
	return true
}

func (a *App) checkpointConversation(c *workspace.Conversation) *workspace.Conversation {
	local := *c
	local.Blocks, local.TurnID, local.Status = nil, "", "idle"
	if v := a.chats[c.ID]; v != nil {
		local.Draft, local.DraftAttachments = text(v.Editor), v.Attachments
	}
	if a.chatCheckpoints == nil {
		a.chatCheckpoints = map[string]*workspace.Conversation{}
	}
	if old := a.chatCheckpoints[c.ID]; old != nil && sameConversationCheckpoint(&local, old) {
		return old
	}
	local.Queue = cloneQueue(local.Queue)
	local.Outbox = cloneQueue(local.Outbox)
	local.QueueDraft.Attachments = slices.Clone(local.QueueDraft.Attachments)
	local.DraftAttachments = slices.Clone(local.DraftAttachments)
	local.Agents = slices.Clone(local.Agents)
	a.chatCheckpoints[c.ID] = &local
	return &local
}

func sameSessionCheckpoint(a, b *session) bool {
	if a == nil || b == nil || a.Active != b.Active || a.Counter != b.Counter || !slices.Equal(a.Tabs, b.Tabs) || !maps.Equal(a.Chats, b.Chats) || !reflect.DeepEqual(a.Mailbox, b.Mailbox) || len(a.Documents) != len(b.Documents) {
		return false
	}
	for i, x := range a.Documents {
		y := b.Documents[i]
		if x.Tab != y.Tab || x.File != y.File || x.Rally != y.Rally {
			return false
		}
	}
	return true
}

func (a *App) stopCheckpointTimers() {
	if a.checkpointTimer != nil {
		a.checkpointTimer.Stop()
	}
	if a.checkpointObserveTimer != nil {
		a.checkpointObserveTimer.Stop()
	}
}
