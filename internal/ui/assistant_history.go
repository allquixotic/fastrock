package ui

import (
	"fmt"
	"github.com/aarzilli/nucular"
	"github.com/allquixotic/fastrock/internal/assistant"
	"sort"
	"strings"
)

//lint:ignore U1000 Used by the opt-in fastrock_automation build.
func (s *assistantView) transcriptText() string {
	var b strings.Builder
	for _, block := range s.History.Blocks {
		b.WriteString(block.Text)
		b.WriteByte('\n')
	}
	return b.String()
}
func (a *App) drawAssistantHistory(w *nucular.Window, s *assistantView) {
	if s.Layout == nil {
		s.Layout = newChatView()
	}
	v := s.Layout
	v.pruneLayouts(s.History.Blocks)
	offset := 0
	for _, block := range s.History.Blocks {
		layout := v.Layouts[block.ID]
		height := 48
		if layout != nil {
			height = layout.Height + 32
		}
		if offset+height < w.Scrollbar.Y-100 || offset > w.Scrollbar.Y+w.Bounds.H+100 {
			w.Row(height).Dynamic(1)
			w.Spacing(1)
			offset += height
			continue
		}
		layout = a.transcriptLayout(v, block, w.LayoutAvailableWidth())
		muted(w, block.Role, a.p)
		first, last, leading, trailing := visibleTranscriptLines(layout, w.WidgetBounds().Y, w.Bounds.Y, w.Bounds.Y+w.Bounds.H, w.Master().Style().Scaling, w.Master().Style().GroupWindow.Spacing.Y)
		if leading > 0 {
			w.RowScaled(max(1, leading-w.Master().Style().GroupWindow.Spacing.Y)).Dynamic(1)
			w.Spacing(1)
		}
		for _, line := range layout.Lines[first:last] {
			w.Row(line.Height).Dynamic(1)
			a.drawTranscriptLine(w, v, block.ID, layout, line)
		}
		if trailing > 0 {
			w.RowScaled(max(1, trailing-w.Master().Style().GroupWindow.Spacing.Y)).Dynamic(1)
			w.Spacing(1)
		}
		offset += layout.Height + 32
	}
}
func proposalPreview(plan assistant.Plan) []string {
	rows := make([]string, 0, len(plan.Changes))
	for _, change := range plan.Changes {
		var b strings.Builder
		fmt.Fprintf(&b, "%s %s", change.Operation, fallback(change.Before.ID(), change.Kind))
		if title := change.Before.String("Name"); title != "" {
			fmt.Fprintf(&b, " · %s", title)
		}
		keys := make([]string, 0, len(change.Fields))
		for k := range change.Fields {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, key := range keys {
			fmt.Fprintf(&b, "\n%s: %s → %v", key, change.Before.String(key), change.Fields[key])
		}
		rows = append(rows, b.String())
	}
	return rows
}
