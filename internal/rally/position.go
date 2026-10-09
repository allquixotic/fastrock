package rally

import (
	"context"
	"errors"
	"net/url"
	"sort"
	"strings"
)

// RankPosition uses WSAPI's relative ranking parameters. DragAndDropRank is an
// opaque server value; clients must not manufacture or update it directly.
type RankPosition struct {
	Ref   string
	Below bool
}

// UpdatePositionIfUnchanged changes fields and relative rank in one mutation.
// The preflight comparison detects intervening edits, as in other Rally saves;
// WSAPI does not offer an atomic compare-and-swap for this operation.
func (c *Client) UpdatePositionIfUnchanged(ctx context.Context, before Object, kind string, fields Object, position *RankPosition) (Object, error) {
	k, ok := CanonicalKind(kind)
	if !ok {
		return nil, errors.New("unsupported Rally entity type")
	}
	if before.String("LastUpdateDate") == "" && before.String("VersionId") == "" {
		return nil, errors.New("reload this work item before moving it; its revision is unavailable")
	}
	if _, ok := fields["Rank"]; ok {
		return nil, errors.New("use relative ranking, not Rank")
	}
	if _, ok := fields["DragAndDropRank"]; ok {
		return nil, errors.New("use relative ranking, not DragAndDropRank")
	}
	if fields == nil {
		fields = Object{}
	}
	fetch := map[string]bool{"ObjectID": true, "LastUpdateDate": true, "VersionId": true, "DragAndDropRank": true}
	for key := range before {
		if !strings.HasPrefix(key, "_") {
			fetch[key] = true
		}
	}
	for key := range fields {
		fetch[key] = true
	}
	names := make([]string, 0, len(fetch))
	for key := range fetch {
		names = append(names, key)
	}
	sort.Strings(names)
	query := url.Values{"fetch": {strings.Join(names, ",")}}
	if position != nil {
		target, err := c.resolve(position.Ref)
		if err != nil {
			return nil, err
		}
		source, err := c.resolve(before.String("_ref"))
		if err != nil {
			return nil, err
		}
		if source.Path == target.Path {
			return nil, errors.New("a work item cannot be ranked against itself")
		}
		parameter := "rankAbove"
		if position.Below {
			parameter = "rankBelow"
		}
		query.Set(parameter, target.Path)
	}
	current, err := c.getFields(ctx, before.String("_ref"), "ObjectID,VersionId,LastUpdateDate")
	if err != nil {
		return nil, err
	}
	for _, key := range []string{"LastUpdateDate", "VersionId"} {
		if stamp := before.String(key); stamp != "" && stamp != current.String(key) {
			return nil, errors.New("item changed on Rally; refresh before moving or undoing it")
		}
	}
	return c.mutateQuery(ctx, "POST", before.String("_ref"), k, fields, query)
}
