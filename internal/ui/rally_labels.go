package ui

import "strings"

func rallyKindLabel(kind string) string {
	switch kind {
	case "HierarchicalRequirement":
		return "User Story"
	case "PortfolioItem/Feature":
		return "Feature"
	case "PortfolioItem/Epic":
		return "Epic"
	case "TestCase":
		return "Test Case"
	case "TestSet":
		return "Test Set"
	case "DefectSuite":
		return "Defect Suite"
	}
	return strings.TrimPrefix(kind, "PortfolioItem/")
}
