package mockrally

import (
	"context"
	"net/http/httptest"
	"slices"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/rally"
)

func TestArtifactFixtureMatchesTeamBoardQueries(t *testing.T) {
	for _, tc := range []struct {
		name  string
		data  *Server
		types []string
		total int
	}{
		{"stories and defects", New(), []string{"HierarchicalRequirement", "Defect"}, 24},
		{"team board", New(), rally.FindPage("teamboard").ArtifactTypes(), 26},
		{"large team board", Large(10000), rally.FindPage("teamboard").ArtifactTypes(), 10006},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := httptest.NewServer(tc.data)
			defer s.Close()
			c, err := rally.New(s.URL, "mock-token", nil)
			if err != nil {
				t.Fatal(err)
			}
			page, err := c.Query(context.Background(), "Artifact", rally.Query{ArtifactTypes: strings.Join(tc.types, ",")})
			if err != nil {
				t.Fatal(err)
			}
			if page.Total != tc.total {
				t.Fatalf("fixture returned %d items, want %d", page.Total, tc.total)
			}
			for _, item := range page.Results {
				if !slices.Contains(tc.types, item.Kind()) {
					t.Fatalf("unexpected fixture kind %q", item.Kind())
				}
			}
		})
	}
}
