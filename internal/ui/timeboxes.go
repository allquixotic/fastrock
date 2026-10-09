package ui

import (
	"fmt"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/rally"
)

type timeboxChoice struct{ Name, Ref, Label string }

type timeboxChoices struct {
	rows                   []rally.Object // immutable metadata snapshot
	project                string
	choices                []timeboxChoice
	byRef                  map[string]int
	byName                 map[string]int
	labels, relativeLabels []string
	missing                map[string][]string
	nextChange, retryAt    time.Time
	updating               bool
}

// Equal names deliberately share a choice across child projects: the work-item
// query uses Name, while a concrete reference is retained for create defaults.
func makeTimeboxChoices(rows []rally.Object, project string, now time.Time) *timeboxChoices {
	p := &timeboxChoices{rows: rows, project: project, byRef: map[string]int{}, byName: map[string]int{}, labels: []string{"All"}, relativeLabels: []string{"All", "Current iteration"}, missing: map[string][]string{}}
	groups := map[string][]rally.Object{}
	for _, row := range rows {
		if row.String("Name") == "" || row.String("_ref") == "" {
			continue
		}
		groups[row.String("Name")] = append(groups[row.String("Name")], row)
		for _, field := range []string{"StartDate", "EndDate", "ReleaseStartDate", "ReleaseDate"} {
			if date, err := time.Parse(time.RFC3339, row.String(field)); err == nil && date.After(now) && (p.nextChange.IsZero() || date.Before(p.nextChange)) {
				p.nextChange = date
			}
		}
	}
	type group struct {
		name  string
		row   rally.Object
		start time.Time
	}
	ordered := make([]group, 0, len(groups))
	for name, rows := range groups {
		sort.Slice(rows, func(i, j int) bool {
			ie, je := project != "" && rows[i].Ref("Project") == project, project != "" && rows[j].Ref("Project") == project
			if ie != je {
				return ie
			}
			is, _ := timeboxDates(rows[i])
			js, _ := timeboxDates(rows[j])
			if !is.Equal(js) {
				return is.After(js)
			}
			return rows[i].String("_ref") < rows[j].String("_ref")
		})
		start, _ := timeboxDates(rows[0])
		ordered = append(ordered, group{name, rows[0], start})
	}
	sort.Slice(ordered, func(i, j int) bool {
		if !ordered[i].start.Equal(ordered[j].start) {
			return ordered[i].start.After(ordered[j].start)
		}
		return ordered[i].name < ordered[j].name
	})
	for _, g := range ordered {
		start, end := timeboxDates(g.row)
		label := g.name
		if !start.IsZero() && !end.IsZero() {
			label += fmt.Sprintf(" · %s – %s", start.Format("2006-01-02"), end.Format("2006-01-02"))
		}
		for _, row := range groups[g.name] {
			s, e := timeboxDates(row)
			if !s.IsZero() && !e.IsZero() && !now.Before(s) && now.Before(e) {
				label += " · Current"
				break
			}
		}
		for _, row := range groups[g.name] {
			p.byRef[row.String("_ref")] = len(p.choices)
		}
		p.byName[g.name] = len(p.choices)
		p.choices = append(p.choices, timeboxChoice{g.name, g.row.String("_ref"), label})
		p.labels = append(p.labels, label)
		p.relativeLabels = append(p.relativeLabels, label)
	}
	if p.choices == nil {
		p.choices = []timeboxChoice{}
	}
	return p
}

func timeboxDates(row rally.Object) (time.Time, time.Time) {
	start, _ := time.Parse(time.RFC3339, fallback(row.String("StartDate"), row.String("ReleaseStartDate")))
	end, _ := time.Parse(time.RFC3339, fallback(row.String("EndDate"), row.String("ReleaseDate")))
	return start, end
}

func (a *App) getTimeboxChoices(kind string) *timeboxChoices {
	if a.timeboxChoices == nil {
		a.timeboxChoices = map[string]*timeboxChoices{}
	}
	p := a.timeboxChoices[kind]
	if p == nil {
		rows := a.iterations
		if kind == "Release" {
			rows = a.releases
		}
		p = &timeboxChoices{rows: rows, project: a.prefs.RallyProject, byRef: map[string]int{}, byName: map[string]int{}, labels: []string{"All"}, relativeLabels: []string{"All", "Current iteration"}, missing: map[string][]string{}}
		a.timeboxChoices[kind] = p
	}
	now := time.Now()
	if !p.updating && !now.Before(p.retryAt) && (p.choices == nil || !p.nextChange.IsZero() && !now.Before(p.nextChange)) {
		p.updating = true
		rows, project := p.rows, p.project
		a.work(func() {
			next := makeTimeboxChoices(rows, project, now)
			a.post(func() {
				p.updating = false
				if a.timeboxChoices[kind] == p {
					a.timeboxChoices[kind] = next
				}
			})
		}, func() { p.updating = false; p.retryAt = now.Add(5 * time.Second) })
	}
	return p
}

func lookupTimeboxName(rows []rally.Object, ref string) string {
	for _, row := range rows {
		if row.String("_ref") == ref {
			return row.String("Name")
		}
	}
	return ""
}

func legacyRelease(ref string) bool { return strings.Contains(strings.ToLower(ref), "/release/") }

func (v *rallyView) migrateTimeboxes() {
	if legacyRelease(v.Timebox) {
		if v.ReleaseTimebox == "" {
			v.ReleaseTimebox, v.ReleaseName = v.Timebox, v.TimeboxName
		}
		v.Timebox, v.TimeboxName = "", ""
	}
}

func (a *App) prepareTimeboxNames(v *rallyView) {
	v.migrateTimeboxes()
	if v.Timebox != "" && v.TimeboxName == "" {
		v.TimeboxName = lookupTimeboxName(a.iterations, v.Timebox)
	}
	if v.ReleaseTimebox != "" && v.ReleaseName == "" {
		v.ReleaseName = lookupTimeboxName(a.releases, v.ReleaseTimebox)
	}
}

func timeboxPredicate(kind, ref, name string, rows []rally.Object) string {
	if ref == "" && name == "" {
		return ""
	}
	if name == "" {
		name = lookupTimeboxName(rows, ref)
	}
	if name != "" {
		return rally.Eq(kind+".Name", name)
	}
	// Unknown legacy selections stay restrictive until their name is available.
	return rally.Eq(kind, ref)
}

func rallyTimeboxesSupported(v *rallyView) bool {
	return v.Spec.Mode == "planning" || v.Spec.ID == "teamstatus" || v.Spec.Mode == "board" && !strings.HasPrefix(v.Spec.Kind, "PortfolioItem/")
}

func (p *timeboxChoices) options(ref, name string, relative, current bool) ([]string, int, int) {
	labels, offset := p.labels, 1
	if relative {
		labels, offset = p.relativeLabels, 2
	}
	if current && relative {
		return labels, 1, offset
	}
	if ref == "" && name == "" {
		return labels, 0, offset
	}
	i, ok := p.byName[name]
	if name == "" {
		i, ok = p.byRef[ref]
	}
	if ok {
		return labels, i + offset, offset
	}
	key := fmt.Sprintf("%t\x00%s\x00%s", relative, ref, name)
	missing := p.missing[key]
	if missing == nil {
		if len(p.missing) >= 16 {
			clear(p.missing)
		}
		missing = append(slices.Clone(labels), "Selected: "+fallback(name, ref))
		p.missing[key] = missing
	}
	return missing, len(missing) - 1, offset
}

func (a *App) drawTimeboxSelector(w *desktop.Window, v *rallyView, kind string) {
	p := a.getTimeboxChoices(kind)
	ref, name := v.Timebox, v.TimeboxName
	if kind == "Release" {
		ref, name = v.ReleaseTimebox, v.ReleaseName
	}
	relative := kind == "Iteration" && v.Spec.ID == "iterationstatus"
	labels, selected, offset := p.options(ref, name, relative, kind == "Iteration" && v.CurrentIteration)
	next := w.ComboSimple(labels, selected, 28)
	if next == selected {
		return
	}
	ref, name = "", ""
	if next >= offset && next-offset < len(p.choices) {
		ref, name = p.choices[next-offset].Ref, p.choices[next-offset].Name
	}
	if kind == "Iteration" {
		v.Timebox, v.TimeboxName, v.CurrentIteration = ref, name, v.Spec.ID == "iterationstatus" && next == 1
	} else {
		v.ReleaseTimebox, v.ReleaseName = ref, name
	}
	a.refreshRally(v)
}

func (a *App) drawTimeboxSelectors(w *desktop.Window, v *rallyView) {
	if !rallyTimeboxesSupported(v) {
		return
	}
	scale := w.Master().Style().Scaling
	if w.LayoutAvailableWidth() >= int(700*scale) {
		w.Row(28).StaticScaled(int(82*scale), 0, int(158*scale), 0)
		w.LabelColored("Iteration", "LC", a.p.Muted)
		a.drawTimeboxSelector(w, v, "Iteration")
		w.LabelColored("FY Quarter / Release", "LC", a.p.Muted)
		a.drawTimeboxSelector(w, v, "Release")
		return
	}
	w.Row(22).Dynamic(2)
	w.LabelColored("Iteration", "LC", a.p.Muted)
	w.LabelColored("FY Quarter / Release", "LC", a.p.Muted)
	w.Row(30).Dynamic(2)
	a.drawTimeboxSelector(w, v, "Iteration")
	a.drawTimeboxSelector(w, v, "Release")
}

func timeboxDefaultRef(rows []rally.Object, project, ref, name string) string {
	if name == "" {
		return ref
	}
	for _, row := range rows {
		if row.String("Name") == name && (project == "" || row.Ref("Project") == project) {
			return row.String("_ref")
		}
	}
	return ""
}
