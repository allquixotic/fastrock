package ui

import (
	"encoding/binary"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/command"
	"github.com/allquixotic/fastrock/internal/desktop/rect"
	"github.com/allquixotic/fastrock/internal/richtext"
	"golang.org/x/image/font/gofont/gobold"
	"golang.org/x/image/font/gofont/goregular"
	"golang.org/x/image/font/sfnt"
)

func TestV29FontVariantsAndSizes(t *testing.T) {
	for v := regularFont; v <= monoBoldItalicFont; v++ {
		face := typeFace(13, v)
		if face != typeFace(13, v) || fontPointSize(face) != 13 {
			t.Fatal("font cache/size", v)
		}
		if v >= monoFont && desktop.FontWidth(face, "iiii") != desktop.FontWidth(face, "WWWW") {
			t.Fatal("non-monospace code font", v)
		}
	}
	for _, tc := range []struct {
		format  richtext.Format
		size    int
		variant fontVariant
	}{
		{richtext.Format{Style: richtext.Bold}, 13, boldFont},
		{richtext.Format{Style: richtext.Bold | richtext.Italic}, 13, boldItalicFont},
		{richtext.Format{Style: richtext.Code | richtext.Italic}, 12, monoItalicFont},
		{richtext.Format{Heading: 1}, 20, boldFont},
		{richtext.Format{Heading: 2}, 16, boldFont},
	} {
		size, v := formattedFont(13, tc.format)
		if size != tc.size || v != tc.variant {
			t.Fatal(size, v, tc)
		}
	}
}
func TestV29TranscriptMeasuresDrawnFaces(t *testing.T) {
	layout := prepareTranscript("# Large heading\n\n**Bold words** and _italic words_ then `iiii WWWW`\n\n```sh\nprintf 'hello'\n```", 130, 13)
	var bold, italic, code, heading bool
	for _, line := range layout.Lines {
		right := 0
		for _, run := range line.Runs {
			if run.Face.Face == nil {
				continue
			}
			bold = bold || run.Face == typeFace(13, boldFont)
			italic = italic || run.Face == typeFace(13, italicFont)
			code = code || run.Face == typeFace(12, monoFont)
			heading = heading || run.Face == typeFace(20, boldFont)
			if run.Width != desktop.FontWidth(run.Face, run.Text) || run.X < right || line.Height < desktop.FontHeight(run.Face) {
				t.Fatal("measured and painted geometry differ", run)
			}
			if run.X+run.Width > 130 {
				t.Fatal("mixed-font wrap overflow", run)
			}
			right = run.X + run.Width
			for i, offset := range run.Offsets {
				if transcriptAdvance(run, offset) != run.Advances[i] {
					t.Fatal("selection advance drift")
				}
			}
		}
	}
	if !bold || !italic || !code || !heading {
		t.Fatal("missing real font variants", bold, italic, code, heading)
	}
}
func TestV29RichUsesRealFacesAndMeasurement(t *testing.T) {
	r := newRichEditor("<p><b>Bold</b> <i>italic</i> <code>code</code></p>")
	r.fontSize = 13
	r.ensureEditor()
	out := &command.Buffer{}
	out.Reset()
	r.paint(out, rect.Rect{X: 10, Y: 20, W: 600, H: 35}, r.doc.Text, 0, typeFace(13, regularFont), colors(false).Text, false)
	seen := map[string]bool{}
	width := 0
	for _, cmd := range out.Commands {
		if cmd.Kind != command.TextCmd {
			continue
		}
		if seen[cmd.Text.String] {
			t.Fatal("text was overstruck", cmd.Text.String)
		}
		// Two separate plain spaces are intentional, with the same normal face.
		if strings.TrimSpace(cmd.Text.String) != "" {
			seen[cmd.Text.String] = true
		}
		if cmd.Text.String == "Bold" && cmd.Text.Face != typeFace(13, boldFont) {
			t.Fatal("synthetic bold")
		}
		if cmd.Text.String == "italic" && cmd.Text.Face != typeFace(13, italicFont) {
			t.Fatal("wrong italic face")
		}
		if cmd.Text.String == "code" && cmd.Text.Face != typeFace(12, monoFont) {
			t.Fatal("wrong code face")
		}
		if cmd.X != 10+width {
			t.Fatal("run position drift", cmd.X, width)
		}
		width += desktop.FontWidth(cmd.Text.Face, cmd.Text.String)
	}
	if r.measure(r.doc.Text, 0, typeFace(13, regularFont)) != width || !seen["Bold"] || !seen["code"] {
		t.Fatal("native geometry differs from painted runs")
	}
	if len(r.widths) > 10 {
		t.Fatal("measurement retained per-character entries")
	}
}

func TestV29FontCollectionSelectsWeight(t *testing.T) {
	// Construct a two-face TTC from the deterministic embedded fixtures.
	data := make([]byte, 20)
	copy(data, "ttcf")
	binary.BigEndian.PutUint32(data[4:], 0x00010000)
	binary.BigEndian.PutUint32(data[8:], 2)
	for i, ttf := range [][]byte{goregular.TTF, gobold.TTF} {
		base := len(data)
		binary.BigEndian.PutUint32(data[12+4*i:], uint32(base))
		member := append([]byte(nil), ttf...)
		n := int(binary.BigEndian.Uint16(member[4:]))
		for j := 0; j < n; j++ {
			record := member[12+16*j:]
			binary.BigEndian.PutUint32(record[8:], binary.BigEndian.Uint32(record[8:])+uint32(base))
		}
		data = append(data, member...)
		for len(data)%4 != 0 {
			data = append(data, 0)
		}
	}
	for _, variant := range []fontVariant{regularFont, boldFont} {
		selected, err := collectionFont(data, variant)
		if err != nil {
			t.Fatal(err)
		}
		f, err := sfnt.Parse(selected)
		if err != nil {
			t.Fatal(err)
		}
		name, _ := f.Name(nil, sfnt.NameIDSubfamily)
		if strings.Contains(strings.ToLower(name), "bold") != (variant == boldFont) {
			t.Fatal("wrong collection weight", name)
		}
	}
	if _, err := collectionFont(data, italicFont); err == nil {
		t.Fatal("invented missing italic")
	}
}
func TestV29InstalledMacMonospace(t *testing.T) {
	if runtime.GOOS != "darwin" {
		t.Skip("macOS installed-font fixture")
	}
	path := "/System/Library/Fonts/Menlo.ttc"
	if _, err := os.Stat(path); os.IsNotExist(err) {
		t.Skip("Menlo not installed")
	}
	for v := monoFont; v <= monoBoldItalicFont; v++ {
		data, err := loadFontFile(path, v)
		if err != nil {
			t.Fatal(v, err)
		}
		f, err := sfnt.Parse(data)
		if err != nil {
			t.Fatal(err)
		}
		name, _ := f.Name(nil, sfnt.NameIDSubfamily)
		if strings.Contains(strings.ToLower(name), "bold") != (v%4 == boldFont || v%4 == boldItalicFont) {
			t.Fatal("wrong Menlo weight", name)
		}
	}
}

func TestV29RejectsWrongFontWeight(t *testing.T) {
	path := filepath.Join(t.TempDir(), "font.ttf")
	if err := os.WriteFile(path, goregular.TTF, 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadFontFile(path, boldFont); err == nil {
		t.Fatal("regular font masqueraded as bold")
	}
	if _, err := loadFontFile(path, regularFont); err != nil {
		t.Fatal(err)
	}
}
