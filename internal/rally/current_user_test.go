package rally

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestV55CurrentUserIsUnscopedAndValidated(t *testing.T) {
	for _, kind := range []string{"valid", "foreign", "wrong type", "wrong id", "collection", "error"} {
		t.Run(kind, func(t *testing.T) {
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				q := r.URL.Query()
				if r.Method != http.MethodGet || r.URL.Path != WSAPI+"user" || len(q) != 1 || q.Get("fetch") != "ObjectID,UserName,DisplayName" {
					t.Errorf("identity request was filtered or scoped: %s %s", r.Method, r.URL)
				}
				ref := "http://" + r.Host + WSAPI + "user/42"
				switch kind {
				case "foreign":
					ref = "https://foreign.invalid" + WSAPI + "user/42"
				case "wrong type":
					ref = "http://" + r.Host + WSAPI + "project/42"
				case "wrong id":
					ref = "http://" + r.Host + WSAPI + "user/43"
				case "collection":
					json.NewEncoder(w).Encode(map[string]any{"QueryResult": map[string]any{"Results": []Object{{"ObjectID": 42, "_ref": ref}}, "TotalResultCount": 1}})
					return
				case "error":
					w.WriteHeader(http.StatusUnauthorized)
					return
				}
				json.NewEncoder(w).Encode(map[string]any{"User": Object{"ObjectID": 42, "_ref": ref, "DisplayName": "Test User", "UserName": "test@example.invalid"}})
			}))
			defer s.Close()
			c, err := New(s.URL, "test-token", nil)
			if err != nil {
				t.Fatal(err)
			}
			u, err := c.CurrentUser(context.Background())
			if kind == "valid" {
				if err != nil || u.String("ObjectID") != "42" || u.String("DisplayName") != "Test User" {
					t.Fatal(u, err)
				}
			} else if err == nil || u != nil {
				t.Fatal("accepted an invalid identity", u, err)
			}
		})
	}
}

func TestV55CurrentUserHonorsCancellation(t *testing.T) {
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Error("cancelled request reached the server")
	}))
	defer s.Close()
	c, _ := New(s.URL, "test", nil)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := c.CurrentUser(ctx); err == nil {
		t.Fatal("cancelled identity lookup succeeded")
	}
}
