package rally

import (
	"context"
	"fmt"
	"strings"
	"time"
)

type schemaEntry struct {
	fields  []Field
	expires time.Time
}

func cloneFields(fields []Field) []Field {
	out := append([]Field(nil), fields...)
	for i := range out {
		out[i].AllowedValues = append([]string(nil), out[i].AllowedValues...)
	}
	return out
}

// Workflow returns valid field values; portfolio states retain their refs.
func (c *Client) Workflow(ctx context.Context, kind, workspace string, fields []Field) ([]Object, error) {
	if strings.HasPrefix(strings.ToLower(kind), "portfolioitem") {
		p, err := c.CachedQuery(ctx, "State", Query{Workspace: workspace, Expression: Eq("TypeDef.TypePath", kind), Order: "OrderIndex ASC", Fetch: "ObjectID,Name,OrderIndex,TypeDef", PageSize: 200}, false)
		if err != nil {
			return nil, err
		}
		if p.Total > len(p.Results) {
			return nil, fmt.Errorf("state catalog is incomplete")
		}
		return p.Results, nil
	}
	for _, f := range fields {
		if f.Name == StateField(kind) {
			states := make([]Object, 0, len(f.AllowedValues))
			for _, value := range f.AllowedValues {
				states = append(states, Object{"Name": value})
			}
			return states, nil
		}
	}
	return nil, nil
}
