// Package ui ports the Codex GUI shell to native nucular widgets.
package ui

import (
	"context"
	"encoding/json"
	"errors"
	"image"
	"image/color"
	"path/filepath"
	"time"

	"github.com/aarzilli/nucular"
	"github.com/aarzilli/nucular/font"
	"github.com/aarzilli/nucular/label"
	"github.com/allquixotic/fastrock/internal/codex"
	"github.com/allquixotic/fastrock/internal/platform"
	"github.com/allquixotic/fastrock/internal/rally"
	"github.com/allquixotic/fastrock/internal/settings"
	"github.com/allquixotic/fastrock/internal/update"
	"github.com/allquixotic/fastrock/internal/workspace"
	"golang.org/x/mobile/event/mouse"
)

type App struct {
	updateStatus     update.Status
	italicFace       font.Face
	navigationFace   font.Face
	draftChats       map[string]bool
	openThreads      []codex.OpenThread
	crossSequence    uint64
	layoutJobs       chan func()
	rallyGeneration  int
	scopeGeneration  int
	importedSessions []string
	transferBuffer   map[string][]codex.Message
	transferPending  bool
	incomingTicket   string
	incomingTab      string
	readyTicket      string
	mailbox          map[string][]mailMessage
	replyWaits       []*replyWait
	crossSends       map[string]int
	drawMax          time.Duration
	drawCount        uint64
	infoViews        map[string]*conversationInfo
	detached         map[string]bool
	recordShortcut   string
	clipboard        string
	connection       Connection
	popping          map[string]bool
	transfers        map[string]string
	sessionName      string
	lastMaintenance  time.Time
	memoryBytes      uint64

	ctx                                               context.Context
	cancel                                            context.CancelFunc
	store                                             *settings.Store
	prefs                                             settings.Preferences
	window                                            nucular.MasterWindow
	p                                                 palette
	state                                             *workspace.State
	client                                            *codex.Client
	catalog                                           codex.Catalog
	updates                                           chan func()
	status, toast, fatal                              string
	exitCode                                          int
	chats                                             map[string]*chatView
	rallyViews                                        map[string]*rallyView
	files                                             map[string]*fileView
	sidebarSearch, newFolder                          *nucular.TextEditor
	archived                                          bool
	collapsed                                         map[string]bool
	settingsView                                      *settingsView
	approvals                                         []approval
	assistant                                         *assistantView
	rallyClient                                       *rally.Client
	workspaces, projects, iterations, releases, users []rally.Object
	rallyErr                                          string
	paletteOpen                                       bool
	paletteSearch                                     *nucular.TextEditor
	writes                                            chan func()
	writerDone                                        chan struct{}
	sessions                                          chan session
	preferences                                       chan settings.Preferences
	lastSession                                       string
	lastCheckpoint                                    time.Time
	historyCursor                                     map[bool]string
	connecting                                        bool
	serverPaused                                      bool
	dragTab                                           string
	dragTabX                                          int
	dragTabMoving                                     bool
	tabWidths                                         []int
	visibleTab                                        string
	sidebarCache                                      sidebarCache
}
type approval struct {
	Choices     []approvalChoice
	Focused     bool
	Armed       time.Time
	URL         string
	Elicitation bool
	Message     codex.Message
	Title       string
	Questions   []question
}
type question struct {
	ID, Header, Text string
	Options          []string
	Descriptions     []string
	Selected         int
	Editor           *nucular.TextEditor
	Secret           bool
	Type             string
	Required         bool
	Form             *elicitationField
}
type fileView struct {
	PendingLine, PendingColumn int
	BasePath                   string
	PendingPosition            *editorPosition
	Offset                     int64
	More                       bool
	LimitReached               bool
	SearchGeneration           int
	Loading                    bool
	Diff                       []diffFile
	Find                       *nucular.TextEditor
	FindOpen, Wrap, Virtual    bool
	Path                       string
	Editor                     *nucular.TextEditor
	Error                      string
}

func Run(ctx context.Context, store *settings.Store, prefs settings.Preferences, connection Connection) int {
	initUIFonts()
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	a := &App{ctx: ctx, cancel: cancel, store: store, prefs: prefs, state: workspace.NewState(), updates: make(chan func(), 512), status: "Starting Codex app-server…", chats: map[string]*chatView{}, rallyViews: map[string]*rallyView{}, files: map[string]*fileView{}, collapsed: map[string]bool{}}
	a.layoutJobs = make(chan func(), 32)
	for range 2 {
		go func() {
			for {
				select {
				case job := <-a.layoutJobs:
					job()
				case <-ctx.Done():
					return
				}
			}
		}()
	}
	a.connection = connection
	a.toast = connection.Notice
	a.client = connection.Client
	a.popping, a.transfers = map[string]bool{}, map[string]string{}
	a.sessionName = "session.json"
	if connection.Ticket != "" {
		a.sessionName = "session-popout-" + connection.Ticket[:min(16, len(connection.Ticket))] + ".json"
	}
	a.writes, a.writerDone = make(chan func(), 128), make(chan struct{})
	a.sessions = make(chan session, 1)
	a.preferences = make(chan settings.Preferences, 1)
	go func() {
		defer close(a.writerDone)
		last := ""
		for {
			select {
			case <-a.ctx.Done():
				return
			case snapshot := <-a.sessions:
				b, _ := json.Marshal(snapshot)
				if string(b) != last {
					last = string(b)
					_ = a.store.WriteJSON(a.sessionName, json.RawMessage(b))
				}
			case p := <-a.preferences:
				if err := a.store.Save(p); err != nil {
					a.post(func() { a.report(err) })
				}
			}
		}
	}()

	if connection.Ticket == "" {
		a.loadSession()
	}
	a.p = colors(prefs.Theme == "light")
	a.italicFace, _ = font.NewFace(uiItalic, prefs.FontSize)
	a.navigationFace, _ = font.NewFace(uiRegular, prefs.FontSize+3)
	a.sidebarSearch = textEditor("", false)
	a.sidebarSearch.Placeholder = "Search conversations"
	a.newFolder = textEditor(prefs.WorkingDirectory, false)
	a.paletteSearch = textEditor("", false)
	if len(a.state.Tabs) == 0 && connection.Ticket == "" {
		a.state.Open(workspace.New, "New tab", "", "")
	}
	a.window = nucular.NewMasterWindowSize(nucular.WindowNoScrollbar, "Fastrock", platform.WindowSize(), a.draw)
	a.window.SetStyle(makeStyle(a.p, prefs.FontSize))

	a.work(func() {
		client := connection.Client
		if connection.Ticket != "" {
			var payload tabTransfer
			e := client.Call(ctx, "fastrock/claim", map[string]string{"ticket": connection.Ticket}, &payload)
			if e != nil {
				a.post(func() { a.fatal = e.Error(); a.exitCode = 1 })
			} else {
				a.post(func() {
					a.incomingTicket = connection.Ticket
					a.transferPending = true
					a.installTransfer(payload)
					if payload.Chat != nil {
						a.transferBuffer = map[string][]codex.Message{payload.Chat.ID: nil}
					}
					a.readyTicket = connection.Ticket
				})
			}
		} else {
			a.post(a.restoreDocuments)
		}
		a.post(func() { a.status = client.Version + " · Connected" })
		a.work(func() {
			var status update.Status
			if client.Call(ctx, "fastrock/update", map[string]bool{}, &status) == nil {
				a.post(func() { a.updateStatus = status })
			}
		})
		go a.consume(client)
		a.loadCatalog(client, prefs.WorkingDirectory)
		a.loadThreads(client, false)
	})
	a.connectRally()
	a.work(func() {
		ticker := time.NewTicker(10 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-a.ctx.Done():
				return
			case <-ticker.C:
				a.window.Changed()
			}
		}
	})
	a.work(func() {
		name := "Fastrock"
		if connection.Ticket != "" {
			name = "Pop-out"
		}
		_ = a.client.Notify("fastrock/name", map[string]string{"name": name})
	})
	a.startAutomation()
	a.window.Main()
	cancel()
	close(a.writes)
	<-a.writerDone
	a.saveSession()
	_ = a.store.Save(a.prefs)
	if a.client != nil {
		a.client.Close()
	}
	return a.exitCode
}
func (a *App) post(f func()) {
	select {
	case a.updates <- f:
		if a.window != nil {
			a.window.Changed()
		}
	case <-a.ctx.Done():
	}
}
func (a *App) work(f func()) { go f() }
func (a *App) report(e error) {
	if e != nil {
		a.toast = e.Error()
	}
}
func (a *App) rpc(method string, params any, done func(json.RawMessage)) {
	a.rpcResult(method, params, done, nil)
}
func (a *App) rpcResult(method string, params any, done func(json.RawMessage), failed func(error)) {
	c := a.client
	if c == nil {
		a.toast = "Codex is still connecting"
		if failed != nil {
			failed(errors.New(a.toast))
		}
		return
	}
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		var raw json.RawMessage
		e := c.Call(ctx, method, params, &raw)
		a.post(func() {
			if e != nil {
				a.report(e)
				if failed != nil {
					failed(e)
				}
				return
			}
			if done != nil {
				done(raw)
			}
		})
	})
}
func (a *App) loadCatalog(c *codex.Client, cwd string) {
	ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
	defer cancel()
	catalog, e := c.Catalog(ctx, cwd)
	a.post(func() {
		if e != nil {
			a.fatal = "Codex protocol is incompatible: " + e.Error()
			a.exitCode = 1
			return
		}
		a.catalog = catalog
	})
}
func (a *App) savePrefs() {
	p := a.prefs
	p.Views = append([]settings.SavedView(nil), p.Views...)
	p.Keymap = cloneKeys(p.Keymap)
	p.RecentFolders = append([]string(nil), p.RecentFolders...)
	if a.client != nil {
		c := a.client
		a.work(func() { _ = c.Notify("fastrock/preferences", map[string]any{"data": p}) })
	}
	select {
	case a.preferences <- p:
	default:
		select {
		case <-a.preferences:
		default:
		}
		select {
		case a.preferences <- p:
		default:
		}
	}

}
func (a *App) theme() {
	a.italicFace, _ = font.NewFace(uiItalic, a.prefs.FontSize)
	a.navigationFace, _ = font.NewFace(uiRegular, a.prefs.FontSize+3)
	if a.prefs.Theme != "light" {
		a.prefs.Theme = "dark"
	}
	a.p = colors(a.prefs.Theme == "light")
	a.window.SetStyle(makeStyle(a.p, a.prefs.FontSize))
	a.savePrefs()
}
func (a *App) openSettings() {
	a.state.Open(workspace.Settings, "Settings", "", "")
	if a.settingsView == nil {
		a.settingsView = newSettingsView(a.prefs)
	}
}
func (a *App) openRally(page string) {
	spec := rally.FindPage(page)
	id := a.state.Open(workspace.Rally, spec.Title, "", page)
	if a.rallyViews[id] == nil {
		v := newRallyView(spec)
		a.rallyViews[id] = v
		a.refreshRally(v)
	}
}
func (a *App) openFile(path string) {
	if !filepath.IsAbs(path) {
		path = filepath.Join(a.prefs.WorkingDirectory, path)
	}
	id := a.state.Open(workspace.File, filepath.Base(path), path, "")
	if a.files[id] != nil {
		return
	}
	v := &fileView{Path: path, Find: textEditor("", false), Wrap: true}
	a.files[id] = v
	a.loadFilePage(v, false)
}
func (a *App) connectRally() {
	a.rallyGeneration++
	generation := a.rallyGeneration
	p := a.prefs
	a.work(func() {
		token, e := a.store.Token(p.RallyEndpoint)
		if e != nil || token == "" {
			a.post(func() {
				if a.rallyGeneration != generation {
					return
				}
				a.rallyErr = "Connect your Rally endpoint and API token in Settings."
			})
			return
		}
		c, e := rally.New(p.RallyEndpoint, token, nil)
		if e != nil {
			a.post(func() {
				if a.rallyGeneration != generation {
					return
				}
				a.rallyErr = e.Error()
			})
			return
		}
		ctx, cancel := context.WithTimeout(a.ctx, 30*time.Second)
		defer cancel()
		workspaces, e := c.All(ctx, "Workspace", rally.Query{})
		if e != nil {
			a.post(func() {
				if a.rallyGeneration != generation {
					return
				}
				a.rallyErr = e.Error()
			})
			return
		}
		a.post(func() {
			if a.rallyGeneration != generation {
				return
			}
			a.rallyClient = c
			a.rallyErr = ""
			a.workspaces = workspaces
			if a.prefs.RallyWorkspace == "" && len(workspaces) > 0 {
				a.prefs.RallyWorkspace = workspaces[0].String("_ref")
			}
			a.loadScope()
			for _, v := range a.rallyViews {
				a.refreshRally(v)
			}
		})
	})
}
func (a *App) loadScope() {
	c := a.rallyClient
	if c == nil {
		return
	}
	p := a.prefs
	a.scopeGeneration++
	generation := a.scopeGeneration
	a.work(func() {
		ctx, cancel := context.WithTimeout(a.ctx, 45*time.Second)
		defer cancel()
		q := rally.Query{Workspace: p.RallyWorkspace, Project: p.RallyProject, Children: p.ProjectChildren, Parents: p.ProjectParents}
		projects, e := c.All(ctx, "Project", rally.Query{Workspace: p.RallyWorkspace})
		iterations, e2 := c.All(ctx, "Iteration", q)
		releases, e3 := c.All(ctx, "Release", q)
		users, e4 := c.All(ctx, "User", rally.Query{Workspace: p.RallyWorkspace})
		a.post(func() {
			if a.scopeGeneration != generation || a.rallyClient != c {
				return
			}
			a.projects = projects
			a.iterations = iterations
			a.releases = releases
			a.users = users
			for _, err := range []error{e, e2, e3, e4} {
				if err != nil {
					a.report(err)
					break
				}
			}
		})
	})
}
func (a *App) draw(w *nucular.Window) {
	started := time.Now()
	defer func() {
		elapsed := time.Since(started)
		a.drawCount++
		if elapsed > a.drawMax {
			a.drawMax = elapsed
		}
	}()
	deadline := time.Now().Add(4 * time.Millisecond)
	for i := 0; i < 64 && time.Now().Before(deadline); i++ {
		select {
		case f := <-a.updates:
			f()
		default:
			i = 64
		}
	}
	if len(a.updates) > 0 {
		a.window.Changed()
	}
	if a.clipboard != "" {
		w.SetClipboard(a.clipboard)
		a.clipboard = ""
	}
	a.shortcuts(w)
	a.checkpoint()
	a.maintain()
	if a.fatal != "" {
		w.Row(100).Dynamic(1)
		w.Spacing(1)
		title(w, "Fastrock cannot start", a.p)
		w.Row(140).Dynamic(1)
		w.LabelWrap(a.fatal)
		w.Row(30).Static(160)
		if w.ButtonText("Exit") {
			a.window.Close()
		}
		return
	}
	a.drawMenu(w)
	a.drawTabs(w)
	if a.toast != "" {
		w.Row(28).Ratio(.92, .08)
		w.LabelColored(a.toast, "LC", a.p.Warning)
		if w.ButtonText("Dismiss") {
			a.toast = ""
		}
	}
	footerHeight := 0
	if a.prefs.StatusBar {
		footerHeight = 25
	}
	h := max(240, w.LayoutAvailableHeight()-footerHeight)
	side := 0
	if a.prefs.Sidebar {
		side = 255
	}
	info := 0
	if a.prefs.Info && a.state.Current() != nil && a.state.Current().Kind == workspace.Chat {
		info = 255
	}
	widths := []int{}
	if side > 0 {
		widths = append(widths, side)
	}
	widths = append(widths, max(360, w.LayoutAvailableWidth()-side-info))
	if info > 0 {
		widths = append(widths, info)
	}
	w.Row(h).Static(widths...)
	if side > 0 {
		if sw := w.GroupBegin("conversation-sidebar", nucular.WindowNoHScrollbar); sw != nil {
			a.drawSidebar(sw)
			sw.GroupEnd()
		}
	}
	if body := w.GroupBegin("document", nucular.WindowNoScrollbar); body != nil {
		t := a.state.Current()
		if a.transferPending || t != nil && a.popping[t.ID] {
			muted(body, "Moving this tab to its new window…", a.p)
		} else if t == nil {
			a.drawNew(body)
		} else {
			switch t.Kind {
			case workspace.New:
				a.drawNew(body)
			case workspace.Chat:
				if a.chats[t.Target] == nil && a.client != nil && !a.serverPaused {
					a.resumeThread(t.Target)
				}
				a.drawChat(body, t.Target)
			case workspace.Rally:
				a.drawRally(body, a.rallyViews[t.ID])
			case workspace.Settings:
				a.drawSettings(body)
			case workspace.File:
				if f := a.files[t.ID]; f != nil && f.Editor == nil && f.Error == "" {
					a.reloadFile(f)
				}
				a.drawFile(body, a.files[t.ID])
			}
		}
		body.GroupEnd()
	}
	if info > 0 {
		if iw := w.GroupBegin("conversation-info", nucular.WindowNoHScrollbar); iw != nil {
			a.drawInfo(iw)
			iw.GroupEnd()
		}
	}
	if a.prefs.StatusBar {
		w.Row(25).Static(max(100, w.LayoutAvailableWidth()-28), 28)
		w.LabelColored(a.status, "LC", a.p.Muted)
		if iconButton(w, "close", false, a.p) {
			a.prefs.StatusBar = false
			a.savePrefs()
		}
	}
	if a.paletteOpen {
		a.drawPalette()
	}
	if ticket := a.readyTicket; ticket != "" {
		a.readyTicket = ""
		a.rpc("fastrock/ready", map[string]string{"ticket": ticket}, nil)
	}
}
func (a *App) drawTabs(w *nucular.Window) {
	settingsWidth := nucular.FontWidth(w.Master().Style().Font, "Settings") + 20
	w.Row(38).Static(36, max(100, w.LayoutAvailableWidth()-96-settingsWidth), 30, 30, settingsWidth)
	if iconButton(w, "sidebar", a.prefs.Sidebar, a.p) {
		a.prefs.Sidebar = !a.prefs.Sidebar
		a.savePrefs()
	}
	oldGroup := w.Master().Style().GroupWindow
	w.Master().Style().GroupWindow.Padding = image.Pt(0, 2)
	w.Master().Style().GroupWindow.Spacing = image.Pt(0, 0)
	if strip := w.GroupBegin("tabs", nucular.WindowNoScrollbar); strip != nil {
		count := len(a.state.Tabs)
		width := min(240, max(84, strip.LayoutAvailableWidth()/max(1, count)))
		if cap(a.tabWidths) < count {
			a.tabWidths = make([]int, count)
		} else {
			a.tabWidths = a.tabWidths[:count]
		}
		for i := range a.tabWidths {
			a.tabWidths[i] = width
		}
		maxScroll := max(0, count*width-strip.LayoutAvailableWidth())
		if in := strip.Input(); in.Mouse.HoveringRect(strip.Bounds) && (in.Mouse.ScrollDelta != 0 || in.Mouse.ScrollDeltaX != 0) {
			strip.Scrollbar.X = min(maxScroll, max(0, strip.Scrollbar.X+int(in.Mouse.ScrollDeltaX-in.Mouse.ScrollDelta)*60))
		}
		if a.visibleTab != a.state.Active {
			for i, t := range a.state.Tabs {
				if t.ID == a.state.Active {
					left, right := i*width, (i+1)*width
					if left < strip.Scrollbar.X {
						strip.Scrollbar.X = left
					}
					if right > strip.Scrollbar.X+strip.LayoutAvailableWidth() {
						strip.Scrollbar.X = right - strip.LayoutAvailableWidth()
					}
				}
			}
			a.visibleTab = a.state.Active
		}
		strip.Scrollbar.X = min(maxScroll, max(0, strip.Scrollbar.X))
		strip.Row(34).Static(a.tabWidths...)
		closeID, keepID, moveID := "", "", ""
		keepRight, moveBy := -1, 0
		wasDragging := a.dragTabMoving
		for i, t := range a.state.Tabs {
			var dot color.RGBA
			if c := a.state.Chats[t.Target]; c != nil {
				dot = a.p.Faint
				if c.Busy() {
					dot = a.p.Accent
				}
				if c.Status == "error" {
					dot = a.p.Danger
				}
			}
			activate, close, b := documentTab(strip, t.Title, t.ID == a.state.Active, dot, a.p)
			in := strip.Input()
			if in.Mouse.IsClickDownInRect(mouse.ButtonLeft, b, true) {
				a.dragTab = t.ID
				a.dragTabX = in.Mouse.Pos.X
			}
			if a.dragTab == t.ID && in.Mouse.Down(mouse.ButtonLeft) && absInt(in.Mouse.Pos.X-a.dragTabX) > 6 {
				a.dragTabMoving = true
			}
			if a.dragTabMoving && in.Mouse.Released(mouse.ButtonLeft) && in.Mouse.HoveringRect(b) {
				from := 0
				for j := range a.state.Tabs {
					if a.state.Tabs[j].ID == a.dragTab {
						from = j
					}
				}
				moveID, moveBy = a.dragTab, i-from
			}
			if b.W > 0 && b.H > 0 {
				if menu := strip.ContextualOpen(0, image.Pt(220, 490), b, nil); menu != nil {
					menu.Row(28).Dynamic(1)
					if menu.MenuItem(label.T("Pop out into new window")) {
						a.popOut(t)
					}
					if menu.MenuItem(label.T("Move to another window…")) {
						a.moveTab(t)
					}
					if t.Kind == workspace.Chat {
						a.chatTabMenu(menu, t)
					}
					if menu.MenuItem(label.T("Close tab")) {
						closeID = t.ID
					}
					if menu.MenuItem(label.T("Close other tabs")) {
						keepID = t.ID
					}
					if menu.MenuItem(label.T("Close tabs to right")) {
						keepRight = i
					}
				}
			}
			if activate && !a.dragTabMoving && !wasDragging {
				a.state.Active = t.ID
			}
			if close {
				closeID = t.ID
			}
		}
		if moveID != "" {
			a.state.Move(moveID, moveBy)
			a.state.Active = moveID
		}
		var closing []string
		if keepID != "" {
			for _, t := range a.state.Tabs {
				if t.ID != keepID {
					closing = append(closing, t.ID)
				}
			}
			a.state.Active = keepID
		} else if keepRight >= 0 {
			for _, t := range a.state.Tabs[keepRight+1:] {
				closing = append(closing, t.ID)
			}
		}
		if len(closing) > 0 {
			a.closeTabs(closing)
		}

		if closeID != "" {
			a.closeTab(closeID)
		}
		if strip.Input().Mouse.Released(mouse.ButtonLeft) {
			a.dragTab = ""
			a.dragTabMoving = false
		}
		strip.GroupEnd()
	}
	w.Master().Style().GroupWindow = oldGroup
	if iconButton(w, "plus", false, a.p) {
		a.state.Open(workspace.New, "New tab", "", "")
	}
	if iconButton(w, "info", a.prefs.Info, a.p) {
		a.prefs.Info = !a.prefs.Info
		a.savePrefs()
	}
	if w.ButtonText("Settings") {
		a.openSettings()
	}
}
func absInt(n int) int {
	if n < 0 {
		return -n
	}
	return n
}
func (a *App) drawNew(w *nucular.Window) {
	w.Row(60).Dynamic(1)
	w.Spacing(1)
	title(w, "What would you like to work on?", a.p)
	muted(w, "Start a Codex conversation or open a Rally workspace.", a.p)
	w.Row(30).Dynamic(4)
	if w.ButtonText("Choose folder…") {
		a.choosePath(false, true, func(path string) { setText(a.newFolder, path); a.prefs.WorkingDirectory = path; a.savePrefs() })
	}
	if w.ButtonText("Folderless chat") {
		a.newThread("")
	}
	if w.ButtonText("Open file…") {
		a.choosePath(false, false, a.openFile)
	}
	if w.ButtonText("Resume chat…") {
		a.chooseConversation(func(c *workspace.Conversation) { a.resumeThread(c.ID) })
	}
	title(w, "Project folder", a.p)
	w.Row(32).Ratio(.8, .2)
	a.newFolder.Edit(w)
	if primary(w, "New conversation", a.p) {
		a.newThread(text(a.newFolder))
	}
	title(w, "Rally workspace", a.p)
	w.Row(34).Dynamic(4)
	for _, id := range []string{"teamboard", "backlog", "portfolioitemstreegrid", "reports"} {
		p := rally.FindPage(id)
		if w.ButtonText(p.Title) {
			a.openRally(id)
		}
	}
	if a.rallyErr != "" {
		muted(w, a.rallyErr, a.p)
		w.Row(30).Static(180)
		if w.ButtonText("Configure Rally") {
			a.openSettings()
		}
	}
	w.Row(30).Static(180)
	if w.ButtonText("Sign in to Codex…") {
		a.settingsPage("Account")
	}
	title(w, "Recent folders", a.p)
	for _, folder := range a.prefs.RecentFolders {
		w.Row(30).Dynamic(1)
		if w.ButtonText(folder) {
			setText(a.newFolder, folder)
			a.newThread(folder)
		}
		if menu := w.ContextualOpen(0, image.Pt(210, 120), w.LastWidgetBounds, nil); menu != nil {
			if menu.MenuItem(label.T("Open folder")) {
				a.openPath(folder, false)
			}
			if menu.MenuItem(label.T("Copy path")) {
				a.copyText(folder)
			}
			if menu.MenuItem(label.T("Forget folder")) {
				a.forgetFolder(folder)
			}
		}
	}
	title(w, "Recent conversations", a.p)
	rows := a.state.Sidebar("", false)
	for _, c := range rows[:min(8, len(rows))] {
		w.Row(32).Dynamic(1)
		if w.ButtonText(c.Title + "  ·  " + filepath.Base(c.Cwd)) {
			a.resumeThread(c.ID)
		}
	}
}
