package ui

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/workspace"
)

func TestSavedBaselinePreservesLaterEditsAndComment(t *testing.T) {
	original := rally.Object{"Name": "old", "Description": "<p>old</p>", "_ref": "/item/1"}
	d := makeDetail(original, "HierarchicalRequirement", false)
	setText(d.Editors["Name"], "submitted")
	submitted := d.values()
	setText(d.Editors["Name"], "later")
	d.Rich["Description"].ensureEditor()
	setText(d.Rich["Description"].editor, "new description")
	d.CommentRich.ensureEditor()
	setText(d.CommentRich.editor, "unsent comment")
	saved := original.Clone()
	saved["Name"] = "submitted"
	d.acceptSave(saved, submitted)
	if text(d.Editors["Name"]) != "later" || d.Rich["Description"].html() != "<p>new description</p>" || !d.dirty() {
		t.Fatal("save discarded later edits")
	}
	if d.CommentRich.html() != "<p>unsent comment</p>" {
		t.Fatal("comment discarded")
	}
}
func TestSelectionRetainsReviewedObjectsOutsideResidentWindow(t *testing.T) {
	v := newRallyView(rally.FindPage("teamboard"))
	o := rally.Object{"_ref": "/item/1", "FormattedID": "US1", "LastUpdateDate": "reviewed"}
	v.selectItem(o, true)
	v.Items = nil
	p := (&App{}).selectionPlan(v, "update", rally.Object{"Blocked": true})
	if len(p.Changes) != 1 || p.Changes[0].Before.String("LastUpdateDate") != "reviewed" {
		t.Fatal("lost reviewed selection")
	}
	v.selectItem(o, false)
	if len(v.Selected) != 0 || len(v.SelectedItems) != 0 {
		t.Fatal("deselection retained object")
	}
}

func TestV59SparseSavePreservesAcknowledgedValuesAndLaterEdits(t *testing.T) {
	d := makeDetail(rally.Object{"Name": "Original", "Description": "<p>Retain</p>", "LastUpdateDate": "reviewed", "Owner": map[string]any{"_ref": "/user/42", "_refObjectName": "Owner name"}}, "HierarchicalRequirement", false)
	setText(d.Editors["Name"], "Submitted")
	setText(d.Editors["PlanEstimate"], "3")
	submitted := d.values()
	changes, err := d.changes()
	if err != nil {
		t.Fatal(err)
	}
	setText(d.Editors["Name"], "Later edit")
	saved := d.savedSnapshot(d.Original, rally.Object{"ObjectID": 7, "Name": "Server normalized", "PlanEstimate": float64(4), "Priority": nil}, changes)
	d.acceptSave(saved, submitted)
	if text(d.Editors["Name"]) != "Later edit" || d.Original.String("Name") != "Server normalized" || text(d.Editors["PlanEstimate"]) != "4" {
		t.Fatal("save lost a later edit or ignored the server", d.values(), d.Original)
	}
	if d.Original.Ref("Owner") != "/user/42" || d.Original.String("Owner") != "Owner name" || d.Rich["Description"].html() != "<p>Retain</p>" || d.Original.String("LastUpdateDate") != "reviewed" {
		t.Fatal("omitted response fields or the revision guard were discarded", d.Original)
	}
	if value, exists := d.Original["Priority"]; !exists || value != nil {
		t.Fatal("explicit server null was ignored")
	}
	// Identity may arrive after the create payload is captured. That default
	// stays an unsaved local change until it has actually been sent to Rally.
	d = makeDetail(rally.Object{"Name": "New"}, "HierarchicalRequirement", true)
	d.ownerDefaultEditor = d.Editors["Owner"]
	d.ownerDefaultRevision = d.ownerDefaultEditor.TextRevision()
	before, submitted := d.Original.Clone(), d.values()
	applyDefaultOwner(d, rally.Object{"_ref": "/user/8", "DisplayName": "Late owner"})
	saved = d.savedSnapshot(before, rally.Object{"ObjectID": 8, "Name": "New"}, rally.Object{"Name": "New"})
	d.acceptSave(saved, submitted)
	if text(d.Editors["Owner"]) != "/user/8" || d.Original.Ref("Owner") != "" || !d.dirty() {
		t.Fatal("late default was mistaken for an acknowledged value", d.values(), d.Original)
	}
}
func TestQueueEditorRestoresParkedDraftAndAttachments(t *testing.T) {
	c := &workspace.Conversation{}
	c.Enqueue("queued", []string{"queued.png"})
	v := newChatView()
	setText(v.Editor, "draft")
	v.Attachments = []string{"draft.png"}
	beginQueueEdit(c, v, c.Queue[0])
	if text(v.Editor) != "queued" || len(v.Attachments) != 1 || v.Attachments[0] != "queued.png" {
		t.Fatal("queued attachments not loaded")
	}
	finishQueueEdit(c, v)
	if text(v.Editor) != "draft" || v.Attachments[0] != "draft.png" {
		t.Fatal("parked draft not restored")
	}
}
func TestSessionWriterRetriesUnchangedFailedSnapshot(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	input := make(chan session, 1)
	success := make(chan struct{}, 1)
	attempts := 0
	go persistSessions(ctx, input, func(s session) error {
		attempts++
		if attempts == 1 {
			return errors.New("disk full")
		}
		if s.Active != "draft" {
			t.Error("wrong snapshot")
		}
		success <- struct{}{}
		return nil
	}, func(error) {})
	input <- session{Active: "draft"}
	select {
	case <-success:
	case <-time.After(3 * time.Second):
		t.Fatal("failed snapshot was never retried")
	}
}
func TestWorkflowRefAndUnavailableState(t *testing.T) {
	v := newRallyView(rally.FindPage("portfoliokanban"))
	v.Workflow = []rally.Object{{"Name": "Delivered", "_ref": "/state/12"}}
	if value, ok := v.stateValue("Delivered"); !ok || value != "/state/12" {
		t.Fatal("portfolio state lost ref")
	}
	if _, ok := v.stateValue("Completed"); ok {
		t.Fatal("invented Completed transition")
	}
}

func TestUnsupportedAcceptanceCriteriaIsNotEditable(t *testing.T) {
	for _, kind := range []string{"Task", "Defect"} {
		d := makeDetail(rally.Object{"Name": "x"}, kind, false)
		if d.Editors["AcceptanceCriteria"] != nil || d.Rich["AcceptanceCriteria"] != nil {
			t.Fatalf("invalid field for %s", kind)
		}
	}
}
