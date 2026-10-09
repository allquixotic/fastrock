package ui

import "strings"

// Matches the installed Codex GUI's provider setup region catalog. Unknown
// configured regions remain visible so opening Settings never changes them.
var bedrockRegions = []struct{ code, name string }{
	{"us-east-2", "US East (Ohio)"}, {"us-east-1", "US East (N. Virginia)"}, {"us-west-2", "US West (Oregon)"},
	{"ap-southeast-3", "Asia Pacific (Jakarta)"}, {"ap-south-1", "Asia Pacific (Mumbai)"}, {"ap-northeast-1", "Asia Pacific (Tokyo)"},
	{"eu-central-1", "Europe (Frankfurt)"}, {"eu-west-1", "Europe (Ireland)"}, {"eu-west-2", "Europe (London)"},
	{"eu-south-1", "Europe (Milan)"}, {"eu-north-1", "Europe (Stockholm)"}, {"sa-east-1", "South America (São Paulo)"},
	{"us-gov-east-1", "AWS GovCloud (US-East)"}, {"us-gov-west-1", "AWS GovCloud (US-West)"},
}

func bedrockRegionChoices(current string) ([]string, int) {
	labels := make([]string, 0, len(bedrockRegions)+1)
	selected := -1
	for i, region := range bedrockRegions {
		labels = append(labels, region.code+" · "+region.name)
		if strings.EqualFold(strings.TrimSpace(current), region.code) {
			selected = i
		}
	}
	if selected < 0 {
		selected = len(labels)
		labels = append(labels, current+" (configured region)")
	}
	return labels, selected
}
