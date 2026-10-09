package windowing

import (
	"sync"

	"golang.org/x/mobile/event/mouse"
	"golang.org/x/mobile/event/paint"
)

// Wheel preserves native fractional touchpad deltas without rounding each
// sample into a full wheel step.
type Wheel struct{ X, Y, DeltaX, DeltaY float32 }

// Events coalesces only adjacent replaceable events. Button/key/lifecycle
// transitions remain ordered barriers, and bounded storage backpressures their
// producers instead of dropping them.
type Events struct {
	mu           sync.Mutex
	ready, space *sync.Cond
	queue        [256]any
	head, count  int
}

func NewEvents() *Events {
	e := &Events{}
	e.ready, e.space = sync.NewCond(&e.mu), sync.NewCond(&e.mu)
	return e
}

func (e *Events) Send(v any) {
	e.mu.Lock()
	defer e.mu.Unlock()
	for {
		if e.count > 0 {
			last := (e.head + e.count - 1) % len(e.queue)
			if merged, ok := coalesce(e.queue[last], v); ok {
				e.queue[last] = merged
				return
			}
		}
		if e.count < len(e.queue) {
			break
		}
		e.space.Wait()
	}
	e.queue[(e.head+e.count)%len(e.queue)] = v
	e.count++
	e.ready.Signal()
}

func (e *Events) Next() any {
	e.mu.Lock()
	defer e.mu.Unlock()
	for e.count == 0 {
		e.ready.Wait()
	}
	v := e.queue[e.head]
	e.queue[e.head] = nil
	e.head = (e.head + 1) % len(e.queue)
	e.count--
	e.space.Signal()
	return v
}

func coalesce(a, b any) (any, bool) {
	switch next := b.(type) {
	case mouse.Event:
		old, ok := a.(mouse.Event)
		return next, ok && old.Direction == mouse.DirNone && next.Direction == mouse.DirNone && old.Button == mouse.ButtonNone && next.Button == mouse.ButtonNone
	case paint.Event:
		_, ok := a.(paint.Event)
		return next, ok
	case Wheel:
		old, ok := a.(Wheel)
		next.DeltaX += old.DeltaX
		next.DeltaY += old.DeltaY
		return next, ok
	}
	return nil, false
}
