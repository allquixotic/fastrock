package rally

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestV45RelativeRankRequestAndValidation(t *testing.T) {
	posts := 0
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "GET" {
			if r.URL.Query().Get("fetch") != "ObjectID,VersionId,LastUpdateDate" {
				t.Error("revision preflight fetched the full artifact", r.URL)
			}
			json.NewEncoder(w).Encode(map[string]any{"HierarchicalRequirement": Object{"ObjectID": 1, "_ref": "/slm/webservice/v2.0/hierarchicalrequirement/1", "VersionId": "7"}})
			return
		}
		posts++
		if r.URL.Query().Get("rankAbove") != "/slm/webservice/v2.0/hierarchicalrequirement/2" || r.URL.Query().Get("rankBelow") != "" || !strings.Contains(r.URL.Query().Get("fetch"), "DragAndDropRank") || r.URL.Query().Get("fetch") == "true" {
			t.Error(r.URL)
		}
		var body map[string]Object
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		if body["HierarchicalRequirement"]["ScheduleState"] != "Accepted" || body["HierarchicalRequirement"]["Rank"] != nil {
			t.Error(body)
		}
		json.NewEncoder(w).Encode(map[string]any{"OperationResult": map[string]any{"Object": Object{"VersionId": "8", "DragAndDropRank": "B"}}})
	}))
	defer server.Close()
	c, _ := New(server.URL, "fixture", nil)
	before := Object{"ObjectID": 1, "_ref": "/slm/webservice/v2.0/hierarchicalrequirement/1", "VersionId": "7"}
	got, err := c.UpdatePositionIfUnchanged(context.Background(), before, "HierarchicalRequirement", Object{"ScheduleState": "Accepted"}, &RankPosition{Ref: "/slm/webservice/v2.0/hierarchicalrequirement/2"})
	if err != nil || got.String("VersionId") != "8" || posts != 1 {
		t.Fatal(got, err, posts)
	}
	for _, ref := range []string{before.String("_ref"), "https://other.invalid/slm/webservice/v2.0/hierarchicalrequirement/2", "/outside/2", "/slm/webservice/v2.0/hierarchicalrequirement/2?rankAbove=3"} {
		if _, err := c.UpdatePositionIfUnchanged(context.Background(), before, "HierarchicalRequirement", nil, &RankPosition{Ref: ref}); err == nil {
			t.Fatal("invalid rank target accepted", ref)
		}
	}
	stale := before.Clone()
	stale["VersionId"] = "6"
	if _, err := c.UpdatePositionIfUnchanged(context.Background(), stale, "HierarchicalRequirement", nil, nil); err == nil || !strings.Contains(err.Error(), "changed") {
		t.Fatal(err)
	}
	missing := before.Clone()
	delete(missing, "VersionId")
	if _, err := c.UpdatePositionIfUnchanged(context.Background(), missing, "HierarchicalRequirement", nil, nil); err == nil {
		t.Fatal("missing revision accepted")
	}
	if _, err := c.UpdatePositionIfUnchanged(context.Background(), before, "HierarchicalRequirement", Object{"DragAndDropRank": "invented"}, nil); err == nil {
		t.Fatal("direct rank write accepted")
	}
	if posts != 1 {
		t.Fatal("invalid request wrote", posts)
	}
}
