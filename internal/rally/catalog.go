package rally

// PageSpec ports the reference's navigation, not its fictional browser data.
type PageSpec struct{ ID, Title, Group, Kind, Mode string }

var Pages = []PageSpec{
	{"myrally", "My Rally", "Home", "HierarchicalRequirement", "dashboard"},
	{"backlog", "Backlog", "Plan", "HierarchicalRequirement", "list"},
	{"userstories", "User Stories", "Plan", "HierarchicalRequirement", "list"},
	{"timeboxes", "Timeboxes", "Plan", "Iteration", "list"},
	{"teamplan", "Team Planning", "Plan", "HierarchicalRequirement", "planning"},
	{"workviews", "Work Views", "Plan", "HierarchicalRequirement", "list"},
	{"iterationstatus", "Iteration Status", "Track", "HierarchicalRequirement", "board"},
	{"teamboard", "Team Board", "Track", "HierarchicalRequirement", "board"},
	{"teamstatus", "Team Status", "Track", "HierarchicalRequirement", "dashboard"},
	{"tasks", "Tasks", "Track", "Task", "list"},
	{"defects", "Defects", "Quality", "Defect", "list"},
	{"defectsuites", "Defect Suites", "Quality", "DefectSuite", "list"},
	{"testcases", "Test Cases", "Quality", "TestCase", "list"},
	{"testfolders", "Test Plan", "Quality", "TestFolder", "list"},
	{"qualitymanagement", "Quality Management", "Quality", "TestCase", "dashboard"},
	{"portfolioitemstreegrid", "Portfolio Items", "Portfolio", "PortfolioItem/Feature", "list"},
	{"capacityplanning", "Capacity Planning", "Portfolio", "HierarchicalRequirement", "planning"},
	{"timeline", "Timeline", "Portfolio", "PortfolioItem/Feature", "timeline"},
	{"releasetracking", "FY Quarter Tracking", "Portfolio", "Release", "list"},
	{"portfoliokanban", "Portfolio Kanban", "Portfolio", "PortfolioItem/Feature", "board"},
	{"reports", "Reports", "Reports", "HierarchicalRequirement", "dashboard"},
	{"customreports", "Custom Reports", "Reports", "HierarchicalRequirement", "dashboard"},
	{"insights", "Insights", "Reports", "HierarchicalRequirement", "dashboard"},
	{"customviews", "Custom Views", "Reports", "HierarchicalRequirement", "list"},
	{"projects", "Teams", "Home", "Project", "list"},
	{"users", "Users", "Home", "User", "list"},
	{"mywork", "My Work Items", "Home", "HierarchicalRequirement", "list"},
}

func FindPage(id string) PageSpec {
	for _, p := range Pages {
		if p.ID == id {
			return p
		}
	}
	return Pages[7]
}
func StateField(kind string) string {
	switch kind {
	case "TestCase":
		return "LastVerdict"
	case "Task", "Defect", "PortfolioItem/Feature", "PortfolioItem/Epic", "Iteration", "Release":
		return "State"
	default:
		return "ScheduleState"
	}
}
func States(kind string) []string {
	switch kind {
	case "Task":
		return []string{"Defined", "In-Progress", "Completed"}
	case "PortfolioItem/Feature", "PortfolioItem/Epic":
		return []string{"No Entry", "Product Backlog", "Implementing", "Deployed", "Delivered"}
	default:
		return []string{"Idea", "Defined", "In-Progress", "Completed", "Accepted"}
	}
}
