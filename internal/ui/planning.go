package ui

import "github.com/allquixotic/fastrock/internal/rally"

type iterationTotal struct {
	points float64
	count  int
}

type planningSummary struct {
	points            float64
	accepted, blocked int
	iterations        map[string]iterationTotal
}

// The caller supplies filtered()'s current result. Its revision advances on
// data, filter and sort changes; the summary retains scalars, never item maps.
func (v *rallyView) preparePlanning(items []rally.Object) planningSummary {
	if v.planningPrepared && v.planningRevision == v.filterRevision {
		return v.planningSummary
	}
	summary := planningSummary{iterations: make(map[string]iterationTotal)}
	stateField := rally.StateField(v.Spec.Kind)
	for _, o := range items {
		points := o.Number("PlanEstimate")
		summary.points += points
		if o.String(stateField) == "Accepted" {
			summary.accepted++
		}
		if o.Bool("Blocked") {
			summary.blocked++
		}
		ref := o.Ref("Iteration")
		total := summary.iterations[ref]
		total.points += points
		total.count++
		summary.iterations[ref] = total
	}
	v.planningPrepared, v.planningRevision, v.planningSummary = true, v.filterRevision, summary
	return summary
}
