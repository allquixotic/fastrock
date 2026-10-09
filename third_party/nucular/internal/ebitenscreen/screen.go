//go:build !nucular_headless

// Package ebitenscreen presents nucular's software-rendered frame using the
// CGo-free Ebitengine desktop driver. Pixels and widgets remain owned by nucular.
package ebitenscreen

import (
	"errors"
	"image"
	"image/color"
	"image/draw"
	"sync"
	"sync/atomic"
	"time"

	"github.com/aarzilli/nucular/internal/windowing"
	"github.com/hajimehoshi/ebiten/v2"
	"golang.org/x/exp/shiny/screen"
	"golang.org/x/image/math/f64"
	"golang.org/x/mobile/event/key"
	"golang.org/x/mobile/event/lifecycle"
	"golang.org/x/mobile/event/mouse"
	"golang.org/x/mobile/event/paint"
	"golang.org/x/mobile/event/size"
)

type display struct{ ready chan *window }
type buffer struct{ pixels *image.RGBA }

func (b *buffer) Release()                {}
func (b *buffer) Size() image.Point       { return b.pixels.Rect.Size() }
func (b *buffer) Bounds() image.Rectangle { return b.pixels.Rect }
func (b *buffer) RGBA() *image.RGBA       { return b.pixels }
func (d *display) NewBuffer(sz image.Point) (screen.Buffer, error) {
	if sz.X <= 0 || sz.Y <= 0 {
		return nil, errors.New("invalid buffer size")
	}
	return &buffer{image.NewRGBA(image.Rectangle{Max: sz})}, nil
}
func (d *display) NewTexture(sz image.Point) (screen.Texture, error) {
	return nil, errors.New("nucular uses software buffers, not screen textures")
}
func (d *display) NewWindow(o *screen.NewWindowOptions) (screen.Window, error) {
	w := &window{events: windowing.NewEvents(), initial: image.Pt(o.Width, o.Height), title: o.Title}
	d.ready <- w
	return w, nil
}

type window struct {
	events           *windowing.Events
	initial          image.Point
	title            string
	closed           atomic.Bool
	mu               sync.Mutex
	surface          windowing.Surface
	gpu              *ebiten.Image
	width, height    int
	cursorX, cursorY int
	buttons          [3]bool
	keys             [256]bool
	repeat           [256]time.Time
	keyHeld          atomic.Bool
	chars            []rune
	focused          bool
}

func Main(f func(screen.Screen)) {
	d := &display{ready: make(chan *window, 1)}
	go f(d)
	w := <-d.ready
	ebiten.SetWindowTitle(w.title)
	ebiten.SetWindowSize(w.initial.X, w.initial.Y)
	ebiten.SetWindowResizingMode(ebiten.WindowResizingModeEnabled)
	ebiten.SetWindowClosingHandled(true)
	ebiten.SetScreenClearedEveryFrame(false)
	ebiten.SetFPSMode(ebiten.FPSModeVsyncOffMinimum)
	// A single coalescing wakeup also services nucular's asynchronous updater.
	stop := make(chan struct{})
	defer close(stop)
	go func() {
		t := time.NewTicker(time.Second / 30)
		defer t.Stop()
		for {
			select {
			case <-t.C:
				if w.closed.Load() {
					return
				}
				w.mu.Lock()
				dirty := !w.surface.Damage.Empty()
				w.mu.Unlock()
				if dirty || w.keyHeld.Load() {
					ebiten.ScheduleFrame()
				}
			case <-stop:
				return
			}
		}
	}()
	if err := ebiten.RunGame(&game{w}); err != nil {
		panic(err)
	}
}
func (w *window) Send(e any) {
	w.events.Send(e)
}
func (w *window) SendFirst(e any) { w.Send(e) }
func (w *window) NextEvent() any  { return w.events.Next() }
func (w *window) Release()        { w.closed.Store(true); ebiten.ScheduleFrame() }
func (w *window) Upload(dp image.Point, src screen.Buffer, sr image.Rectangle) {
	w.mu.Lock()
	defer w.mu.Unlock()
	sz := image.Pt(max(w.width, dp.X+sr.Dx()), max(w.height, dp.Y+sr.Dy()))
	w.surface.Ensure(sz)
	w.surface.Upload(dp, src.RGBA(), sr)
}
func (w *window) Publish() screen.PublishResult {
	ebiten.ScheduleFrame()
	return screen.PublishResult{}
}
func (w *window) Fill(r image.Rectangle, c color.Color, op draw.Op) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.surface.Pixels != nil {
		r = r.Intersect(w.surface.Pixels.Bounds())
		draw.Draw(w.surface.Pixels, r, &image.Uniform{C: c}, image.Point{}, op)
		w.surface.Damage = w.surface.Damage.Union(r)
	}
}
func (w *window) Draw(f64.Aff3, screen.Texture, image.Rectangle, draw.Op, *screen.DrawOptions) {
	panic("ebitenscreen: texture drawing is unsupported; use Upload")
}
func (w *window) DrawUniform(f64.Aff3, color.Color, image.Rectangle, draw.Op, *screen.DrawOptions) {
	panic("ebitenscreen: transformed fills are unsupported; use Fill")
}
func (w *window) Copy(image.Point, screen.Texture, image.Rectangle, draw.Op, *screen.DrawOptions) {
	panic("ebitenscreen: texture copying is unsupported; use Upload")
}
func (w *window) Scale(image.Rectangle, screen.Texture, image.Rectangle, draw.Op, *screen.DrawOptions) {
	panic("ebitenscreen: texture scaling is unsupported; use Upload")
}
func (w *window) Layout(width, height int) (int, int) {
	w.mu.Lock()
	changed := width != w.width || height != w.height
	if changed {
		w.width, w.height = width, height
	}
	w.mu.Unlock()
	if changed {
		w.Send(size.Event{WidthPx: width, HeightPx: height, PixelsPerPt: 1})
		w.Send(paint.Event{})
	}
	return max(width, 1), max(height, 1)
}

var keyMap = []struct {
	e ebiten.Key
	k key.Code
}{
	{ebiten.KeyBackspace, key.CodeDeleteBackspace}, {ebiten.KeyDelete, key.CodeDeleteForward}, {ebiten.KeyEnter, key.CodeReturnEnter}, {ebiten.KeyTab, key.CodeTab}, {ebiten.KeyEscape, key.CodeEscape},
	{ebiten.KeyArrowLeft, key.CodeLeftArrow}, {ebiten.KeyArrowRight, key.CodeRightArrow}, {ebiten.KeyArrowUp, key.CodeUpArrow}, {ebiten.KeyArrowDown, key.CodeDownArrow},
	{ebiten.KeyHome, key.CodeHome}, {ebiten.KeyEnd, key.CodeEnd}, {ebiten.KeyPageUp, key.CodePageUp}, {ebiten.KeyPageDown, key.CodePageDown},
	{ebiten.KeyComma, key.CodeComma}, {ebiten.KeyPeriod, key.CodeFullStop}, {ebiten.KeySpace, key.CodeSpacebar}, {ebiten.KeyInsert, key.CodeInsert}, {ebiten.KeyNumpadEnter, key.CodeKeypadEnter},
}

func init() {
	for i := 0; i < 26; i++ {
		keyMap = append(keyMap, struct {
			e ebiten.Key
			k key.Code
		}{ebiten.KeyA + ebiten.Key(i), key.CodeA + key.Code(i)})
	}
	for i := 0; i < 9; i++ {
		keyMap = append(keyMap, struct {
			e ebiten.Key
			k key.Code
		}{ebiten.KeyDigit1 + ebiten.Key(i), key.Code1 + key.Code(i)})
	}
	keyMap = append(keyMap, struct {
		e ebiten.Key
		k key.Code
	}{ebiten.KeyDigit0, key.Code0})
	for i := 0; i < 12; i++ {
		keyMap = append(keyMap, struct {
			e ebiten.Key
			k key.Code
		}{ebiten.KeyF1 + ebiten.Key(i), key.CodeF1 + key.Code(i)})
	}

}

func (w *window) Update() error {
	if w.closed.Load() {
		return ebiten.Termination
	}
	if ebiten.IsWindowBeingClosed() {
		w.Send(lifecycle.Event{To: lifecycle.StageDead})
		return nil
	}
	focused := ebiten.IsFocused()
	if focused != w.focused {
		from := lifecycle.StageVisible
		if w.focused {
			from = lifecycle.StageFocused
		}
		to := lifecycle.StageVisible
		if focused {
			to = lifecycle.StageFocused
		}
		w.Send(lifecycle.Event{From: from, To: to})
		w.focused = focused
	}
	x, y := ebiten.CursorPosition()
	if x != w.cursorX || y != w.cursorY {
		w.Send(mouse.Event{X: float32(x), Y: float32(y)})
		w.cursorX, w.cursorY = x, y
	}
	for i, b := range []ebiten.MouseButton{ebiten.MouseButtonLeft, ebiten.MouseButtonMiddle, ebiten.MouseButtonRight} {
		down := ebiten.IsMouseButtonPressed(b)
		if down != w.buttons[i] {
			dir := mouse.DirRelease
			if down {
				dir = mouse.DirPress
			}
			w.Send(mouse.Event{X: float32(x), Y: float32(y), Button: mouse.Button(i + 1), Direction: dir})
			w.buttons[i] = down
		}
	}
	dx, dy := ebiten.Wheel()
	if dx != 0 || dy != 0 {
		w.Send(windowing.Wheel{X: float32(x), Y: float32(y), DeltaX: float32(dx), DeltaY: float32(dy)})
	}
	var mods key.Modifiers
	if ebiten.IsKeyPressed(ebiten.KeyShift) {
		mods |= key.ModShift
	}
	if ebiten.IsKeyPressed(ebiten.KeyControl) {
		mods |= key.ModControl
	}
	if ebiten.IsKeyPressed(ebiten.KeyAlt) {
		mods |= key.ModAlt
	}
	if ebiten.IsKeyPressed(ebiten.KeyMeta) {
		mods |= key.ModMeta
	}
	now := time.Now()
	held := false
	for i, m := range keyMap {
		down := ebiten.IsKeyPressed(m.e)
		held = held || down
		if down != w.keys[i] {
			dir := key.DirRelease
			if down {
				dir = key.DirPress
			}
			w.Send(key.Event{Code: m.k, Modifiers: mods, Direction: dir})
			w.keys[i] = down
			w.repeat[i] = now.Add(400 * time.Millisecond)
		} else if down && now.After(w.repeat[i]) {
			w.Send(key.Event{Code: m.k, Modifiers: mods, Direction: key.DirNone})
			w.repeat[i] = now.Add(35 * time.Millisecond)
		}
	}
	w.keyHeld.Store(held)
	w.chars = ebiten.AppendInputChars(w.chars[:0])
	for _, r := range w.chars {
		if r >= 32 && mods&key.ModMeta == 0 && (mods&key.ModControl == 0 || mods&key.ModAlt != 0) {
			w.Send(key.Event{Rune: r, Code: key.CodeUnknown, Modifiers: mods & key.ModShift, Direction: key.DirPress})
		}
	}
	return nil
}

type game struct{ *window }

func (g *game) Draw(screenImage *ebiten.Image) {
	w := g.window
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.surface.Pixels == nil {
		return
	}
	sz := w.surface.Pixels.Rect.Size()
	if w.gpu == nil || w.gpu.Bounds().Size() != sz {
		if w.gpu != nil {
			w.gpu.Deallocate()
		}
		w.gpu = ebiten.NewImage(sz.X, sz.Y)
		w.surface.Damage = w.surface.Pixels.Bounds()
	}
	if !w.surface.Damage.Empty() {
		r, pixels := w.surface.PackedDamage()
		w.gpu.SubImage(r).(*ebiten.Image).WritePixels(pixels)
		w.surface.Damage = image.Rectangle{}
	}
	visible := image.Rect(0, 0, w.width, w.height).Intersect(w.gpu.Bounds())
	if !visible.Empty() {
		screenImage.DrawImage(w.gpu.SubImage(visible).(*ebiten.Image), nil)
	}
}
