package ui

import (
	"github.com/allquixotic/fastrock/internal/rally"
	"slices"
)

type pickerOption struct{ names, refs []string }

type pickerChoices struct {
	Names, Refs []string
	indices     map[string]int
	labels      map[string][]string
	missing     map[string]pickerOption
}

// Construct labels off the UI thread with each metadata load. The normal draw
// path then only looks up the selected reference and reuses immutable slices.
func makePickerChoices(rows []rally.Object) *pickerChoices {
	p := &pickerChoices{Names: []string{"None"}, Refs: []string{""}, indices: map[string]int{"": 0}, labels: map[string][]string{}, missing: map[string]pickerOption{}}
	for _, o := range rows {
		ref := o.String("_ref")
		if _, found := p.indices[ref]; found {
			continue
		}
		p.indices[ref] = len(p.Refs)
		p.Names = append(p.Names, fallback(o.String("DisplayName"), fallback(o.String("Name"), o.String("UserName"))))
		p.Refs = append(p.Refs, ref)
	}
	return p
}

func (p *pickerChoices) options(empty, selected string) (names, refs []string, index int) {
	names = p.labels[empty]
	if names == nil {
		names = slices.Clone(p.Names)
		names[0] = empty
		p.labels[empty] = names
	}
	if i, found := p.indices[selected]; found {
		return names, p.Refs, i
	}
	key := empty + "\x00" + selected
	extra := p.missing[key]
	if extra.names == nil {
		if len(p.missing) >= 16 {
			clear(p.missing)
		}
		extra = pickerOption{append(slices.Clone(names), "Selected: "+selected), append(slices.Clone(p.Refs), selected)}
		p.missing[key] = extra
	}
	// Only absent selections need a sentinel; no change is made until the user
	// actually chooses another option.
	return extra.names, extra.refs, len(p.Refs)
}

func (a *App) picker(key string, rows []rally.Object) *pickerChoices {
	if a.scopeChoices == nil {
		a.scopeChoices = map[string]*pickerChoices{}
	}
	if a.scopeChoices[key] == nil {
		a.scopeChoices[key] = makePickerChoices(rows)
	}
	return a.scopeChoices[key]
}
