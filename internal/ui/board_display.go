package ui

import (
	"fmt"
	"hash/fnv"
	"image/color"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/settings"
)

type boardDisplayDraft struct {
	ColorBy  string
	WIP, Age *desktop.TextEditor
	Error    string
}
type boardDisplayDraftState struct{ ColorBy, WIP, Age, Error string }

func (d *boardDisplayDraft) snapshot() *boardDisplayDraftState {
	if d == nil {
		return nil
	}
	return &boardDisplayDraftState{d.ColorBy, text(d.WIP), text(d.Age), d.Error}
}
func (s *boardDisplayDraftState) restore() *boardDisplayDraft {
	if s == nil {
		return nil
	}
	return &boardDisplayDraft{ColorBy: s.ColorBy, WIP: textEditor(s.WIP, false), Age: textEditor(s.Age, false), Error: s.Error}
}
func (d *boardDisplayDraft) matches(s *boardDisplayDraftState) bool {
	if d == nil || s == nil {
		return d == nil && s == nil
	}
	return *d.snapshot() == *s
}
func (v *rallyView) beginBoardSettings() {
	d := v.Display.Normalized()
	v.DisplayDraft = &boardDisplayDraft{ColorBy: d.ColorBy, WIP: textEditor(strconv.Itoa(d.WIPLimit), false), Age: textEditor(strconv.Itoa(d.AgeDays), false)}
}
func (a *App) setBoardDisplay(v *rallyView, d settings.BoardDisplay) {
	v.Display = d.Normalized()
	// Each queued preference snapshot owns a separate value. Other open boards
	// retain their current view; this preference supplies defaults for new ones.
	a.prefs.RallyDisplay = v.Display.Copy()
	a.savePrefs()
}
func (a *App) applyBoardSettings(v *rallyView) bool {
	d := v.DisplayDraft
	if d == nil {
		return false
	}
	wip, ew := strconv.Atoi(strings.TrimSpace(text(d.WIP)))
	age, ea := strconv.Atoi(strings.TrimSpace(text(d.Age)))
	if ew != nil || wip < 0 || wip > 100000 {
		d.Error = "WIP limit must be a whole number from 0 to 100000."
		return false
	}
	if ea != nil || age < 0 || age > 36500 {
		d.Error = "Age threshold must be a whole number from 0 to 36500 days."
		return false
	}
	next := v.Display
	next.WIPLimit, next.AgeDays, next.ColorBy = wip, age, d.ColorBy
	a.setBoardDisplay(v, next)
	v.DisplayDraft = nil
	return true
}
func (a *App) drawBoardDisplayControls(w *desktop.Window, v *rallyView) {
	board := v.Mode == "board"
	captions := []string{"Comfortable", "Compact"}
	if board {
		captions = append(captions, "Page settings")
	}
	compactRallyButtons(w, 26, captions, 20, func(i int) {
		if i < 2 {
			density := captions[i]
			if button(w, density, v.Display.Density == density, a.p) && v.Display.Density != density {
				d := v.Display
				d.Density = density
				a.setBoardDisplay(v, d)
			}
		} else if button(w, "Page settings", v.DisplayDraft != nil, a.p) {
			if v.DisplayDraft == nil {
				v.beginBoardSettings()
			} else {
				v.DisplayDraft = nil
			}
		}
	})
	if !board {
		return
	}
	d := v.DisplayDraft
	if d == nil {
		return
	}
	w.Row(26).Ratio(.5, .5)
	w.Label("Card color", "LC")
	colors := []string{"Work Item", "Owner", "Priority"}
	d.ColorBy = colors[w.ComboSimple(colors, index(colors, d.ColorBy), 26)]
	w.Row(26).Ratio(.5, .5)
	w.Label("WIP limit (0 = unlimited)", "LC")
	d.WIP.Edit(w)
	w.Row(26).Ratio(.5, .5)
	w.Label("Age threshold in days", "LC")
	d.Age.Edit(w)
	muted(w, "WIP counts loaded matching cards. Age is time since last update; 0 hides age warnings.", a.p)
	if d.Error != "" {
		muted(w, d.Error, a.p)
	}
	w.Row(28).Static(90, 90)
	if primary(w, "Apply", a.p) {
		a.applyBoardSettings(v)
	}
	if w.ButtonText("Cancel") {
		v.DisplayDraft = nil
	}
}

type boardCardMetrics struct {
	Height, Gap, Padding, Line, Header, TitleY, TitleLines, OwnerY, OwnerH, IterationY, FooterY, FooterH, StatusH int
}

func boardMetrics(density string, scale float64, face font.Face) boardCardMetrics {
	s := func(n int) int { return max(1, int(math.Round(float64(n)*scale))) }
	m := boardCardMetrics{Padding: s(12), Gap: s(10), Line: max(s(18), desktop.FontHeight(face)), TitleLines: 3}
	between, titleGap := s(4), s(10)
	if density == "Compact" {
		m.Padding, m.Gap, m.TitleLines, between, titleGap = s(8), s(7), 2, s(2), s(6)
	}
	m.Header = max(s(20), m.Line)
	m.TitleY = m.Padding + m.Header + titleGap
	m.OwnerY = m.TitleY + m.TitleLines*m.Line + between
	m.OwnerH = max(s(22), m.Line)
	m.IterationY = m.OwnerY + m.OwnerH + between
	m.FooterY = m.IterationY + m.Line + between
	m.FooterH, m.StatusH = max(s(22), m.Line), max(s(23), m.Line)
	m.Height = m.FooterY + m.FooterH + between + m.StatusH + s(1)
	return m
}

// Lane scroll offsets include the header. Preserve the same fractional card
// when density, font size or the destination window's DPI changes.
func boardScrollAtStride(scroll, old, next, oldHeader, nextHeader int) int {
	if old <= 0 || old == next && oldHeader == nextHeader {
		return scroll
	}
	if scroll <= oldHeader {
		return min(scroll, nextHeader)
	}
	return nextHeader + int(math.Round(float64(scroll-oldHeader)*float64(next)/float64(old)))
}
func (v *rallyView) updateBoardStride(stride, header int) {
	if v.boardStride > 0 && (v.boardStride != stride || v.boardHeader != header) {
		if v.RestoreLaneScroll == nil {
			v.RestoreLaneScroll = map[string]int{}
		}
		for key, value := range v.LaneScroll {
			v.RestoreLaneScroll[key] = boardScrollAtStride(value, v.boardStride, stride, v.boardHeader, header)
		}
		for key, value := range v.RestoreLaneScroll {
			if _, ok := v.LaneScroll[key]; !ok {
				v.RestoreLaneScroll[key] = boardScrollAtStride(value, v.boardStride, stride, v.boardHeader, header)
			}
		}
	}
	v.boardStride, v.boardHeader = stride, header
}
func boardLaneLabel(lane boardLane, limit int) (string, bool) {
	maxLabel := "∞"
	if limit > 0 {
		maxLabel = strconv.Itoa(limit)
	}
	return fmt.Sprintf("%s   %d/%s", lane.state, len(lane.cards), maxLabel), limit > 0 && len(lane.cards) > limit
}

func stableBoardColor(identity string, fallback color.RGBA) color.RGBA {
	if identity == "" {
		return fallback
	}
	h := fnv.New32a()
	_, _ = h.Write([]byte(identity))
	// The same identity keeps its color across page loads and sessions.
	colors := [...]uint32{0x21a2e0, 0x59a765, 0xa482cd, 0xdf76aa, 0xee9a45, 0x59b5aa, 0x9ca850}
	return hex(colors[int(h.Sum32())%len(colors)])
}
func (c *boardCard) workItemColor(p palette) color.RGBA {
	value := strings.TrimSpace(c.object.String("DisplayColor"))
	if len(value) == 7 && value[0] == '#' {
		if n, err := strconv.ParseUint(value[1:], 16, 24); err == nil {
			return hex(uint32(n))
		}
	}
	for _, known := range rallyColors {
		if strings.EqualFold(value, known.Name) {
			n, _ := strconv.ParseUint(known.Value[1:], 16, 24)
			return hex(uint32(n))
		}
	}
	return stableBoardColor(c.kind, p.Accent)
}
func (c *boardCard) displayColor(mode string, p palette) color.RGBA {
	switch mode {
	case "Owner":
		owner := c.object.Ref("Owner")
		if owner == "" && c.owner != "Unassigned" {
			owner = c.owner
		}
		return stableBoardColor(owner, p.Muted)
	case "Priority":
		return stableBoardColor(c.object.String("Priority"), p.Muted)
	default:
		return c.workItemColor(p)
	}
}
func boardAge(updated time.Time, threshold int, now time.Time) (int, bool) {
	if updated.IsZero() || updated.After(now) {
		return 0, false
	}
	days := int(now.Sub(updated) / (24 * time.Hour))
	return days, threshold > 0 && days >= threshold
}
func ellipsizeBoardTitle(face font.Face, line string, width int) string {
	runes := []rune(strings.TrimSpace(line))
	if desktop.FontWidth(face, "…") > width {
		return ""
	}
	lo, hi := 0, len(runes)
	for lo < hi {
		mid := (lo + hi + 1) / 2
		if desktop.FontWidth(face, string(runes[:mid])+"…") <= width {
			lo = mid
		} else {
			hi = mid - 1
		}
	}
	return string(runes[:lo]) + "…"
}
