package ui

import (
	"context"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
)

const rallyPageSize = 128
const rallyWindowItems = 2048

// Stream only lightweight card fields. Full HTML and collections are loaded
// when an artifact is opened. The window advances only on viewport demand.
const cardFields = "ObjectID,FormattedID,Name,ScheduleState,State,Owner,Iteration,Release,Project,Feature,Parent,PlanEstimate,Estimate,ToDo,Actuals,Blocked,Ready,Tasks,LastUpdateDate,Rank"

func (a *App) requestRallyPage(v *rallyView, start int, replace bool) {
	if v.Closed || v.Loading || a.rallyClient == nil {
		return
	}
	c := a.rallyClient
	q := a.rallyQuery(v)
	q.Start = max(1, start)
	q.PageSize = rallyPageSize
	q.Fetch = cardFields
	appliedSearch := text(v.Search)
	generation := v.Generation
	v.Loading = true
	v.Evicted = false
	ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
	v.cancel = cancel
	a.work(func() {
		defer cancel()
		page, err := c.CachedQuery(ctx, v.Spec.Kind, q, replace)
		// Expensive text/card projection stays on the worker. The UI swaps an
		// immutable batch at a frame boundary.
		projection := make([]string, len(page.Results))
		for i, o := range page.Results {
			projection[i] = searchable(o)
		}
		a.post(func() {
			if v.Closed || v.Generation != generation {
				return
			}
			v.Loading = false
			v.cancel = nil
			if err != nil {
				v.Error = err.Error()
				v.Failures++
				v.RetryAfter = time.Now().Add(time.Duration(5*(1<<min(v.Failures-1, 6))) * time.Second)
				return
			}
			v.Error = ""
			v.Failures = 0
			v.RetryAfter = time.Time{}
			v.Total = page.Total
			v.AppliedSearch = appliedSearch
			prepend := !replace && q.Start < v.Start
			if replace {
				v.Items = append([]rally.Object(nil), page.Results...)
				v.Start = q.Start
				v.filterSearch = projection
				v.Next = q.Start + len(page.Results)
				v.More = v.Next <= v.Total && len(page.Results) > 0
			} else if prepend {
				v.Items = append(append(make([]rally.Object, 0, len(page.Results)+len(v.Items)), page.Results...), v.Items...)
				v.filterSearch = append(projection, v.filterSearch...)
				v.Start = q.Start
				if v.scrollAdjustment == nil {
					v.scrollAdjustment = map[string]int{}
				}
				for _, o := range page.Results {
					v.scrollAdjustment[o.String(rally.StateField(v.Spec.Kind))] -= 206
				}
			} else {
				v.Items = append(v.Items, page.Results...)
				v.filterSearch = append(v.filterSearch, projection...)
				v.Next = q.Start + len(page.Results)
				v.More = v.Next <= v.Total && len(page.Results) > 0
			}
			if len(v.Items) > rallyWindowItems {
				drop := len(v.Items) - rallyWindowItems
				if prepend {
					v.Next -= drop
					v.More = true
					clear(v.Items[rallyWindowItems:])
					v.Items = v.Items[:rallyWindowItems]
					v.filterSearch = v.filterSearch[:rallyWindowItems]
				} else {
					if v.scrollAdjustment == nil {
						v.scrollAdjustment = map[string]int{}
					}
					for _, o := range v.Items[:drop] {
						v.scrollAdjustment[o.String(rally.StateField(v.Spec.Kind))] += 206
					}
					clear(v.Items[:drop])
					v.Items = append([]rally.Object(nil), v.Items[drop:]...)
					v.filterSearch = append([]string(nil), v.filterSearch[drop:]...)
					v.Start += drop
				}
			}
			v.Refreshed = time.Now()

			v.filterSource = nil
			v.cardSource = nil
			v.boardPrepared = false
		})
	})
}
func (a *App) needRallyPage(v *rallyView) {
	if v.More && !v.Loading && time.Now().After(v.RetryAfter) {
		a.requestRallyPage(v, v.Next, false)
	}
}
func (a *App) previousRallyPage(v *rallyView) {
	if v.Start > 1 && !v.Loading && time.Now().After(v.RetryAfter) {
		a.requestRallyPage(v, max(1, v.Start-rallyPageSize), false)
	}
}
