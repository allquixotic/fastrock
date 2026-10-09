package ui

import (
	"fmt"
	"strings"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/rect"
	"golang.org/x/mobile/event/mouse"
)

func approvalScale(w *nucular.Window) float64 { return max(1, w.Master().Style().Scaling) }
func approvalWrap(w *nucular.Window, value string, width int, face font.Face) []string {
	return nucular.WrapText(face, value, max(40, width))
}
func approvalTextHeight(w *nucular.Window, value string, width int, face font.Face) int {
	return max(int(24*approvalScale(w)), len(approvalWrap(w, value, width, face))*(nucular.FontHeight(face)+2)+2*w.Master().Style().Text.Padding.Y+2)
}
func approvalChoiceHeight(w *nucular.Window, c approvalChoice) int {
	return max(int(30*approvalScale(w)), approvalTextHeight(w, c.Title, int(float64(w.Bounds.W-80)*.88), w.Master().Style().Font)+12)
}

const approvalKeyHint = "↑/↓ choose · Enter confirms · Esc cancels"
const approvalUnansweredHint = "Unanswered questions are sent without an answer. Skip sends no answers."
const approvalNoChoicesHint = "Codex did not offer any approval choices for this request."

func approvalHasDecisions(r *approval) bool {
	return r.Message.Method == "item/commandExecution/requestApproval" || r.Message.Method == "item/fileChange/requestApproval"
}
func approvalHintHeight(w *nucular.Window, hint string) int {
	return approvalTextHeight(w, hint, w.Bounds.W-60, w.Master().Style().Font)
}
func (a *App) drawApprovalHint(w *nucular.Window, hint string) {
	w.RowScaled(approvalHintHeight(w, hint)).Dynamic(1)
	previous := w.Master().Style().Text.Color
	w.Master().Style().Text.Color = a.p.Muted
	w.LabelWrap(hint)
	w.Master().Style().Text.Color = previous
}
func approvalFooterHeight(w *nucular.Window, r *approval) int {
	scale := approvalScale(w)
	spacing := w.Master().Style().GroupWindow.Spacing.Y
	h := 0
	if r.URL != "" {
		h += int(28*scale) + spacing
	}
	if len(r.Choices) > 0 {
		for _, c := range r.Choices {
			h += approvalChoiceHeight(w, c) + spacing
		}
		return h + approvalHintHeight(w, approvalKeyHint) + spacing
	}
	if approvalHasDecisions(r) {
		return h + approvalHintHeight(w, approvalNoChoicesHint) + spacing
	}
	if r.FormError != "" {
		h += int(40*scale) + spacing
	}
	if r.Message.Method == "item/tool/requestUserInput" {
		h += approvalHintHeight(w, approvalUnansweredHint) + spacing
	}
	return h + int(28*scale) + spacing
}
func approvalBodyHeight(w *nucular.Window, r *approval) int {
	scale := approvalScale(w)
	width := w.Bounds.W - 60
	face := w.Master().Style().Font
	titleFace := typeFace(fontPointSize(face)+3, boldFont)
	h := approvalTextHeight(w, r.Title, width, titleFace) + int(44*scale)
	if r.OriginThreadID != r.ThreadID {
		h += int(24 * scale)
	}
	if r.Content.Reason != "" {
		h += approvalTextHeight(w, r.Content.Reason, width, face) + 8
	}
	if r.Content.Code != "" {
		h += min(int(150*scale), max(int(50*scale), (strings.Count(r.Content.Code, "\n")+1)*(nucular.FontHeight(face)+5))) + int(30*scale)
	}
	if r.Content.Caption != "" {
		h += approvalTextHeight(w, r.Content.Caption, width, face) + 6
	}
	for _, line := range r.Content.Details {
		if line != "" {
			h += approvalTextHeight(w, line, width, face) + 6
		}
	}
	if len(r.Content.Diff) > 0 {
		h += min(int(240*scale), len(r.Content.Diff)*int(24*scale)) + int(30*scale)
	}
	for _, q := range r.Questions {
		h += int(60*scale) + approvalTextHeight(w, q.Text, width, face) + (len(q.Options)+len(q.Descriptions))*int(34*scale)
		if q.Error != "" {
			h += int(40 * scale)
		}
	}
	return h
}
func approvalCardDimensions(w *nucular.Window, r *approval) (int, int) {
	footer := approvalFooterHeight(w, r)
	padding := 2*w.Master().Style().GroupWindow.Padding.Y + 12
	// The body scrolls independently; decisions stay visible even for long context.
	available := max(int(120*approvalScale(w)), min(int(480*approvalScale(w)), w.Bounds.H*3/4-footer))
	if r.HeightLimit > 0 {
		available = min(available, max(int(48*approvalScale(w)), r.HeightLimit-footer-padding))
	}
	body := min(approvalBodyHeight(w, r), available)
	total := body + footer + padding
	if r.HeightLimit > 0 {
		total = min(total, r.HeightLimit)
	}
	return body, total
}
func (a *App) approvalHeight(w *nucular.Window, thread string, limit ...int) int {
	for i := range a.approvals {
		if a.approvals[i].ThreadID == thread {
			a.approvals[i].HeightLimit = 0
			if len(limit) > 0 {
				a.approvals[i].HeightLimit = max(60, limit[0]-w.Master().Style().GroupWindow.Spacing.Y)
			}
			_, height := approvalCardDimensions(w, &a.approvals[i])
			return height + w.Master().Style().GroupWindow.Spacing.Y
		}
	}
	return 0
}
func (a *App) drawApprovalCard(w *nucular.Window, r *approval, count int) {
	bodyHeight, total := approvalCardDimensions(w, r)
	style := w.Master().Style()
	previous := style.GroupWindow
	style.GroupWindow.Border = 1
	style.GroupWindow.BorderColor = a.p.BorderStrong
	if r.Focused {
		style.GroupWindow.BorderColor = a.p.Accent
	}
	w.RowScaled(total).Dynamic(1)
	// Very short windows can scroll the entire card when even the decision
	// rows exceed the available area. Never clip away an action permanently.
	box := w.GroupBegin("approval", nucular.WindowBorder|nucular.WindowNoHScrollbar)
	if box == nil {
		style.GroupWindow = previous
		return
	}
	box.RowScaled(bodyHeight).Dynamic(1)
	if body := box.GroupBegin("approval-body", nucular.WindowNoHScrollbar); body != nil {
		a.drawApprovalBody(body, r, count)
		body.GroupEnd()
	}
	a.drawApprovalActions(box, r)
	box.GroupEnd()
	style.GroupWindow = previous
}
func (a *App) drawApprovalBody(w *nucular.Window, r *approval, count int) {
	face := w.Master().Style().Font
	large := typeFace(fontPointSize(face)+3, boldFont)
	w.Master().Style().Font = large
	w.RowScaled(approvalTextHeight(w, r.Title, w.Bounds.W-60, large)).Ratio(.04, .96)
	b, out := w.Custom(w.CustomState())
	if out != nil {
		size := max(6, int(8*approvalScale(w)))
		out.FillCircle(rect.Rect{X: b.X + (b.W-size)/2, Y: b.Y + (b.H-size)/2, W: size, H: size}, a.p.Warning)
	}
	w.LabelWrap(r.Title)
	if w.Input().Mouse.Clicked(mouse.ButtonLeft, w.LastWidgetBounds) {
		r.Focused = true
		r.Armed = time.Now().Add(300 * time.Millisecond)
	}
	w.Master().Style().Font = face
	if count > 1 {
		muted(w, fmt.Sprintf("1 of %d", count), a.p)
	}
	if r.OriginThreadID != r.ThreadID {
		muted(w, "From sub-agent "+r.OriginThreadID, a.p)
	}
	drawText := func(value string) {
		if value != "" {
			w.RowScaled(approvalTextHeight(w, value, w.Bounds.W-40, face)).Dynamic(1)
			w.LabelWrap(value)
		}
	}
	drawText(r.Content.Reason)
	if r.Content.Code != "" {
		w.Row(26).Static(95)
		if w.ButtonText("Copy code") {
			a.copyText(r.Content.Code)
		}
		w.Master().Style().Font = typeFace(max(8, fontPointSize(face)-1), monoFont)
		h := min(int(150*approvalScale(w)), max(int(50*approvalScale(w)), (strings.Count(r.Content.Code, "\n")+1)*(nucular.FontHeight(w.Master().Style().Font)+5)))
		w.RowScaled(h).Dynamic(1)
		approvalCodeEditor(r).Edit(w)
		w.Master().Style().Font = face
	}
	drawText(r.Content.Caption)
	for _, detail := range r.Content.Details {
		drawText(detail)
	}
	if len(r.Content.Diff) > 0 {
		w.Row(26).Static(95)
		if w.ButtonText("Copy diff") {
			lines := make([]string, 0, len(r.Content.Diff))
			for _, line := range r.Content.Diff {
				lines = append(lines, line.Text)
			}
			a.copyText(strings.Join(lines, "\n"))
		}
		w.RowScaled(min(int(240*approvalScale(w)), len(r.Content.Diff)*int(24*approvalScale(w)))).Dynamic(1)
		if diff := w.GroupBegin("approval-diff", 0); diff != nil {
			old := w.Master().Style().Font
			w.Master().Style().Font = typeFace(max(8, fontPointSize(old)-1), monoFont)
			for _, line := range r.Content.Diff {
				fg, bg := a.p.Text, a.p.Sunken
				switch line.Kind {
				case "add":
					fg, bg = a.p.DiffAddFG, a.p.DiffAddBG
				case "remove":
					fg, bg = a.p.DiffDelFG, a.p.DiffDelBG
				case "hunk":
					fg = a.p.DiffHunkFG
				case "file":
					fg = a.p.Accent
				case "note":
					fg = a.p.Muted
				}
				diff.Row(24).Dynamic(1)
				bounds, out := diff.Custom(diff.CustomState())
				if out != nil {
					out.FillRect(bounds, 0, bg)
					labelAt(out, inset(bounds, 6, 0), line.Text, w.Master().Style().Font, fg)
				}
			}
			w.Master().Style().Font = old
			diff.GroupEnd()
		}
	}
	for i := range r.Questions {
		a.drawQuestion(w, &r.Questions[i])
		if r.Questions[i].Editor != nil && r.Questions[i].Editor.Active {
			r.Focused = true
		}
	}
	if r.FormError != "" {
		errors := false
		for _, q := range r.Questions {
			errors = errors || q.Error != ""
		}
		if !errors {
			r.FormError = ""
		}
	}
	w.Row(24).Static(190)
	if w.ButtonText("View full request details") {
		a.openApprovalDetails(r)
	}
}
func (a *App) drawApprovalChoice(w *nucular.Window, r *approval, i int, c approvalChoice) bool {
	w.RowScaled(approvalChoiceHeight(w, c)).Ratio(.08, .92)
	p := a.p
	if i == approvalCancelIndex(r.Choices) || c.Key == 'd' || c.Key == 'b' {
		p.Accent = p.Danger
	} else if c.Key == 'p' || c.Key == 'h' || c.Key == 'r' {
		p.Accent = p.Warning
	} else {
		p.Accent = p.Success
	}
	w.LabelColored("["+strings.ToUpper(string(c.Key))+"]", "CC", p.Accent)
	bounds, out := w.Custom(w.CustomState())
	if out == nil {
		return false
	}
	bg := p.Surface
	if w.Input().Mouse.HoveringRect(bounds) {
		bg = p.Hover
	}
	border := p.BorderStrong
	if r.Focused && r.Selected == i {
		border = a.p.Accent
		bg = p.Selected
	}
	out.FillRect(bounds, 4, border)
	out.FillRect(inset(bounds, 1, 1), 3, bg)
	face := w.Master().Style().Font
	lines := approvalWrap(w, c.Title, bounds.W-20, face)
	lineHeight := nucular.FontHeight(face) + 4
	for j, line := range lines {
		out.DrawText(rect.Rect{X: bounds.X + 10, Y: bounds.Y + 6 + j*lineHeight, W: bounds.W - 20, H: lineHeight}, line, face, p.Text)
	}
	return w.Input().Mouse.Clicked(mouse.ButtonLeft, bounds)
}
