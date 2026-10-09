package ui

import "testing"

func TestLocalProviderPathsAndModelNames(t *testing.T) {
	for _, s := range []string{"http://localhost:11434", "http://localhost:11434/v1/"} {
		u, e := providerEndpoint(s, "/api/pull")
		if e != nil || u != "http://localhost:11434/api/pull" {
			t.Fatal(u, e)
		}
	}
	if _, e := providerEndpoint("file:///tmp/test", "/api/pull"); e == nil {
		t.Fatal("accepted non-HTTP endpoint")
	}
	for _, s := range []string{"gpt-oss:20b", "library/qwen3:8b-q4_K_M"} {
		if !validPullName(s) {
			t.Fatal(s)
		}
	}
	for _, s := range []string{"", "/x", "model:", "a;b", "gpt oss"} {
		if validPullName(s) {
			t.Fatal(s)
		}
	}
}
