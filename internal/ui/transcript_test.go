package ui

import (
	"encoding/json"
	"github.com/aarzilli/nucular/font"
	"github.com/allquixotic/fastrock/internal/richtext"
	"strings"
	"testing"
)

func TestTranscriptFormatsLinksAndTables(t *testing.T) {
	l := prepareTranscript("# Heading\n\n**bold** and _italic_ with [link](https://example.com).\n\n| Name | Value |\n|---|---|\n| one | two |\n\n```go\nfmt.Println(42)\n```", 600, 13)
	bold, italic, link, table, code := false, false, false, false, false
	var plain strings.Builder
	for _, line := range l.Lines {
		table = table || line.Table
		code = code || line.Code
		for _, r := range line.Runs {
			plain.WriteString(r.Text)
			bold = bold || r.Format.Style&richtext.Bold != 0
			italic = italic || r.Format.Style&richtext.Italic != 0
			link = link || r.Format.Link == "https://example.com"
		}
	}
	if !bold || !italic || !link || !table || !code {
		t.Fatalf("missing formatting %v %v %v %v %v", bold, italic, link, table, code)
	}
	if strings.Contains(plain.String(), "**") || strings.Contains(plain.String(), "](") {
		t.Fatal(plain.String())
	}
}

func TestRichSelectionUsesUTF8Boundaries(t *testing.T) {
	l := prepareTranscript("**Hello** 🚀 café\n\n[Details](https://example.com)", 600, 13)
	start := strings.Index(l.Plain, "🚀")
	s := transcriptSelection{BlockID: "reply", Layout: l, Anchor: start + len("🚀 café"), End: start}
	if got := s.text("reply"); got != "🚀 café" {
		t.Fatalf("selection %q", got)
	}
	if s.text("other") != "" {
		t.Fatal("selection leaked across messages")
	}
	f, _ := font.NewFace(uiRegular, 13)
	defer f.Face.Close()
	for _, line := range l.Lines {
		for _, run := range line.Runs {
			if got := transcriptHit(line, run.X, f); got != run.Start {
				t.Fatalf("hit %d, want %d", got, run.Start)
			}
		}
	}
}

func TestDisabledSkillsCannotBeInvoked(t *testing.T) {
	var skills skillList
	if err := json.Unmarshal([]byte(`{"data":[{"skills":[{"name":"enabled","path":"/a","enabled":true},{"name":"disabled","path":"/b","enabled":false}]}]}`), &skills); err != nil {
		t.Fatal(err)
	}
	got := skillInputs("$enabled $disabled $enabled", skills)
	if len(got) != 1 || got[0]["name"] != "enabled" {
		t.Fatal(got)
	}
}
func TestCompletionPreservesSurroundingDraft(t *testing.T) {
	v := newChatView()
	setText(v.Editor, "Please check @src later")
	v.Editor.Cursor = len([]rune("Please check @src"))
	v.SuggestQuery = "@src"
	v.Suggest = []string{"@src/main.go"}
	acceptSuggestion(v)
	if text(v.Editor) != "Please check @src/main.go  later" {
		t.Fatal(text(v.Editor))
	}
}
