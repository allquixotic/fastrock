package ui

import (
	"context"
	"fmt"
	"github.com/allquixotic/fastrock/internal/rally"
)

// Metadata uses projections and the same cache/admission budget as cards.
// A bounded picker never silently presents a truncated catalog as complete.
func scopeObjects(ctx context.Context, c *rally.Client, kind string, q rally.Query) ([]rally.Object, error) {
	q.Fetch = "ObjectID,Name"
	if kind == "User" {
		q.Fetch += ",DisplayName,UserName"
	}
	if kind == "Iteration" {
		q.Fetch += ",StartDate,EndDate,Project"
	}
	if kind == "Release" {
		q.Fetch += ",ReleaseStartDate,ReleaseDate,Project"
	}
	q.Order = "Name"
	q.PageSize = 200
	var rows []rally.Object
	for q.Start = 1; q.Start <= 2000; {
		page, err := c.CachedQuery(ctx, kind, q, false)
		if err != nil {
			return nil, err
		}
		rows = append(rows, page.Results...)
		q.Start += len(page.Results)
		if q.Start > page.Total || len(page.Results) == 0 {
			return rows, nil
		}
	}
	return nil, fmt.Errorf("more than 2,000 matching choices; narrow the selected workspace or project")
}
