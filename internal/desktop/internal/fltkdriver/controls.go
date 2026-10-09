//go:build !fltk_headless

package fltkdriver

import (
	"image/color"
	"strings"

	"github.com/allquixotic/fastrock/internal/desktop/internal/windowing"
	"github.com/pwiecz/go-fltk"
	"golang.org/x/mobile/event/key"
)

type nativeWidget struct {
	desc                          windowing.Control
	button                        *fltk.Button
	input                         *fltk.Input
	sequence                      uint64
	base                          string
	capturePending, commitPending bool
}

func rgb(c color.RGBA) fltk.Color { return fltk.ColorFromRgb(c.R, c.G, c.B) }

func (w *window) Controls(controls []windowing.Control) {
	w.mu.Lock()
	w.controls = controls
	w.mu.Unlock()
	w.schedule()
}

func (w *window) syncControls() {
	w.mu.Lock()
	descriptors := w.controls
	w.mu.Unlock()
	if w.widgets == nil {
		w.widgets = make(map[uint64]*nativeWidget)
	}
	live := make(map[uint64]bool, len(descriptors))
	for _, d := range descriptors {
		live[d.ID] = true
		n := w.widgets[d.ID]
		if n != nil && n.desc == d {
			continue
		}
		// The wrapper lacks per-input text size; recreate when its font changes.
		if n != nil && n.input != nil && n.desc.FontSize != d.FontSize {
			n.input.Destroy()
			delete(w.widgets, d.ID)
			n = nil
		}
		if n == nil {
			n = &nativeWidget{desc: d, sequence: d.Sequence, base: d.Text}
			w.widgets[d.ID] = n
			w.native.Begin()
			if d.Kind == windowing.ButtonControl {
				n.button = fltk.NewButton(d.Bounds.Min.X, d.Bounds.Min.Y, d.Bounds.Dx(), d.Bounds.Dy())
				n.button.SetBox(fltk.RFLAT_BOX)
				n.button.SetDownBox(fltk.RFLAT_BOX)
				n.button.SetDrawHandler(func(draw func()) {
					draw()
					b := n.desc.Bounds
					fltk.SetDrawColor(rgb(n.desc.Border))
					for i := 0; i < n.desc.BorderWidth; i++ {
						fltk.DrawRect(b.Min.X+i, b.Min.Y+i, b.Dx()-2*i, b.Dy()-2*i)
					}
				})
				n.button.SetCallback(func() { w.Send(windowing.ControlEvent{ID: d.ID, Kind: windowing.ButtonControl}) })
				n.button.SetEventHandler(func(e fltk.Event) bool {
					if e == fltk.FOCUS || e == fltk.PUSH && fltk.EventButton() == fltk.LeftMouse {
						w.Send(windowing.ControlFocusEvent{ID: d.ID})
					}
					if e == fltk.MOVE || e == fltk.ENTER || e == fltk.MOUSEWHEEL {
						w.handle(e)
					}
					if (e == fltk.PUSH || e == fltk.RELEASE) && fltk.EventButton() == fltk.RightMouse {
						return w.handle(e)
					}
					if e == fltk.KEYDOWN && fltk.EventKey() != ' ' && fltk.EventKey() != fltk.ENTER_KEY {
						return w.handle(e)
					}
					return false
				})
			} else {
				setInputFontSize(d.FontSize)
				n.input = fltk.NewInput(d.Bounds.Min.X, d.Bounds.Min.Y, d.Bounds.Dx(), d.Bounds.Dy())
				n.input.SetBox(fltk.BORDER_BOX)
				n.input.SetValue(d.Text)
				n.input.SetCallbackCondition(fltk.WhenChanged)
				n.input.SetCallback(func() { w.captureInput(n, false) })
				n.input.SetEventHandler(func(e fltk.Event) bool {
					if e == fltk.FOCUS || e == fltk.PUSH && fltk.EventButton() == fltk.LeftMouse {
						w.Send(windowing.ControlFocusEvent{ID: d.ID})
					}
					if e == fltk.KEYDOWN {
						code, mods := keyCode(fltk.EventKey()), modifiers(fltk.EventState())
						editing := mods&(key.ModControl|key.ModMeta) != 0 && (code == key.CodeA || code == key.CodeC || code == key.CodeV || code == key.CodeX || code == key.CodeZ || code == key.CodeY)
						if !editing && code != key.CodeReturnEnter {
							w.Send(key.Event{Code: code, Modifiers: mods, Direction: key.DirPress, Rune: -1})
						}
						w.deferCapture(n, code == key.CodeReturnEnter)
						if code == key.CodeEscape {
							return true
						}
					} else if e == fltk.FOCUS || e == fltk.UNFOCUS || e == fltk.RELEASE {
						w.deferCapture(n, false)
					}
					if e == fltk.MOUSEWHEEL {
						return w.handle(e)
					}
					if e == fltk.MOVE || e == fltk.ENTER {
						w.handle(e)
					}
					if (e == fltk.PUSH || e == fltk.RELEASE) && fltk.EventButton() == fltk.RightMouse {
						return w.handle(e)
					}
					return false
				})
				n.input.SetDrawHandler(func(draw func()) {
					draw()
					if n.input.Value() == "" && n.desc.Placeholder != "" {
						fltk.SetDrawColor(rgb(n.desc.Foreground))
						fltk.SetDrawFont(fltk.HELVETICA, n.desc.FontSize)
						b := n.desc.Bounds
						fltk.Draw(n.desc.Placeholder, b.Min.X+4, b.Min.Y, max(0, b.Dx()-8), b.Dy(), fltk.ALIGN_LEFT|fltk.ALIGN_INSIDE|fltk.ALIGN_CLIP)
					}
				})
			}
			w.native.End()
		}
		n.desc = d
		b := d.Bounds
		if n.button != nil {
			n.button.Resize(b.Min.X, b.Min.Y, b.Dx(), b.Dy())
			n.button.SetLabel(strings.ReplaceAll(d.Text, "@", "@@"))
			n.button.SetLabelSize(d.FontSize)
			n.button.SetLabelColor(rgb(d.Foreground))
			n.button.SetColor(rgb(d.Background))
			n.button.SetSelectionColor(rgb(d.Selection))
		} else {
			fltk.SetForegroundColor(d.Foreground.R, d.Foreground.G, d.Foreground.B)
			n.input.Resize(b.Min.X, b.Min.Y, b.Dx(), b.Dy())
			n.input.SetColor(rgb(d.Background))
			n.input.SetSelectionColor(rgb(d.Selection))
			// Never roll back typing with an older layout snapshot.
			if d.Sequence >= n.sequence {
				n.sequence, n.base = d.Sequence, d.Text
				if n.input.Value() != d.Text {
					n.input.SetValue(d.Text)
				}
				n.input.SetInsertPosition(d.Cursor, d.Mark)
				if d.Focused && !n.input.HasFocus() {
					n.input.TakeFocus()
				}
			}
		}
	}
	for id, n := range w.widgets {
		if live[id] {
			continue
		}
		delete(w.widgets, id)
		if n.input != nil {
			n.input.Destroy()
		} else {
			n.button.Destroy()
		}
	}
}

func (w *window) deferCapture(n *nativeWidget, committed bool) {
	n.commitPending = n.commitPending || committed
	if n.capturePending {
		return
	}
	n.capturePending = true
	fltk.AddTimeout(0, func() {
		n.capturePending = false
		if w.closed.Load() || w.widgets[n.desc.ID] != n {
			return
		}
		committed := n.commitPending
		n.commitPending = false
		w.captureInput(n, committed)
	})
}

func (w *window) captureInput(n *nativeWidget, committed bool) {
	if w.closed.Load() || w.widgets[n.desc.ID] != n {
		return
	}
	n.sequence++
	value := n.input.Value()
	w.Send(windowing.ControlEvent{ID: n.desc.ID, Kind: windowing.InputControl, Sequence: n.sequence,
		Base: n.base, Text: value, Cursor: n.input.InsertPosition(), Mark: n.input.Mark(),
		Focused: n.input.HasFocus(), Committed: committed})
	n.base = value
}
