package richtext

import (
	"container/list"
	"slices"
	"sync"
	"time"
	"unsafe"
	"weak"
)

type editDelta struct {
	start, removed, inserted int
	text                     bool
	beforeText, afterText    []rune
	beforeSpans, afterSpans  []Span
}

type undoGroup struct {
	owner   weak.Pointer[Document]
	edits   []editDelta
	bytes   int
	element *list.Element
}

// All editors in this process share one history budget. Weak owners allow
// closed documents to be collected; entries cannot pin an entire editor.
type historyPool struct {
	mu           sync.Mutex
	limit, bytes int
	groups       list.List
}

var sharedHistory = historyPool{limit: 8 << 20}

// SetHistoryBudget changes the aggregate limit for every editor in this
// process. The application divides its total allowance among live windows.
// Trimming releases only history, never current document contents.
func SetHistoryBudget(bytes int) {
	sharedHistory.mu.Lock()
	defer sharedHistory.mu.Unlock()
	sharedHistory.limit = max(0, bytes)
	sharedHistory.trim()
}

func (d *Document) historyPool() *historyPool {
	if d.history != nil {
		return d.history
	}
	return &sharedHistory
}

func deltaBytes(e editDelta) int {
	n := (cap(e.beforeText)+cap(e.afterText))*int(unsafe.Sizeof(rune(0))) +
		(cap(e.beforeSpans)+cap(e.afterSpans))*int(unsafe.Sizeof(Span{}))
	for _, spans := range [][]Span{e.beforeSpans, e.afterSpans} {
		for _, s := range spans {
			n += len(s.Format.Link)
		}
	}
	return n
}

func (p *historyPool) remove(g *undoGroup) {
	if g.element == nil {
		return
	}
	p.groups.Remove(g.element)
	p.bytes -= g.bytes
	g.element = nil
	clear(g.edits)
	g.edits = nil
}

func (p *historyPool) trim() {
	for p.bytes > p.limit && p.groups.Len() > 0 {
		g := p.groups.Front().Value.(*undoGroup)
		d := g.owner.Value()
		if d == nil {
			p.remove(g)
			continue
		}
		// Keep each remaining undo/redo chain contiguous. The earliest undo
		// or furthest redo can be removed without skipping an intermediate state.
		if len(d.undo) > 0 {
			p.remove(d.undo[0])
			d.undo = slices.Delete(d.undo, 0, 1)
		} else if len(d.redo) > 0 {
			p.remove(d.redo[0])
			d.redo = slices.Delete(d.redo, 0, 1)
		} else {
			p.remove(g)
		}
	}
}

func (d *Document) record(delta editDelta, coalesce bool) {
	p := d.historyPool()
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, g := range d.redo {
		p.remove(g)
	}
	d.redo = nil
	now := time.Now()
	var g *undoGroup
	if coalesce && !d.lastEdit.IsZero() && now.Sub(d.lastEdit) <= 650*time.Millisecond && len(d.undo) > 0 {
		g = d.undo[len(d.undo)-1]
	}
	if g == nil {
		g = &undoGroup{owner: weak.Make(d)}
		g.element = p.groups.PushBack(g)
		d.undo = append(d.undo, g)
	}
	d.lastEdit = time.Time{}
	if coalesce {
		d.lastEdit = now
	}
	oldCapacity := cap(g.edits)
	g.edits = append(g.edits, delta)
	added := deltaBytes(delta) + (cap(g.edits)-oldCapacity)*int(unsafe.Sizeof(editDelta{}))
	if g.bytes == 0 {
		added += int(unsafe.Sizeof(*g)) + int(unsafe.Sizeof(list.Element{})) + 16
	}
	g.bytes += added
	p.bytes += added
	if len(d.undo) > 100 {
		p.remove(d.undo[0])
		d.undo = slices.Delete(d.undo, 0, 1)
	}
	p.trim()
}

func (d *Document) applyDelta(e editDelta, reverse bool) {
	removed, inserted := e.removed, e.inserted
	text, spans := e.afterText, e.afterSpans
	if reverse {
		removed, inserted = inserted, removed
		text, spans = e.beforeText, e.beforeSpans
	}
	d.replaceSpans(e.start, removed, inserted, spans)
	if e.text {
		d.Text = slices.Replace(d.Text, e.start, e.start+removed, text...)
	}
}

func (d *Document) Undo(redo bool) bool {
	p := d.historyPool()
	p.mu.Lock()
	defer p.mu.Unlock()
	d.lastEdit = time.Time{}
	from, to := &d.undo, &d.redo
	if redo {
		from, to = to, from
	}
	if len(*from) == 0 {
		return false
	}
	g := (*from)[len(*from)-1]
	(*from)[len(*from)-1] = nil
	*from = (*from)[:len(*from)-1]
	if redo {
		for _, e := range g.edits {
			d.applyDelta(e, false)
		}
	} else {
		for i := len(g.edits) - 1; i >= 0; i-- {
			d.applyDelta(g.edits[i], true)
		}
	}
	*to = append(*to, g)
	d.pending = nil
	d.invalidate()
	return true
}

// DiscardHistory releases only undo data; current unsaved content stays intact.
func (d *Document) DiscardHistory() {
	p := d.historyPool()
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, g := range d.undo {
		p.remove(g)
	}
	for _, g := range d.redo {
		p.remove(g)
	}
	d.undo, d.redo = nil, nil
	d.lastEdit = time.Time{}
}
