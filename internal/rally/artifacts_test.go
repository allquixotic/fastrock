package rally

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
)

func TestV66ArtifactQuery(t *testing.T) {
	var artifacts atomic.Int32
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		q := r.URL.Query()
		if q.Get("workspace") != WSAPI+"workspace/7" {
			t.Error("workspace lost", q)
		}
		artifacts.Add(1)
		if r.URL.Path != WSAPI+"artifact" || q.Get("start") != "129" || q.Get("pagesize") != "128" || q.Get("project") != WSAPI+"project/8" || q.Get("projectScopeDown") != "true" || q.Get("order") != "DragAndDropRank ASC" {
			t.Error("paging/scope lost", r.URL.String())
		}
		if q.Get("types") != "HierarchicalRequirement,Defect" && q.Get("types") != "Defect" {
			t.Error("wrong artifact types", q)
		}
		for _, clause := range []string{`Name contains "keep"`} {
			if !strings.Contains(q.Get("query"), clause) {
				t.Error("missing type/user predicate", q)
			}
		}
		objects := []Object{{"_ref": WSAPI + "hierarchicalrequirement/1", "_type": "HierarchicalRequirement", "ObjectID": 1, "Name": "Keep story"}, {"_ref": WSAPI + "defect/2", "ObjectID": 2, "Name": "Keep defect"}}
		if q.Get("types") == "Defect" {
			objects = objects[1:]
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": objects, "TotalResultCount": 200, "StartIndex": 129}})
	}))
	defer s.Close()
	c, _ := New(s.URL, "fixture", nil)
	q := Query{ArtifactTypes: "HierarchicalRequirement,Defect", Workspace: WSAPI + "workspace/7", Project: WSAPI + "project/8", Children: true, Expression: `(Name contains "keep")`, Start: 129, PageSize: 128, Order: "DragAndDropRank ASC", Fetch: "ObjectID,Name"}
	for range 2 {
		p, err := c.CachedQuery(context.Background(), "Artifact", q, false)
		if err != nil || p.Total != 200 || p.Start != 129 || len(p.Results) != 2 || p.Results[1].String("_type") != "Defect" {
			t.Fatal(p, err)
		}
	}
	if artifacts.Load() != 1 {
		t.Fatal("cache missed", artifacts.Load())
	}
	q.ArtifactTypes = "Defect"
	p, err := c.CachedQuery(context.Background(), "Artifact", q, false)
	if err != nil || len(p.Results) != 1 || artifacts.Load() != 2 {
		t.Fatal("type filters reused another cached result", p, err)
	}
	before := artifacts.Load()
	p, err = c.Query(context.Background(), "Task", q)
	if err != nil || len(p.Results) != 0 || artifacts.Load() != before {
		t.Fatal("concrete query escaped the type scope", p, err)
	}
}

func TestV66ArtifactQueryRejectsInvalidTypes(t *testing.T) {
	for _, tc := range []struct{ name, types, objects string }{
		{"empty", "", `[]`},
		{"nonartifact", "User", `[]`},
		{"unknown type", "Defect OR Task", `[]`},
		{"foreign ref", "Defect", `[{"_ref":"https://foreign.test/slm/webservice/v2.0/defect/1"}]`},
		{"wrong type", "Defect", `[{"_ref":"/slm/webservice/v2.0/task/1"}]`},
		{"mismatched type", "Defect", `[{"_ref":"/slm/webservice/v2.0/defect/1","_type":"Task"}]`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				results := tc.objects
				var objects []Object
				if err := json.Unmarshal([]byte(results), &objects); err != nil {
					t.Error(err)
				}
				json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": objects, "TotalResultCount": len(objects), "StartIndex": 1}})
			}))
			defer s.Close()
			c, _ := New(s.URL, "fixture", nil)
			if _, err := c.Query(context.Background(), "Artifact", Query{ArtifactTypes: tc.types}); err == nil {
				t.Fatal("accepted invalid type scope or response")
			}
		})
	}
	c, _ := New("https://rally.test", "fixture", nil)
	if _, err := c.Create(context.Background(), "Artifact", Object{}); err == nil {
		t.Fatal("abstract artifact became a writable type")
	}
}
