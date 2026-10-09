package ui

import (
	"slices"

	"github.com/allquixotic/fastrock/internal/rally"
)

func cloneTransferFields(in []rally.Field) []rally.Field {
	out := slices.Clone(in)
	for i := range out {
		out[i].AllowedValues = slices.Clone(out[i].AllowedValues)
	}
	return out
}

func cloneObjects(in []rally.Object) []rally.Object {
	out := make([]rally.Object, len(in))
	for i, o := range in {
		out[i] = o.Clone()
	}
	return out
}

type richPresentation struct {
	Mode             string
	Position, Source editorPosition
}

func (r *richEditor) presentationState() richPresentation {
	if r == nil {
		return richPresentation{}
	}
	s := r.presentation
	s.Mode = r.mode
	if r.editor != nil {
		s.Position = position(r.editor)
	}
	if r.source != nil {
		s.Source = position(r.source)
	}
	return s
}

func (r *richEditor) restorePresentation(s richPresentation) {
	if r == nil {
		return
	}
	switch s.Mode {
	case "Edit", "Preview", "HTML":
		r.mode = s.Mode
	}
	r.presentation = s
	s.Position.apply(r.editor)
	s.Source.apply(r.source)
}

func (s *detailTransfer) capturePresentation(d *detailView) {
	s.Positions = make(map[string]editorPosition, len(d.Editors))
	for k, e := range d.Editors {
		s.Positions[k] = position(e)
	}
	s.RichViews = make(map[string]richPresentation, len(d.Rich))
	for k, r := range d.Rich {
		s.RichViews[k] = r.presentationState()
	}
	s.CommentView = d.CommentRich.presentationState()
}

func (s *detailTransfer) samePresentation(d *detailView) bool {
	if len(s.Positions) != len(d.Editors) || len(s.RichViews) != len(d.Rich) {
		return false
	}
	for k, e := range d.Editors {
		if s.Positions[k] != position(e) {
			return false
		}
	}
	for k, r := range d.Rich {
		if s.RichViews[k] != r.presentationState() {
			return false
		}
	}
	return s.CommentView == d.CommentRich.presentationState()
}

func (s *detailTransfer) restorePresentation(d *detailView) {
	for k, p := range s.Positions {
		p.apply(d.Editors[k])
	}
	for k, p := range s.RichViews {
		d.Rich[k].restorePresentation(p)
	}
	d.CommentRich.restorePresentation(s.CommentView)
}

// Caret/scroll changes need a new immutable envelope, not another copy of the
// saved rich text. Content revisions are checked before entering this path.
func (c *documentCheckpoint) refreshPresentation() {
	if c.detail == nil || c.value.Rally == nil || c.value.Rally.Detail.samePresentation(c.detail) {
		return
	}
	r := *c.value.Rally
	d := *r.Detail
	d.capturePresentation(c.detail)
	r.Detail = &d
	c.value.Rally = &r
}
