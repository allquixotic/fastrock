package nucular

import (
	"errors"
	"fmt"
	"sync"

	"github.com/aarzilli/nucular/command"
	"github.com/aarzilli/nucular/internal/clipboard"
)

type clipboardJob struct {
	kind  command.CommandKind
	text  string
	reply func(string, error)
}
type clipboardResult struct {
	id   uint64
	text string
	err  error
}
type clipboardTarget struct {
	valid     func() bool
	apply     func(string)
	submitted bool
}

var clipboardWorker struct {
	sync.Once
	jobs chan clipboardJob
}

// One bounded worker orders clipboard operations across all windows. Native
// clipboard calls never execute from the layout/rasterization owner.
func submitClipboard(job clipboardJob) error {
	clipboardWorker.Do(func() {
		clipboardWorker.jobs = make(chan clipboardJob, 64)
		go func() {
			for job := range clipboardWorker.jobs {
				executeClipboard(job)
			}
		}()
	})
	select {
	case clipboardWorker.jobs <- job:
		return nil
	default:
		return errors.New("Clipboard is busy; try again")
	}
}

var clipboardNativeStart sync.Once

func executeClipboard(job clipboardJob) {
	var value string
	var err error
	defer func() {
		if r := recover(); r != nil {
			err = fmt.Errorf("Clipboard unavailable: %v", r)
		}
		job.reply(value, err)
	}()
	clipboardNativeStart.Do(clipboard.Start)
	if job.kind == command.SetClipboardCmd {
		err = clipboard.Write(job.text)
	} else {
		value, err = clipboard.Read(job.kind == command.GetPrimarySelectionCmd)
	}
}

func (ctx *context) requestClipboard(c command.Command) {
	if target, ok := ctx.clipboardTargets[c.ClipboardID]; ok {
		target.submitted = true
		ctx.clipboardTargets[c.ClipboardID] = target
	}
	job := clipboardJob{kind: c.Kind, text: c.Text.String, reply: func(value string, err error) {
		select {
		case ctx.clipboardResults <- clipboardResult{c.ClipboardID, value, err}:
			ctx.mw.Changed()
		case <-ctx.clipboardClosed:
		}
	}}
	submit := ctx.clipboardSubmit
	if submit == nil {
		submit = submitClipboard
	}
	if err := submit(job); err != nil {
		delete(ctx.clipboardTargets, c.ClipboardID)
		ctx.reportClipboard(err)
	}
}

func (ctx *context) discardUndrawnClipboardTargets() {
	for id, target := range ctx.clipboardTargets {
		if !target.submitted {
			delete(ctx.clipboardTargets, id)
		}
	}
}

func (ctx *context) reportClipboard(err error) {
	if err != nil && ctx.clipboardError != nil {
		ctx.clipboardError(err)
	}
}

func (ctx *context) drainClipboard() {
	for range 64 {
		select {
		case r := <-ctx.clipboardResults:
			target, ok := ctx.clipboardTargets[r.id]
			delete(ctx.clipboardTargets, r.id)
			if r.err != nil {
				ctx.reportClipboard(r.err)
				continue
			}
			if !ok {
				continue
			}
			if target.valid != nil && !target.valid() {
				ctx.reportClipboard(errors.New("The editor changed while reading the clipboard; paste again"))
				continue
			}
			target.apply(r.text)
			ctx.mw.Changed()
		default:
			return
		}
	}
}

// OnClipboardError receives native failures and canceled pastes on the UI
// owner. Applications can display them through their normal notice system.
func (mw *masterWindowCommon) OnClipboardError(fn func(error)) { mw.ctx.clipboardError = fn }

// FrameID identifies the current layout pass for checking that a paste's
// original editor is still visible when a delayed clipboard result arrives.
func (win *Window) FrameID() uint64 { return win.ctx.drawGeneration }

// RequestClipboard keeps a read bound to its originating editor. Callbacks run
// after layout, on the UI owner. A false valid callback cancels the paste.
func (win *Window) RequestClipboard(valid func() bool, apply func(string)) {
	win.requestClipboardTarget(command.GetClipboardCmd, valid, apply)
}
func (win *Window) requestClipboardTarget(kind command.CommandKind, valid func() bool, apply func(string)) {
	ctx := win.ctx
	if len(ctx.clipboardTargets) >= 64 {
		ctx.reportClipboard(errors.New("Clipboard is busy; try again"))
		return
	}
	ctx.nextClipboardID++
	id := ctx.nextClipboardID
	if ctx.clipboardTargets == nil {
		ctx.clipboardTargets = make(map[uint64]clipboardTarget)
	}
	ctx.clipboardTargets[id] = clipboardTarget{valid: valid, apply: apply}
	win.cmds.GetClipboard()
	win.cmds.Commands[len(win.cmds.Commands)-1].ClipboardID = id
	win.cmds.Commands[len(win.cmds.Commands)-1].Kind = kind
}
