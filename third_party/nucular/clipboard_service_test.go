//go:build nucular_headless

package nucular

import (
	"errors"
	"image"
	"strings"
	"testing"

	"github.com/aarzilli/nucular/command"
)

func TestV4ClipboardPasteKeepsOriginalEditor(t *testing.T) {
	for _, change := range []string{"unchanged", "text", "cursor", "hidden", "failure"} {
		t.Run(change, func(t *testing.T) {
			ed := &TextEditor{Buffer: []rune("draft"), Flags: EditField}
			show, paste := true, false
			h := NewHeadlessHarness(WindowNoScrollbar, image.Pt(300, 160), func(w *Window) {
				if show {
					w.Row(28).Dynamic(1)
					ed.Edit(w)
				}
				if paste {
					ed.requestPaste()
					paste = false
				}
			})
			var jobs []clipboardJob
			var notice error
			h.window.ctx.clipboardSubmit = func(job clipboardJob) error { jobs = append(jobs, job); return nil }
			h.window.OnClipboardError(func(err error) { notice = err })
			h.Frame(true)
			ed.Active = true
			ed.Cursor, ed.SelectStart, ed.SelectEnd = 5, 5, 5
			paste = true
			h.Frame(true)
			if len(jobs) != 1 || string(ed.Buffer) != "draft" {
				t.Fatalf("renderer did not queue read: %d %s", len(jobs), string(ed.Buffer))
			}
			switch change {
			case "text":
				ed.Paste(" typed")
			case "cursor":
				ed.Cursor = 0
			case "hidden":
				show = false
			}
			var err error
			if change == "failure" {
				err = errors.New("clipboard is unavailable")
			}
			done := make(chan struct{})
			go func() { jobs[0].reply(" pasted", err); close(done) }()
			<-done
			h.Frame(true)
			if change == "unchanged" {
				if string(ed.Buffer) != "draft pasted" || notice != nil {
					t.Fatalf("paste not delivered: %s %v", string(ed.Buffer), notice)
				}
			} else if strings.Contains(string(ed.Buffer), "pasted") || notice == nil {
				t.Fatalf("stale paste accepted or no feedback: %s %v", string(ed.Buffer), notice)
			}
			if len(h.window.ctx.clipboardTargets) != 0 {
				t.Fatal("completed request retained")
			}
		})
	}
}

func TestV4ClipboardAdmissionAndUndrawnTargets(t *testing.T) {
	h := NewHeadlessHarness(WindowNoScrollbar, image.Pt(200, 100), nil)
	w := h.window.ctx.Windows[0]
	var notice error
	h.window.OnClipboardError(func(err error) { notice = err })
	w.RequestClipboard(nil, func(string) {})
	h.window.ctx.clipboardSubmit = func(clipboardJob) error { return errors.New("queue full") }
	c := w.cmds.Commands[len(w.cmds.Commands)-1]
	h.window.ctx.requestClipboard(c)
	if notice == nil || len(h.window.ctx.clipboardTargets) != 0 {
		t.Fatal("rejected job retained")
	}
	for range 1000 {
		w.RequestClipboard(nil, func(string) {})
	}
	if len(h.window.ctx.clipboardTargets) > 64 {
		t.Fatal("unbounded pending clipboard")
	}
	h.window.ctx.discardUndrawnClipboardTargets()
	if len(h.window.ctx.clipboardTargets) != 0 {
		t.Fatal("discarded layout retained callbacks")
	}
	// A failed write is also surfaced, without requiring an editor callback.
	h.window.ctx.clipboardSubmit = func(job clipboardJob) error { job.reply("", errors.New("write failed")); return nil }
	h.window.ctx.requestClipboard(command.Command{Kind: command.SetClipboardCmd})
	h.window.ctx.drainClipboard()
	if notice == nil || notice.Error() != "write failed" {
		t.Fatal(notice)
	}
}
