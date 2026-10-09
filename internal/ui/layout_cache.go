package ui

import "github.com/allquixotic/fastrock/internal/workspace"

const transcriptLayoutBudget = 8 << 20

func (l *transcriptLayout) bytes() int {
	n := len(l.Text) + len(l.Plain) + len(l.Cwd) + len(l.Markup) + len(l.Lines)*80 + len(l.LineOffsets)*8
	for _, line := range l.Lines {
		n += len(line.Runs) * 128
		for _, r := range line.Runs {
			n += len(r.Text) + (len(r.Advances)+len(r.Offsets))*8
		}
	}
	return n
}
func (v *chatView) boundLayouts(keep string) {
	for {
		bytes := 0
		oldest := ""
		used := ^uint64(0)
		for id, l := range v.Layouts {
			bytes += l.bytes()
			if id != keep && l.Used < used {
				oldest, used = id, l.Used
			}
		}
		if len(v.Layouts) <= 128 && bytes <= transcriptLayoutBudget {
			return
		}
		if oldest == "" {
			delete(v.Layouts, keep)
			return
		}
		delete(v.Layouts, oldest)
	}
}
func (v *chatView) pruneLayouts(blocks []workspace.Block) {
	first := ""
	if len(blocks) > 0 {
		first = blocks[0].ID
	}
	if len(blocks) == v.LayoutBlockCount && first == v.LayoutFirstID {
		return
	}
	v.LayoutBlockCount, v.LayoutFirstID = len(blocks), first
	live := make(map[string]bool, len(blocks))
	for _, b := range blocks {
		live[b.ID] = true
	}
	for id := range v.Layouts {
		if !live[id] {
			delete(v.Layouts, id)
		}
	}
	for id := range v.Expanded {
		if !live[id] {
			delete(v.Expanded, id)
		}
	}
	if !live[v.RichSelection.BlockID] {
		v.RichSelection = transcriptSelection{}
	}
	if !live[v.SelectID] {
		v.SelectID = ""
		v.Selection = nil
	}
}
