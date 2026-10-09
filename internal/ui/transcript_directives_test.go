//go:build fltk_headless

package ui

import (
	"fmt"
	"image"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/richtext"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

func transcriptLinkTargets(layout *transcriptLayout) map[string]string {
	links := map[string]string{}
	for _, line := range layout.Lines {
		for _, run := range line.Runs {
			if run.Format.Link != "" {
				links[run.Text] = run.Format.Link
			}
		}
	}
	return links
}

func TestV65DirectiveParser(t *testing.T) {
	source := `::code-comment{title="A \"quoted\" title" body='body with \\ path' file="C:\\repo\\a.go" start=5 end=9 priority=P1} trailing`
	d, ok := parseTranscriptDirective(source)
	if !ok || d.name != "code-comment" || d.attributes["title"] != `A "quoted" title` || d.attributes["body"] != `body with \ path` || d.attributes["file"] != `C:\repo\a.go` || source[d.length:] != " trailing" {
		t.Fatal(d, ok)
	}
	d, ok = parseTranscriptDirective(`:codex-followup[Run [unit] tests]{prompt="go test ./..."}`)
	if !ok || d.label != "Run [unit] tests" {
		t.Fatal(d, ok)
	}
	for _, bad := range []string{`::code-comment{x=1 x=2}`, `::::code-comment{}`, `::1comment{}`, `:codex-followup[unclosed{x=1}`, "::code-comment{body=\"a\nb\"}", `::code-comment{body="unterminated}`, `::code-comment{body=}`, `:codex-followup[x]{a}`} {
		if _, ok := parseTranscriptDirective(bad); ok {
			t.Fatal("malformed directive accepted", bad)
		}
	}
}

func TestV65DirectivesRenderWithoutChangingSource(t *testing.T) {
	source := "Review:\n\n" + `::code-comment{title="Wrong branch" body="Keep **the current draft**." file="/repo/src/a.go" start=3 end=5 priority=1}` + "\n\n" +
		`See :codex-file-citation{path="src/hello world.go:12"}. :codex-followup[Run [unit] tests]{prompt="private follow-up content"}` + "\n" +
		`::git-stage{cwd="/repo"} ::git-commit{cwd="/repo"}`
	layout := prepareTranscript(source, 1600, 13)
	if layout.Text != source || !strings.Contains(layout.Plain, "[P1] Wrong branch") || !strings.Contains(layout.Plain, "Keep the current draft") || !strings.Contains(layout.Plain, "Run [unit] tests") || strings.Contains(layout.Plain, "::") || strings.Contains(layout.Plain, "private follow-up content") {
		t.Fatal(layout.Plain)
	}
	links := transcriptLinkTargets(layout)
	if path, line, _ := fileLocation(links["/repo/src/a.go:3-5"]); path != "/repo/src/a.go" || line != 3 {
		t.Fatal("comment location missing", links)
	}
	if path, line, _ := fileLocation(links["src/hello world.go:12"]); path != "src/hello world.go" || line != 12 {
		t.Fatal("citation lost spaces or line", links)
	}
	user := prepareMarkdownTranscript(`:codex-followup[Run tests]{prompt="keep this literal"}`, 1000, 13, false)
	if !strings.Contains(user.Plain, ":codex-followup") {
		t.Fatal("user examples interpreted as assistant directives", user.Plain)
	}
}

func TestV65DirectivesPreserveCodeAndInvalidInput(t *testing.T) {
	directive := `:codex-followup[Label]{prompt="keep raw"}`
	for _, source := range []string{
		"```text\n" + directive + "\n```", "~~~~text\n" + directive + "\n~~~~", "    " + directive,
		"`" + directive + "`", "``code ` " + directive + "``",
		"> ```\n> " + directive + "\n> ```", "- ```\n  " + directive + "\n  ```",
		`\:codex-followup[Label]{prompt="keep raw"}`, `::code-comment{title="missing body"}`,
		`:codex-followup[unclosed`,
	} {
		if got := visibleTranscriptMarkdown(source); got != source {
			t.Errorf("literal input rewritten:\n%s\n=>\n%s", source, got)
		}
	}
	for _, source := range []string{
		`::code-comment{title="[P2] Keep priority" body="Body" file="x.go" priority=0}`,
		`::code-comment{title="Plain" body="Body" file="x.go" start=-3 end=0}`,
	} {
		layout := prepareTranscript(source, 1000, 13)
		if strings.Contains(layout.Plain, "::code-comment") || strings.Contains(layout.Plain, "[P0] [P2]") || !strings.Contains(layout.Plain, "x.go:1") {
			t.Fatal(layout.Plain)
		}
	}
}

func TestV65TranscriptLinks(t *testing.T) {
	source := "See src/a.go:12, `src/b.go#L8-L9` and [named file](<src/hello%20world.go:5:2>).\n\n" +
		"mailto:team@example.test and [email](MAILTO:team@example.test).\n\n" +
		"Unicode\u00a0src/Ω.go:3\u2003done.\n\n" +
		"```\nsrc/not-linked.go:15\n```\n\n" + "`x / y` and [existing](https://example.test)."
	layout := prepareTranscript(source, 1400, 13)
	links := transcriptLinkTargets(layout)
	for label, path := range map[string]string{"src/a.go:12": "src/a.go:12", "src/b.go#L8-L9": "src/b.go#L8-L9", "named file": "src/hello%20world.go:5:2", "email": "MAILTO:team@example.test", "src/Ω.go:3": "src/Ω.go:3", "existing": "https://example.test"} {
		if links[label] != path {
			t.Errorf("%s: got %q want %q (all %v)", label, links[label], path, links)
		}
	}
	for _, target := range links {
		if strings.Contains(target, "not-linked") {
			t.Fatal("fenced code became an active file link", links)
		}
	}
	for _, candidate := range []string{"a/b", "v1.2", "foo()", "api::Type", "package.name", "3.14", "https://host/a.go", "mailto:x@y.z", "$(touch)/a.go", "--option=/a.go", "a.go:0"} {
		if citationDestination(candidate) != "" {
			t.Fatal("non-citation became a file link", candidate)
		}
	}
	for _, line := range layout.Lines {
		for _, run := range line.Runs {
			if !utf8.ValidString(run.Text) {
				t.Fatal("autolink split a Unicode character")
			}
		}
	}
}

func TestV65LinkValidationAndLocations(t *testing.T) {
	for _, tc := range []struct {
		target, path string
		line, column int
	}{
		{"src/a.go:12-14", "src/a.go", 12, 1}, {"src/a.go:12:3", "src/a.go", 12, 3},
		{"src/a.go#L12-L14", "src/a.go", 12, 1}, {"src/a.go#L12C3", "src/a.go", 12, 3},
		{"file:///tmp/a%23b.go#L2", "/tmp/a#b.go", 2, 1}, {"file://localhost/tmp/a.go:8", "/tmp/a.go", 8, 1},
		{"src/hello%20world.go:5", "src/hello world.go", 5, 1}, {`C:\work\a.go:8`, `C:\work\a.go`, 8, 1},
		{`file:///C:/work/a%20b.go#L12C4`, "C:/work/a b.go", 12, 4},
		{"file://server/share/a.go:3", "//server/share/a.go", 3, 1}, {"src/a.go:9#L2", "src/a.go", 2, 1},
	} {
		t.Run(tc.target, func(t *testing.T) {
			path, line, column := fileLocation(tc.target)
			if path != tc.path || line != tc.line || column != tc.column || transcriptSafeLink(tc.target) == "" {
				t.Fatal(path, line, column)
			}
		})
	}
	for _, bad := range []string{"javascript:alert(1)", "data:text/plain,foo", "https://", "custom://host", "javascript%3Aalert(1)", "file:opaque", "file:///tmp/a%00b", "src/a.go\nother", "", "#anchor", "a%zz.go"} {
		if got := transcriptSafeLink(bad); got != "" {
			t.Fatal("unsupported target accepted", bad, got)
		}
	}
	for _, target := range []string{"https://example.test", "HTTP://example.test", "mailto:team@example.test", "MAILTO:team@example.test"} {
		if !externalTranscriptLink(target) || transcriptSafeLink(target) != target {
			t.Fatal("web/email link treated as a file", target)
		}
	}
	if richtext.Parse(`<a href="file:///tmp/local.go">file</a>`).FormatAt(0).Link != "" {
		t.Fatal("Rally editor unexpectedly accepted local links")
	}
	layout := prepareTranscript(`[bad](javascript:alert) [local](file:///tmp/a.go)`, 900, 13)
	if links := transcriptLinkTargets(layout); links["bad"] != "" || links["local"] != "file:///tmp/a.go" {
		t.Fatal(links)
	}
}

func TestV65CitationClickUsesOwningDirectory(t *testing.T) {
	for _, scale := range []float64{1, 1.5, 2} {
		t.Run(fmt.Sprint(scale), func(t *testing.T) {
			a := presetApp(t)
			root, other := t.TempDir(), t.TempDir()
			if err := os.WriteFile(filepath.Join(root, "source.go"), []byte("one\ntwo\nthree\n"), 0600); err != nil {
				t.Fatal(err)
			}
			a.prefs.WorkingDirectory = other
			c := &workspace.Conversation{ID: "other", Cwd: other}
			a.state.Chats[c.ID] = c
			a.state.Open(workspace.Chat, "Other", c.ID, "")
			v := newChatView()
			layout := prepareTranscript("source.go:3", 900, 13)
			layout.Cwd = root
			var click image.Point
			h := desktop.NewHeadlessHarness(0, image.Pt(900, 300), func(w *desktop.Window) {
				if click != (image.Point{}) {
					m := &w.Input().Mouse
					m.Pos, m.Buttons[mouse.ButtonLeft].ClickedPos = click, click
					m.Buttons[mouse.ButtonLeft].Clicked = true
				}
				w.Row(layout.Lines[0].Height).Dynamic(1)
				a.drawTranscriptLine(w, v, "reply", layout, layout.Lines[0])
				b := w.LastWidgetBounds
				click = image.Pt(b.X+8, b.Y+4)
			})
			style := makeStyle(a.p, 13)
			style.Scale(scale)
			h.Master().SetStyle(style)
			h.Frame(false)
			h.Frame(false)
			current := a.state.Current()
			if current == nil || current.Kind != workspace.File || current.Target != filepath.Join(root, "source.go") {
				t.Fatal("citation used active chat instead of its owner", current)
			}
			file := a.files[current.ID]
			drain(t, a, func() bool { return !file.Loading && file.Editor != nil && file.PendingLine == 0 })
			if file.Editor.Cursor != len([]rune("one\ntwo\n")) {
				t.Fatal("citation did not navigate to its line", file.Editor.Cursor)
			}
		})
	}
}

func TestV65LayoutDirectoryCache(t *testing.T) {
	a := presetApp(t)
	a.layoutJobs = make(chan func(), 4)
	v := newChatView()
	v.Cwd = "/first"
	b := workspace.Block{ID: "block", Role: "assistant", Text: "src/main.go:3"}
	first := a.transcriptLayout(v, b, 800)
	(<-a.layoutJobs)()
	drain(t, a, func() bool { return !v.Layouts[b.ID].Pending })
	if first.Cwd != "/first" || v.Layouts[b.ID].Cwd != "/first" {
		t.Fatal("layout lost its directory")
	}
	v.Cwd = "/second"
	second := a.transcriptLayout(v, b, 800)
	if second == v.Layouts[b.ID] && !second.Pending || second.Cwd != "/second" {
		t.Fatal("directory change reused the old link context")
	}
	(<-a.layoutJobs)()
	drain(t, a, func() bool { return !v.Layouts[b.ID].Pending })
	if v.Layouts[b.ID].Cwd != "/second" {
		t.Fatal("stale directory published")
	}
	b.Role = "tool"
	literal := a.transcriptLayout(v, b, 800)
	if !literal.Pending || len(literal.Lines) != 0 {
		t.Fatal("literal output reused interactive Markdown")
	}
	(<-a.layoutJobs)()
	drain(t, a, func() bool { return !v.Layouts[b.ID].Pending })
	if v.Layouts[b.ID].Markup != "literal" || len(transcriptLinkTargets(v.Layouts[b.ID])) != 0 {
		t.Fatal("role change retained Markdown actions")
	}
}

func FuzzV65DirectiveAndLinkRendering(f *testing.F) {
	for _, s := range []string{`::code-comment{title="Title" body="Body" file="src/a.go" start=3}`, `:codex-followup[Label]{prompt="a"}`, "`src/a.go:3`", "x\u00a0src/Ω.go:3", "[x](javascript:bad)", "> ```\n::git-stage{}\n> ```"} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, source string) {
		if len(source) > 4096 || !utf8.ValidString(source) {
			t.Skip()
		}
		layout := prepareTranscript(source, 400, 13)
		if layout.Text != source || !utf8.ValidString(layout.Plain) {
			t.Fatal("source or Unicode lost")
		}
		for _, line := range layout.Lines {
			if line.Start < 0 || line.End < line.Start || line.End > len(layout.Plain) {
				t.Fatal("invalid selection bounds")
			}
		}
	})
}
