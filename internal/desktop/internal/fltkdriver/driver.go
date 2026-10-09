//go:build !fltk_headless

// Package fltkdriver owns FLTK's native windows and event loop. Custom control
// layout and rasterization run on the existing, separately locked UI owner.
package fltkdriver

import (
	"errors"
	"image"
	"runtime"
	"sync"
	"sync/atomic"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop/internal/windowing"
	"github.com/pwiecz/go-fltk"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/lifecycle"
	"golang.org/x/mobile/event/mouse"
	"golang.org/x/mobile/event/paint"
	"golang.org/x/mobile/event/size"
)

// FLTK and Cocoa require the event loop on the process's original OS thread.
// This does not open a display; --version/--doctor and tests remain non-GUI.
func init() { runtime.LockOSThread() }

type display struct{ ready chan *window }
type buffer struct{ pixels *image.RGBA }

func (b *buffer) Release()          {}
func (b *buffer) Size() image.Point { return b.pixels.Rect.Size() }
func (b *buffer) RGBA() *image.RGBA { return b.pixels }
func (d *display) NewBuffer(sz image.Point) (windowing.Buffer, error) {
	if sz.X <= 0 || sz.Y <= 0 {
		return nil, errors.New("invalid buffer size")
	}
	return &buffer{image.NewRGBA(image.Rectangle{Max: sz})}, nil
}
func (d *display) NewWindow(o *windowing.NewWindowOptions) (windowing.Window, error) {
	w := &window{events: windowing.NewEvents(), title: o.Title, width: o.Width, height: o.Height}
	d.ready <- w
	return w, nil
}

type window struct {
	events        *windowing.Events
	closed        atomic.Bool
	waking        atomic.Bool
	mu            sync.Mutex
	surface       windowing.Surface
	title         string
	titleDirty    bool
	icon          image.Image
	iconDirty     bool
	width, height int
	native        *fltk.Window // native and cached are accessed only on the FLTK thread
	canvas        *fltk.Box
	cached        *fltk.RgbImage
	iconImage     *fltk.RgbImage
	buttons       [4]bool
	keys          map[int]bool
	controls      []windowing.Control
	widgets       map[uint64]*nativeWidget
}

func Main(f func(windowing.Display)) {
	// Initialize FLTK's cross-thread wake queue before the layout owner can
	// publish its first frame.
	fltk.Lock()
	d := &display{ready: make(chan *window, 1)}
	done := make(chan struct{})
	go func() { defer close(done); f(d) }()
	w := <-d.ready
	fltk.SetKeyboardScreenScaling(false) // application shortcuts own Ctrl/Cmd +/-
	w.native = fltk.NewWindow(w.width, w.height, w.title)
	w.native.SetSizeRange(320, 240, 0, 0, 0, 0, false)
	w.canvas = fltk.NewBox(fltk.NO_BOX, 0, 0, w.width, w.height)
	w.canvas.SetDrawHandler(func(func()) { w.draw() })
	w.canvas.SetEventHandler(w.handle)
	w.canvas.SetResizeHandler(w.resize)
	w.native.Resizable(w.canvas)
	w.native.SetCallback(func() {
		// Do not hide first: the application may veto closing for unsaved work.
		w.Send(lifecycle.Event{To: lifecycle.StageDead})
	})
	w.native.End()
	w.native.Show()
	w.canvas.TakeFocus()
	w.resize()
	w.Send(lifecycle.Event{To: lifecycle.StageFocused})
	fltk.Run()
	// The native loop returns after Release hides the window. Finish draining
	// the UI owner's shutdown before the application saves its session.
	<-done
	if w.cached != nil {
		w.cached.Destroy()
		w.cached = nil
	}
	w.native.Destroy()
	w.native = nil
	fltk.Check() // process deferred widget deletion while callback owners live
	if w.iconImage != nil {
		w.iconImage.Destroy()
	}
	fltk.Unlock()
}

func (w *window) NextEvent() any { return w.events.Next() }
func (w *window) Send(e any)     { w.events.Send(e) }
func (w *window) Release()       { w.closed.Store(true); w.schedule() }
func (w *window) Upload(dp image.Point, src windowing.Buffer, sr image.Rectangle) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.surface.Ensure(image.Pt(w.width, w.height))
	w.surface.Upload(dp, src.RGBA(), sr)
}
func (w *window) Publish() { w.schedule() }
func (w *window) SetTitle(title string) {
	w.mu.Lock()
	w.title, w.titleDirty = title, true
	w.mu.Unlock()
	w.schedule()
}
func (w *window) SetIcon(icon image.Image) {
	w.mu.Lock()
	w.icon, w.iconDirty = icon, true
	w.mu.Unlock()
	w.schedule()
}

// Only one callback can be queued, regardless of streaming update rate.
// FLTK's Awake is the only native operation permitted off its event thread.
func (w *window) schedule() {
	if !w.waking.CompareAndSwap(false, true) {
		return
	}
	for !fltk.Awake(w.flush) {
		time.Sleep(time.Millisecond)
	}
}
func (w *window) flush() {
	w.waking.Store(false)
	if w.native == nil {
		return
	}
	if w.closed.Load() {
		w.native.Hide()
		return
	}
	w.mu.Lock()
	title, titleDirty := w.title, w.titleDirty
	icon, iconDirty := w.icon, w.iconDirty
	w.titleDirty, w.iconDirty = false, false
	w.mu.Unlock()
	w.syncControls()
	if titleDirty {
		w.native.SetLabel(title)
	}
	if iconDirty && icon != nil {
		if img, err := fltk.NewRgbImageFromImage(icon); err == nil {
			w.native.SetIcons([]*fltk.RgbImage{img})
			if w.iconImage != nil {
				w.iconImage.Destroy()
			}
			w.iconImage = img
		}
	}
	w.canvas.Redraw()
}
func (w *window) resize() {
	width, height := w.canvas.W(), w.canvas.H()
	if width < 1 || height < 1 {
		return
	}
	w.mu.Lock()
	w.width, w.height = width, height
	w.mu.Unlock()
	// FLTK supplies logical coordinates and applies monitor scaling itself.
	// Keep layout, pointer coordinates and rendered image in that same space.
	w.Send(size.Event{WidthPx: width, HeightPx: height, PixelsPerPt: 1})
	w.Send(paint.Event{})
}
func (w *window) draw() {
	if w.closed.Load() {
		return
	}
	w.mu.Lock()
	if w.surface.Pixels != nil && !w.surface.Damage.Empty() {
		view := w.surface.Pixels.Bounds().Intersect(image.Rect(0, 0, w.width, w.height))
		if !view.Empty() {
			img, err := fltk.NewRgbImageFromImage(w.surface.Pixels.SubImage(view))
			if err == nil {
				if w.cached != nil {
					w.cached.Destroy()
				}
				w.cached = img
				w.surface.Damage = image.Rectangle{}
			}
		}
	}
	w.mu.Unlock()
	if w.cached != nil {
		w.cached.Draw(0, 0, w.canvas.W(), w.canvas.H())
	}
}
func (w *window) handle(e fltk.Event) bool {
	if w.closed.Load() {
		return true
	}
	x, y := float32(fltk.EventX()), float32(fltk.EventY())
	switch e {
	case fltk.FOCUS:
		w.Send(lifecycle.Event{To: lifecycle.StageFocused})
	case fltk.UNFOCUS:
		for code := range w.keys {
			w.Send(key.Event{Code: keyCode(code), Direction: key.DirRelease})
		}
		clear(w.keys)
		for b, down := range w.buttons {
			if down {
				w.Send(mouse.Event{X: x, Y: y, Button: mouse.Button(b), Direction: mouse.DirRelease})
			}
		}
		clear(w.buttons[:])
		w.Send(lifecycle.Event{From: lifecycle.StageFocused, To: lifecycle.StageVisible})
	case fltk.PUSH, fltk.RELEASE:
		b := int(fltk.EventButton())
		if b < 1 || b >= len(w.buttons) {
			return false
		}
		w.buttons[b] = e == fltk.PUSH
		direction := mouse.DirRelease
		if e == fltk.PUSH {
			direction = mouse.DirPress
			w.canvas.TakeFocus()
		}
		w.Send(mouse.Event{X: x, Y: y, Button: mouse.Button(b), Direction: direction})
	case fltk.MOVE, fltk.DRAG, fltk.ENTER:
		w.Send(mouse.Event{X: x, Y: y})
	case fltk.MOUSEWHEEL:
		w.Send(windowing.Wheel{X: x, Y: y, DeltaX: -float32(fltk.EventDX()), DeltaY: -float32(fltk.EventDY())})
	case fltk.KEYDOWN, fltk.KEYUP:
		code := fltk.EventKey()
		if w.keys == nil {
			w.keys = make(map[int]bool)
		}
		direction := key.DirRelease
		if e == fltk.KEYDOWN {
			direction = key.DirPress
			if w.keys[code] {
				direction = key.DirNone
			}
			w.keys[code] = true
		} else {
			delete(w.keys, code)
		}
		mods := modifiers(fltk.EventState())
		w.Send(key.Event{Code: keyCode(code), Modifiers: mods, Direction: direction, Rune: -1})
		if e == fltk.KEYDOWN && mods&(key.ModControl|key.ModMeta) == 0 {
			for _, r := range fltk.EventText() {
				if r >= ' ' {
					w.Send(key.Event{Rune: r, Code: key.CodeUnknown, Modifiers: mods, Direction: key.DirPress})
				}
			}
		}
	default:
		return false
	}
	return true
}
