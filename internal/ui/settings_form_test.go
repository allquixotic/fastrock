//go:build fltk_headless

package ui

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestV34FormFailuresAndStaleReplies(t *testing.T) {
	a := settingsFixture(t)
	s := a.settingsView
	s.Page = "Local providers"
	a.settingsFormCall("test", "fixture/fail", nil, nil)
	key := "page:Local providers:test"
	if !s.ActionFeedback[key].Pending {
		t.Fatal("pending not visible")
	}
	drain(t, a, func() bool { return !s.ActionFeedback[key].Pending })
	if !s.ActionFeedback[key].Failed || !strings.Contains(s.ActionFeedback[key].Message, "rejected") || a.toast != "" {
		t.Fatal(s.ActionFeedback[key], a.toast)
	}
	a.settingsFormCall("test", "config/batchWrite", nil, nil)
	a.serverGeneration++
	drain(t, a, func() bool { return !s.ActionFeedback[key].Pending })
	if !s.ActionFeedback[key].Failed || !strings.Contains(s.ActionFeedback[key].Message, "restarted") {
		t.Fatal(s.ActionFeedback[key])
	}
	a.settingsFormCall("test", "fixture/fail", nil, nil)
	a.resetSettingsConnection()
	a.settingsFormCall("test", "config/batchWrite", nil, nil)
	drain(t, a, func() bool { return !s.ActionFeedback[key].Pending })
	if s.ActionFeedback[key].Failed || s.ActionFeedback[key].Message != "Saved" {
		t.Fatal("old callback clobbered retry", s.ActionFeedback[key])
	}

}
func TestV34ProviderErrorsStayInline(t *testing.T) {
	a := settingsFixture(t)
	f := &bedrockForm{Region: textEditor("", false)}
	a.applyBedrock(f)
	if f.Busy || !f.Feedback.Failed || a.toast != "" {
		t.Fatal(f.Feedback, a.toast)
	}
	setText(f.Region, "us-east-1")
	a.applyBedrock(f)
	if !f.Busy || !f.Feedback.Pending {
		t.Fatal("missing provider progress")
	}
	drain(t, a, func() bool { return !f.Busy })
	if !f.Feedback.Failed || !strings.Contains(f.Feedback.Message, "credentials rejected") || a.toast != "" {
		t.Fatal(f.Feedback, a.toast)
	}
	v := a.localProvider(a.settingsView)
	setText(v.PullName, "../invalid")
	a.pullLocal(v)
	if !v.Failed || !strings.Contains(v.Status, "Enter a model name") || a.toast != "" {
		t.Fatal(v.Status, a.toast)
	}
}
func TestV34RawConfigErrorsPreserveDraft(t *testing.T) {
	a := settingsFixture(t)
	s := a.settingsView
	s.RawPath = filepath.Join(t.TempDir(), "config.toml")
	original := "model = \"original\"\n"
	if err := os.WriteFile(s.RawPath, []byte(original), 0600); err != nil {
		t.Fatal(err)
	}
	setText(s.Raw, "[invalid")
	a.saveRawConfig()
	if !s.RawFeedback.Pending {
		t.Fatal("missing saving state")
	}
	drain(t, a, func() bool { return !s.Busy })
	if !s.RawFeedback.Failed || text(s.Raw) != "[invalid" || a.toast != "" {
		t.Fatal(s.RawFeedback, a.toast)
	}
	got, err := os.ReadFile(s.RawPath)
	if err != nil || string(got) != original {
		t.Fatal(string(got), err)
	}
}
