package ui

import (
	"regexp"
	"strings"

	"github.com/aarzilli/nucular"
)

var markdownLink = regexp.MustCompile(`\[([^\]]+)\]\(([^\s)]+)\)`)

func (a *App) markdown(w *nucular.Window, source string) {
	inCode := false
	for _, line := range strings.Split(source, "\n") {
		if strings.HasPrefix(line, "```") {
			inCode = !inCode
			continue
		}
		if line == "" {
			w.Row(8).Dynamic(1)
			w.Spacing(1)
			continue
		}
		color := a.p.Text
		if inCode {
			color = a.p.Muted
		}
		if strings.HasPrefix(line, "#") {
			line = strings.TrimLeft(line, "# ")
			color = a.p.Accent
		}
		if strings.HasPrefix(line, "- ") {
			line = "• " + line[2:]
		}
		line = strings.ReplaceAll(line, "**", "")
		width := max(100, w.LayoutAvailableWidth())
		face := w.Master().Style().Font
		rows := len(nucular.WrapText(face, line, width-8))
		w.Row(max(28, rows*nucular.FontHeight(face)+8)).Dynamic(1)
		w.LabelWrapColored(line, color)
		for _, m := range markdownLink.FindAllStringSubmatch(line, -1) {
			w.Row(24).Static(min(width, 500))
			if w.ButtonText("Open: " + m[1]) {
				a.openLink(m[2])
			}
		}
	}
}
