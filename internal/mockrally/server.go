// Package mockrally is a loopback-only, in-memory WSAPI fixture. The response
// shapes follow the httptest fixtures in allquixotic/fastrally's Rally client.
package mockrally

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/allquixotic/fastrock/internal/rally"
)

type Server struct {
	mu       sync.Mutex
	Objects  map[string]rally.Object
	Next     int
	Token    string
	Requests int
}

func New() *Server {
	s := &Server{Objects: map[string]rally.Object{}, Next: 1000, Token: "mock-token"}
	add := func(kind string, id int, fields rally.Object) rally.Object {
		ref := rally.WSAPI + strings.ToLower(kind) + "/" + strconv.Itoa(id)
		fields["_ref"] = ref
		fields["_type"] = kind
		fields["ObjectID"] = id
		fields["LastUpdateDate"] = "2026-10-01T12:00:00Z"
		s.Objects[ref] = fields
		return fields
	}
	ws := add("Workspace", 1, rally.Object{"Name": "Sample Workspace"})
	workspace := map[string]any{"_ref": ws["_ref"], "_refObjectName": ws["Name"]}
	teams := []rally.Object{}
	for i, name := range []string{"Sample Delivery Team", "Sample Platform Team", "Sample Experience Team"} {
		p := add("Project", i+10, rally.Object{"Name": name, "State": "Open", "Workspace": workspace})
		teams = append(teams, p)
	}
	owners := []rally.Object{}
	for i, name := range []string{"Alex Morgan", "Jordan Lee", "Casey Rivera", "Taylor Quinn"} {
		o := add("User", i+20, rally.Object{"Name": name, "DisplayName": name, "UserName": strings.ReplaceAll(strings.ToLower(name), " ", ".") + "@example.invalid"})
		owners = append(owners, o)
	}
	reference := func(o rally.Object) map[string]any {
		return map[string]any{"_ref": o["_ref"], "_refObjectName": o["Name"], "ObjectID": o["ObjectID"]}
	}
	iterations := []rally.Object{}
	for i := range 2 {
		it := add("Iteration", i+30, rally.Object{"Name": fmt.Sprintf("Iteration %d", i+12), "StartDate": fmt.Sprintf("2026-10-%02d", 1+i*14), "EndDate": fmt.Sprintf("2026-10-%02d", 14+i*14), "Project": reference(teams[0]), "Workspace": workspace, "State": "Planning"})
		iterations = append(iterations, it)
	}
	release := add("Release", 40, rally.Object{"Name": "Sample Release 1", "State": "Defined", "Project": reference(teams[0]), "Workspace": workspace})
	featureNames := []string{"A clearer project overview", "Flexible team planning", "Accessible work-item editing", "Reliable notifications", "Quality at every step"}
	features := []rally.Object{}
	for i, name := range featureNames {
		o := add("PortfolioItem/Feature", 50+i, rally.Object{"Name": name, "FormattedID": fmt.Sprintf("F%d", 101+i), "State": map[string]any{"_ref": rally.WSAPI + "state/91", "_refObjectName": "Implementing"}, "Project": reference(teams[i%3]), "Workspace": workspace, "Owner": reference(owners[i%4]), "PlannedStartDate": "2026-10-01", "PlannedEndDate": "2026-10-31"})
		features = append(features, o)
	}
	for i, name := range []string{"Product Backlog", "Implementing", "Deployed", "Delivered"} {
		add("State", 90+i, rally.Object{"Name": name})
	}
	names := []string{"See my assigned work in one place", "Save a personal team-board view", "Filter work by owner and iteration", "Create a story from the backlog", "Reorder backlog priorities", "View project health at a glance", "Add an attachment to a work item", "Receive a notification when assigned", "Discuss acceptance criteria with my team", "Plan the next iteration with capacity", "Compare planned and delivered points", "Edit a story using only the keyboard", "Link test cases to acceptance criteria", "Group stories by their parent feature", "See a complete revision history", "Customize notification preferences", "Review a release's remaining work", "Run a regression test set", "Search for work by ID or title", "Share a planning view with the team"}
	for i, name := range names {
		states := []string{"In-Progress", "Defined", "Accepted", "Completed", "In-Progress", "Defined"}
		id := 100 + i
		ref := rally.WSAPI + "hierarchicalrequirement/" + strconv.Itoa(id)
		add("HierarchicalRequirement", id, rally.Object{"Name": name, "FormattedID": fmt.Sprintf("US%d", 1001+i), "Description": "<p>As a <strong>team member</strong>, I want a <em>clear and predictable workflow</em> so I can focus on delivering useful work.</p><ul><li>Search work by owner and iteration</li><li>Review <u>acceptance criteria</u> before completion</li></ul><p><a href=\"https://example.com\">Project documentation</a></p>", "Notes": "Fictional test data. No production Rally account is used.", "AcceptanceCriteria": "Given the work item is open, when the change is saved, the updated values are visible.", "ScheduleState": states[i%len(states)], "PlanEstimate": []int{5, 3, 3, 5, 3, 8}[i%6], "Blocked": i == 4, "Ready": i%2 == 0, "Workspace": workspace, "Project": reference(teams[0]), "Owner": reference(owners[i%4]), "Iteration": reference(iterations[i%2]), "Release": reference(release), "Feature": reference(features[i%5]), "Tasks": map[string]any{"_ref": ref + "/tasks", "Count": 1}, "Discussion": map[string]any{"_ref": ref + "/discussion", "Count": 0}, "Attachments": map[string]any{"_ref": ref + "/attachments", "Count": 0}, "RevisionHistory": map[string]any{"_ref": rally.WSAPI + "revisionhistory/" + strconv.Itoa(id)}, "Rank": i, "c_TeamNote": "Ready for review"})
	}
	for i, name := range []string{"Owner filter resets after changing a view", "Long story titles overlap the point badge", "Notification count includes read messages", "Date picker skips the final day of October"} {
		add("Defect", 200+i, rally.Object{"Name": name, "FormattedID": fmt.Sprintf("DE%d", 201+i), "State": "Open", "ScheduleState": "Defined", "PlanEstimate": 2, "Project": reference(teams[0]), "Workspace": workspace, "Owner": reference(owners[i%4]), "Priority": "Normal", "Severity": "Minor Problem"})
	}
	for i := range 5 {
		add("Task", 300+i, rally.Object{"Name": "Implement and verify " + names[i], "FormattedID": fmt.Sprintf("TA%d", 301+i), "State": "Defined", "Estimate": 4, "ToDo": 3, "Actuals": 1, "Project": reference(teams[0]), "Workspace": workspace, "Owner": reference(owners[i%4]), "WorkProduct": rally.WSAPI + "hierarchicalrequirement/" + strconv.Itoa(100+i)})
	}
	for i, kind := range []string{"TestCase", "TestFolder", "TestSet", "DefectSuite", "PortfolioItem/Epic"} {
		add(kind, 400+i, rally.Object{"Name": "Sample " + kind, "FormattedID": fmt.Sprintf("%s%d", strings.ToUpper(kind[:2]), i+101), "Project": reference(teams[0]), "Workspace": workspace, "ScheduleState": "Defined", "LastVerdict": "Pass"})
	}
	return s
}
func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.Requests++
	w.Header().Set("Content-Type", "application/json")
	write := func(v any) { _ = json.NewEncoder(w).Encode(v) }
	if r.Header.Get("ZSESSIONID") != s.Token {
		w.WriteHeader(http.StatusUnauthorized)
		write(map[string]any{"OperationResult": map[string]any{"Errors": []string{"Invalid token"}}})
		return
	}
	if !strings.HasPrefix(r.URL.Path, rally.WSAPI) {
		http.NotFound(w, r)
		return
	}
	ref := r.URL.Path
	tail := strings.TrimPrefix(ref, rally.WSAPI)
	if strings.HasSuffix(tail, "/create") && r.Method == "POST" {
		kind := strings.TrimSuffix(tail, "/create")
		canonical, _ := rally.CanonicalKind(kind)
		var body map[string]rally.Object
		if json.NewDecoder(r.Body).Decode(&body) != nil {
			w.WriteHeader(400)
			return
		}
		var fields rally.Object
		for _, f := range body {
			fields = f
		}
		s.Next++
		fields["ObjectID"] = s.Next
		fields["_type"] = canonical
		fields["_ref"] = rally.WSAPI + kind + "/" + strconv.Itoa(s.Next)
		fields["FormattedID"] = fmt.Sprintf("NEW%d", s.Next)
		fields["LastUpdateDate"] = time.Now().UTC().Format(time.RFC3339Nano)
		s.Objects[fields.String("_ref")] = fields
		write(map[string]any{"CreateResult": map[string]any{"Object": fields, "Errors": []string{}, "Warnings": []string{}}})
		return
	}
	if o, ok := s.Objects[ref]; ok {
		switch r.Method {
		case "GET":
			write(map[string]any{path.Base(o.Kind()): o})
		case "DELETE":
			delete(s.Objects, ref)
			write(map[string]any{"OperationResult": map[string]any{"Errors": []string{}}})
		case "POST":
			var body map[string]rally.Object
			if json.NewDecoder(r.Body).Decode(&body) != nil {
				w.WriteHeader(400)
				return
			}
			for _, f := range body {
				for k, v := range f {
					o[k] = v
				}
			}
			o["LastUpdateDate"] = time.Now().UTC().Format(time.RFC3339Nano)
			write(map[string]any{"OperationResult": map[string]any{"Object": o, "Errors": []string{}, "Warnings": []string{}}})
		default:
			w.WriteHeader(405)
		}
		return
	}
	if r.Method != "GET" {
		w.WriteHeader(404)
		return
	}
	rows := []rally.Object{}
	if tail == "typedefinition" {
		kind := extractLiteral(r.URL.Query().Get("query"))
		rows = []rally.Object{{"ObjectID": 800, "TypePath": kind, "Attributes": map[string]any{"_ref": rally.WSAPI + "typedefinition/800/attributes"}}}
	} else if tail == "typedefinition/800/attributes" {
		for _, f := range []struct{ name, typ string }{{"Name", "STRING"}, {"Description", "TEXT"}, {"PlanEstimate", "DECIMAL"}, {"c_TeamNote", "STRING"}} {
			rows = append(rows, rally.Object{"ElementName": f.name, "Name": f.name, "AttributeType": f.typ, "Required": f.name == "Name", "ReadOnly": false})
		}
	} else if strings.HasSuffix(tail, "/tasks") {
		parent := strings.TrimSuffix(ref, "/tasks")
		for _, o := range s.Objects {
			if o.Kind() == "Task" && o.Ref("WorkProduct") == parent {
				rows = append(rows, o)
			}
		}
	} else {
		for _, o := range s.Objects {
			if strings.EqualFold(o.Kind(), tail) && matches(o, r.URL.Query().Get("query")) {
				if workspace := r.URL.Query().Get("workspace"); workspace != "" && o.Ref("Workspace") != "" && o.Ref("Workspace") != workspace {
					continue
				}
				if project := r.URL.Query().Get("project"); project != "" && o.Ref("Project") != "" && o.Ref("Project") != project {
					continue
				}
				rows = append(rows, o)
			}
		}
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].Number("ObjectID") < rows[j].Number("ObjectID") })
	total := len(rows)
	start, _ := strconv.Atoi(r.URL.Query().Get("start"))
	start = max(1, start)
	pageSize, _ := strconv.Atoi(r.URL.Query().Get("pagesize"))
	if pageSize < 1 {
		pageSize = 200
	}
	offset := min(start-1, total)
	rows = rows[offset:min(offset+pageSize, total)]
	write(map[string]any{"QueryResult": map[string]any{"Results": rows, "StartIndex": start, "PageSize": pageSize, "TotalResultCount": total, "Errors": []string{}, "Warnings": []string{}}})
}

var clause = regexp.MustCompile(`([A-Za-z_.]+)\s*(=|!=|contains)\s*("(?:[^"\\]|\\.)*"|true|false|null|[0-9]+)`)

// The fixture evaluates nested AND/OR queries, including quoted parentheses.
func matches(o rally.Object, q string) bool {
	q = strings.TrimSpace(q)
	if q == "" {
		return true
	}
	for strings.HasPrefix(q, "(") && matchingParen(q) == len(q)-1 {
		q = strings.TrimSpace(q[1 : len(q)-1])
	}
	for _, op := range []string{" OR ", " AND "} {
		if i := booleanSplit(q, op); i >= 0 {
			if op == " OR " {
				return matches(o, q[:i]) || matches(o, q[i+len(op):])
			}
			return matches(o, q[:i]) && matches(o, q[i+len(op):])
		}
	}
	m := clause.FindStringSubmatch(q)
	if m == nil {
		return false
	}
	value := m[3]
	if strings.HasPrefix(value, `"`) {
		_ = json.Unmarshal([]byte(value), &value)
	}
	key := m[1]
	actual := o.String(key)
	if strings.HasSuffix(key, ".Name") {
		actual = o.String(strings.TrimSuffix(key, ".Name"))
	}
	if value == "null" {
		actual = o.Ref(key)
		value = ""
	}
	if ref := o.Ref(key); strings.Contains(value, "/") && ref != "" {
		actual = ref
	}
	switch m[2] {
	case "=":
		return actual == value
	case "!=":
		return actual != value
	case "contains":
		return strings.Contains(strings.ToLower(actual), strings.ToLower(value))
	}
	return false
}
func booleanSplit(q, op string) int {
	depth := 0
	quoted, escape := false, false
	for i := 0; i < len(q); i++ {
		c := q[i]
		if escape {
			escape = false
			continue
		}
		if quoted && c == '\\' {
			escape = true
			continue
		}
		if c == '"' {
			quoted = !quoted
			continue
		}
		if quoted {
			continue
		}
		if c == '(' {
			depth++
		}
		if c == ')' {
			depth--
		}
		if depth == 0 && strings.HasPrefix(q[i:], op) {
			return i
		}
	}
	return -1
}
func matchingParen(q string) int {
	depth := 0
	quoted, escape := false, false
	for i := 0; i < len(q); i++ {
		c := q[i]
		if escape {
			escape = false
			continue
		}
		if quoted && c == '\\' {
			escape = true
			continue
		}
		if c == '"' {
			quoted = !quoted
			continue
		}
		if quoted {
			continue
		}
		if c == '(' {
			depth++
		}
		if c == ')' {
			depth--
			if depth == 0 {
				return i
			}
		}
	}
	return -1
}
func extractLiteral(q string) string {
	m := clause.FindStringSubmatch(q)
	if m == nil {
		return ""
	}
	var s string
	_ = json.Unmarshal([]byte(m[3]), &s)
	return s
}

// Large is a deterministic fixture for virtual scrolling and memory stress.
func Large(stories int) *Server {
	s := New()
	base := s.Objects[rally.WSAPI+"hierarchicalrequirement/100"]
	for i := 20; i < stories; i++ {
		o := base.Clone()
		id := 10000 + i
		ref := rally.WSAPI + "hierarchicalrequirement/" + strconv.Itoa(id)
		o["_ref"], o["ObjectID"], o["FormattedID"], o["Name"], o["Rank"] = ref, id, fmt.Sprintf("US%d", 20000+i), fmt.Sprintf("Large fixture story %d", i), i
		o["ScheduleState"] = []string{"Defined", "In-Progress", "Completed", "Accepted"}[i%4]
		s.Objects[ref] = o
	}
	return s
}
