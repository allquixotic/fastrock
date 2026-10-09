package ui

import (
	"context"
	"fmt"
	"image"
	"sort"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/font"
	"github.com/allquixotic/fastrock/internal/desktop/label"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/richtext"
	"golang.org/x/mobile/event/mouse"
)

type detailCollectionRow struct {
	Body                           *transcriptLayout
	User, Date, Initials, Revision string
	Header, Height                 int
	Truncated                      bool
}
type detailCollectionLayout struct {
	Source                                   *rally.Object
	Length, Width, Size, Spacing, Generation int
	Tab                                      string
	Scale                                    float64
	Face                                     font.Face
	Rows                                     []detailCollectionRow
	Offsets                                  []int
}

func collectionText(o rally.Object) string {
	return fallback(o.String("Text"), o.String("Description"))
}
func postInitials(name string) string {
	var result []rune
	for _, word := range strings.Fields(cut(name, 200)) {
		r, _ := utf8.DecodeRuneInString(word)
		result = append(result, r)
		if len(result) == 2 {
			break
		}
	}
	if len(result) == 0 {
		return "?"
	}
	return strings.ToUpper(string(result))
}
func postDate(value string) string {
	if date, err := time.Parse(time.RFC3339, value); err == nil {
		return date.UTC().Format("2006-01-02 15:04 UTC")
	}
	return fallback(cut(value, 40), "Date unavailable")
}
func prepareDetailCollection(l *detailCollectionLayout, items []rally.Object) {
	l.Offsets = []int{0}
	budget := 512 << 10
	for _, item := range items {
		source := collectionText(item)
		limit := min(len(source), 8192, budget)
		for limit > 0 && limit < len(source) && !utf8.RuneStart(source[limit]) {
			limit--
		}
		budget -= limit
		preview := source[:limit]
		row := detailCollectionRow{User: fallback(item.String("User"), "Unknown author"), Date: postDate(item.String("CreationDate")), Revision: fallback(item.String("RevisionNumber"), "—"), Truncated: limit < len(source)}
		row.Initials = postInitials(row.User)
		width := l.Width - int(24*l.Scale)
		if l.Tab == "Revisions" {
			width = int(float64(l.Width)*.48) - int(16*l.Scale)
		}
		row.Body = prepareDocumentTranscript(richtext.Parse(preview), preview, max(40, width), l.Size)
		if l.Tab == "Discussions" {
			row.Header = int(34 * l.Scale)
			if l.Width < int(520*l.Scale) {
				row.Header = int(58 * l.Scale)
			}
			row.Height = row.Header + row.Body.Height + int(20*l.Scale)
		} else {
			row.Height = max(int(34*l.Scale), row.Body.Height+int(12*l.Scale))
		}
		if row.Truncated {
			row.Height += int(28 * l.Scale)
		}
		l.Rows = append(l.Rows, row)
		l.Offsets = append(l.Offsets, l.Offsets[len(l.Offsets)-1]+row.Height+l.Spacing)
	}
}

func (a *App) collectionTextLayout(w *desktop.Window, d *detailView, items []rally.Object) *detailCollectionLayout {
	var source *rally.Object
	if len(items) > 0 {
		source = &items[0]
	}
	key := detailCollectionLayout{Source: source, Length: len(items), Width: w.LayoutAvailableWidth(), Size: fontPointSize(w.Master().Style().Font), Spacing: w.WindowStyle().Spacing.Y, Generation: d.collectionGeneration, Tab: d.Tab, Scale: w.Master().Style().Scaling, Face: w.Master().Style().Font}
	matches := func(l *detailCollectionLayout) bool {
		return l != nil && l.Source == key.Source && l.Length == key.Length && l.Width == key.Width && l.Size == key.Size && l.Spacing == key.Spacing && l.Generation == key.Generation && l.Tab == key.Tab && l.Scale == key.Scale && l.Face == key.Face
	}
	if matches(d.collectionLayout) {
		return d.collectionLayout
	}
	if matches(d.collectionLayoutPending) {
		return nil
	}
	d.collectionLayoutPending = &key
	a.work(func() {
		prepareDetailCollection(&key, items)
		a.post(func() {
			if d.collectionLayoutPending == &key && d.collectionGeneration == key.Generation && d.Tab == key.Tab {
				d.collectionLayout = &key
				d.collectionLayoutPending = nil
			}
		})
	}, func() {
		if d.collectionLayoutPending == &key {
			d.collectionLayoutPending = nil
		}
	})
	return nil
}

func collectionSkip(w *desktop.Window, height, spacing int) {
	if height > 0 {
		w.RowScaled(max(1, height-spacing)).Dynamic(1)
		w.Spacing(1)
	}
}
func (a *App) drawCollectionText(w *desktop.Window, v *rallyView, d *detailView, items []rally.Object) {
	if len(items) == 0 {
		return
	}
	l := a.collectionTextLayout(w, d, items)
	if l == nil {
		muted(w, "Preparing collection…", a.p)
		return
	}
	if d.Tab == "Revisions" {
		w.Row(30).Ratio(.1, .24, .18, .48)
		for _, s := range []string{"Revision", "Date", "User", "Changes"} {
			w.Label(s, "LC")
		}
	}
	top := w.LayoutNextRowY()
	clip := w.Commands().Clip
	first := sort.Search(len(l.Rows), func(i int) bool { return top+l.Offsets[i+1] > clip.Y })
	last := sort.Search(len(l.Rows), func(i int) bool { return top+l.Offsets[i] >= clip.Y+clip.H })
	last = max(first, last)
	collectionSkip(w, l.Offsets[first], l.Spacing)
	for i := first; i < last; i++ {
		w.RowScaled(l.Rows[i].Height).Dynamic(1)
		b, out := w.Custom(w.CustomState())
		if out != nil {
			a.drawCollectionRow(w, out, b, v, d, items[i], l.Rows[i], l.Scale)
		}
	}
	collectionSkip(w, l.Offsets[len(l.Rows)]-l.Offsets[last], l.Spacing)
}

func (a *App) drawCollectionRow(w *desktop.Window, out *command.Buffer, b rect.Rect, v *rallyView, d *detailView, item rally.Object, row detailCollectionRow, scale float64) {
	pad := int(8 * scale)
	textBox := inset(b, pad, pad)
	if d.Tab == "Discussions" {
		out.FillRect(b, 4, a.p.Surface)
		avatar := rect.Rect{X: b.X + pad, Y: b.Y + pad, W: int(24 * scale), H: int(24 * scale)}
		out.FillCircle(avatar, a.p.Selected)
		labelAt(out, inset(avatar, 2, 0), row.Initials, w.Master().Style().Font, a.p.Text)
		name := rect.Rect{X: avatar.X + avatar.W + pad, Y: b.Y, W: max(0, b.W-int(300*scale)), H: int(34 * scale)}
		date := rect.Rect{X: b.X + b.W - int(250*scale), Y: b.Y, W: int(180 * scale), H: int(34 * scale)}
		if row.Header > int(34*scale) {
			name.W = max(0, b.W-int(125*scale))
			date.X = name.X
			date.Y += int(26 * scale)
		}
		labelAt(out, name, cut(row.User, 200), w.Master().Style().Font, a.p.Text)
		out.FillRect(inset(date, 2, 4), 4, a.p.Alt)
		labelAt(out, inset(date, 5, 0), row.Date, w.Master().Style().Font, a.p.Muted)
		del := rect.Rect{X: b.X + b.W - int(68*scale), Y: b.Y + pad, W: int(60 * scale), H: int(24 * scale)}
		color := a.p.Danger
		if d.Pending || d.CollectionLoading || a.rallyClient == nil {
			color = a.p.Faint
		}
		labelAt(out, del, "Delete", w.Master().Style().Font, color)
		if w.Input().Mouse.Clicked(mouse.ButtonLeft, del) && !d.Pending && !d.CollectionLoading && a.rallyClient != nil {
			post := item.Clone()
			a.confirm("Delete comment?", "Delete this comment by "+cut(row.User, 100)+" from "+row.Date+"?", func() { a.deleteDiscussion(v, d, post) })
		}
		textBox.Y = b.Y + row.Header
		textBox.H = b.H - row.Header - pad
	} else {
		widths := []float64{.1, .24, .18}
		values := []string{row.Revision, row.Date, row.User}
		x := b.X
		for i, width := range widths {
			cell := rect.Rect{X: x + pad, Y: b.Y, W: max(0, int(float64(b.W)*width)-pad), H: int(34 * scale)}
			labelAt(out, cell, cut(values[i], 200), w.Master().Style().Font, a.p.Muted)
			x += int(float64(b.W) * width)
		}
		textBox.X = x + pad
		textBox.W = max(0, b.X+b.W-textBox.X-pad)
		out.StrokeLine(image.Pt(b.X, b.Y+b.H-1), image.Pt(b.X+b.W, b.Y+b.H-1), 1, a.p.Border)
	}
	a.paintCollectionText(w, out, textBox, row.Body)
	if row.Truncated {
		link := rect.Rect{X: textBox.X, Y: b.Y + b.H - int(28*scale), W: textBox.W, H: int(28 * scale)}
		labelAt(out, link, "Preview shortened · Open full source", w.Master().Style().Font, a.p.Accent)
		if w.Input().Mouse.Clicked(mouse.ButtonLeft, link) {
			a.openText("Collection source", collectionText(item))
		}
	}
	if menu := w.ContextualOpen(0, image.Pt(230, 85), b, nil); menu != nil {
		if menu.MenuItem(label.T("Copy displayed text")) {
			a.copyText(row.Body.Plain)
		}
		if menu.MenuItem(label.T("Open full source")) {
			a.openText("Collection source", collectionText(item))
		}
	}
}

func (a *App) paintCollectionText(w *desktop.Window, out *command.Buffer, b rect.Rect, l *transcriptLayout) {
	clip := out.Clip
	first := sort.Search(len(l.Lines), func(i int) bool { return b.Y+l.LineOffsets[i]+l.Lines[i].Height > clip.Y })
	for i := first; i < len(l.Lines) && b.Y+l.LineOffsets[i] < clip.Y+clip.H; i++ {
		line := l.Lines[i]
		for _, run := range line.Runs {
			q := rect.Rect{X: b.X + run.X, Y: b.Y + l.LineOffsets[i], W: run.Width + 2, H: line.Height}
			face := run.Face
			if face.Face == nil {
				face = w.Master().Style().Font
			}
			color := a.p.Text
			if run.Format.Link != "" {
				color = a.p.Accent
			}
			out.DrawText(q, run.Text, face, color)
			if run.Format.Style&richtext.Strike != 0 {
				out.StrokeLine(image.Pt(q.X, q.Y+q.H/2), image.Pt(q.X+q.W, q.Y+q.H/2), 1, color)
			}
			if run.Format.Link != "" {
				out.StrokeLine(image.Pt(q.X, q.Y+q.H-2), image.Pt(q.X+q.W, q.Y+q.H-2), 1, color)
				if w.Input().Mouse.HoveringRect(q) {
					w.Tooltip(run.Format.Link)
				}
				if w.Input().Mouse.Clicked(mouse.ButtonLeft, q) {
					a.openLink(run.Format.Link)
				}
			}
		}
	}
}

func (a *App) deleteDiscussion(v *rallyView, d *detailView, post rally.Object) {
	if v.Closed || v.Detail != d || d.Pending || d.CollectionLoading || d.Loading || d.Saving || d.New || a.rallyClient == nil || d.Tab != "Discussions" {
		return
	}
	ref := post.String("_ref")
	if ref == "" {
		d.Error = "This comment has no reference; refresh the discussion"
		return
	}
	d.Pending = true
	c := a.rallyClient
	queued := a.writeWork(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		err := c.Delete(ctx, ref)
		a.post(func() {
			d.Pending = false
			if v.Closed || v.Detail != d {
				return
			}
			if a.rallyClient != c {
				d.Error = "Rally connection changed; refresh to confirm whether the comment was deleted"
				return
			}
			if err != nil {
				d.Error = fmt.Sprintf("Comment could not be deleted: %v", err)
				return
			}
			delete(d.collectionCounts, "Discussions")
			a.toast = "Comment deleted"
			if d.Tab == "Discussions" {
				a.loadCollection(d)
			}
		})
	}, func() { d.Pending = false; d.Error = errWorkQueueFull.Error() })
	if !queued && a.ctx.Err() != nil {
		d.Pending = false
		d.Error = a.ctx.Err().Error()
	}
}
