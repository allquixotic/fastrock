package ui

import (
	"testing"

	"github.com/allquixotic/fastrock/internal/rally"
)

func TestV22PlanningTotalsFollowFilteredRevision(t *testing.T) {
	v := newRallyView(rally.FindPage("teamplan"))
	v.Spec.Kind = "HierarchicalRequirement"
	v.Items = []rally.Object{
		{"_ref": "/story/1", "Name": "One", "Iteration": map[string]any{"_ref": "/iteration/1"}, "PlanEstimate": float64(3), "ScheduleState": "Accepted"},
		{"_ref": "/story/2", "Name": "Two", "Iteration": map[string]any{"_ref": "/iteration/1"}, "PlanEstimate": float64(5), "Blocked": true},
		{"_ref": "/story/3", "Name": "Three", "Iteration": map[string]any{"_ref": "/iteration/2"}, "PlanEstimate": float64(2)},
	}
	items := v.filtered()
	totals := v.preparePlanning(items)
	if totals.points != 10 || totals.accepted != 1 || totals.blocked != 1 || totals.iterations["/iteration/1"] != (iterationTotal{points: 8, count: 2}) {
		t.Fatalf("bad totals: %+v", totals)
	}
	if n := testing.AllocsPerRun(100, func() { v.preparePlanning(v.filtered()) }); n != 0 {
		t.Fatalf("warm totals allocate %g", n)
	}
	v.OnlyBlocked = true
	totals = v.preparePlanning(v.filtered())
	if totals.points != 5 || totals.accepted != 0 || totals.blocked != 1 || len(totals.iterations) != 1 {
		t.Fatalf("stale filtered totals: %+v", totals)
	}
	v.Items[1]["PlanEstimate"] = float64(8)
	v.Generation++
	totals = v.preparePlanning(v.filtered())
	if totals.points != 8 {
		t.Fatal("stale refreshed totals", totals)
	}
	v.Items = nil
	totals = v.preparePlanning(v.filtered())
	if totals.points != 0 || len(totals.iterations) != 0 {
		t.Fatal("stale empty totals", totals)
	}
}
