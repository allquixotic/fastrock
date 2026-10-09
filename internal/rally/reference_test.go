package rally

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
)

func TestV58ReferenceKindRestrictsOriginAndObjectPath(t *testing.T) {
	c, err := New("https://rally.example", "test", nil)
	if err != nil {
		t.Fatal(err)
	}
	for ref, want := range map[string]string{
		WSAPI + "user/42": "User",
		"https://rally.example" + WSAPI + "portfolioitem/feature/9": "PortfolioItem/Feature",
		"https://other.example" + WSAPI + "user/42":                 "",
		WSAPI + "user/42?fetch=true":                                "",
		WSAPI + "user/../42":                                        "",
		WSAPI + "user/zero":                                         "",
		WSAPI + "user/0":                                            "",
		WSAPI + "user/42/Tasks":                                     "",
		"/outside/user/42":                                          "",
	} {
		got, ok := c.ReferenceKind(ref)
		if got != want || ok != (want != "") {
			t.Fatal(ref, got, ok)
		}
	}
}

func TestV58SchemaKeepsReferenceTargets(t *testing.T) {
	var calls atomic.Int32
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path == WSAPI+"typedefinition" {
			fmt.Fprintf(w, `{"QueryResult":{"Results":[{"ObjectID":1,"Attributes":{"_ref":"http://%s/slm/webservice/v2.0/typedefinition/1/Attributes"}}],"TotalResultCount":1,"StartIndex":1}}`, r.Host)
			return
		}
		rows := []Object{
			{"ElementName": "WorkProduct", "Name": "Work item", "AttributeType": "OBJECT", "Type": "Artifact"},
			{"ElementName": "c_Reviewer", "Name": "Reviewer", "AttributeType": "OBJECT", "Type": "User"},
			{"ElementName": "Feature", "Name": "Feature", "AttributeType": "OBJECT", "Type": "PortfolioItem", "AllowedValueType": map[string]any{"TypePath": "PortfolioItem/Feature"}},
		}
		json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": rows, "TotalResultCount": len(rows), "StartIndex": 1}})
	}))
	defer s.Close()
	c, _ := New(s.URL, "test", nil)
	fields, err := c.Fields(context.Background(), "Task")
	if err != nil || len(fields) != 3 || fields[1].ReferenceType != "User" || fields[2].ReferenceType != "PortfolioItem/Feature" {
		t.Fatal(fields, err)
	}
	fields[1].ReferenceType = "modified"
	fields, err = c.Fields(context.Background(), "Task")
	if err != nil || fields[1].ReferenceType != "User" || calls.Load() != 2 {
		t.Fatal("schema cache changed or was not reused", fields, err, calls.Load())
	}
}
