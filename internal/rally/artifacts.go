package rally

import (
	"context"
	"fmt"
	"slices"
	"strings"
)

// Kind remains the default creation/schema type; these pages read a mixed,
// server-paged Artifact collection instead of separate per-type result sets.
func (p PageSpec) ArtifactTypes() []string {
	if p.ID == "teamboard" {
		return []string{"HierarchicalRequirement", "Defect", "TestSet", "DefectSuite"}
	}
	return nil
}

func (p PageSpec) QueryKind() string {
	if len(p.ArtifactTypes()) > 0 {
		return "Artifact"
	}
	return p.Kind
}

func canonicalArtifactTypes(types []string) ([]string, error) {
	out := make([]string, 0, len(types))
	for _, value := range types {
		kind, ok := CanonicalKind(value)
		if !ok || !slices.Contains(ArtifactKinds, kind) {
			return nil, fmt.Errorf("unsupported artifact type %q", value)
		}
		if !slices.Contains(out, kind) {
			out = append(out, kind)
		}
	}
	if len(out) == 0 {
		return nil, fmt.Errorf("choose at least one artifact type")
	}
	return out, nil
}

// The official SDK's artifact Store sends concrete type paths as the `types`
// query parameter. This also lets WSAPI resolve fields shared by those types.
// https://rally1.rallydev.com/docs/en-us/saas/apps/2.1/doc/source/Store7.html
func (c *Client) queryArtifacts(ctx context.Context, q Query) (Page, error) {
	types, err := canonicalArtifactTypes(strings.Split(q.ArtifactTypes, ","))
	if err != nil {
		return Page{}, err
	}
	q.ArtifactTypes = strings.Join(types, ",")
	page, err := c.Collection(ctx, "artifact", q)
	if err != nil {
		return Page{}, err
	}
	for _, object := range page.Results {
		kind, valid := c.ReferenceKind(object.String("_ref"))
		declared, known := CanonicalKind(object.String("_type"))
		if !valid || !slices.Contains(types, kind) || object.String("_type") != "" && (!known || declared != kind) {
			return Page{}, fmt.Errorf("Rally returned an artifact outside the requested types or connection")
		}
		object["_type"] = kind
	}
	return page, nil
}

func isArtifactQuery(kind string) bool { return strings.EqualFold(kind, "Artifact") }
