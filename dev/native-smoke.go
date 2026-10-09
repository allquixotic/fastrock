//go:build ignore

// Windows-only native input fixture. Never run on macOS.
// Build: GOOS=windows ... go build -o native-smoke.exe dev/native-smoke.go
package main

import (
	"fmt"
	"image"
	"os"
	"runtime"
	"time"

	"github.com/allquixotic/fastrock/internal/desktop"
	"github.com/allquixotic/fastrock/internal/desktop/label"
)

func main() {
	if runtime.GOOS != "windows" {
		panic("native GUI smoke tests run only on Windows")
	}
	ed := &desktop.TextEditor{Flags: desktop.EditField, Maxlen: 100, Buffer: []rune("Select and replace"), Placeholder: "Type café Native edit test"}
	canvas := &desktop.TextEditor{Flags: desktop.EditBox, Buffer: []rune("Canvas editor stays intact"), Active: true}
	passed, committed := false, false
	var window desktop.MasterWindow
	window = desktop.NewMasterWindowSize(desktop.WindowNoScrollbar, "Fastrock FLTK native smoke", image.Pt(600, 300), func(w *desktop.Window) {
		w.Row(40).Dynamic(1)
		w.Label("Native FLTK field and button", "LC")
		w.Row(40).Dynamic(1)
		if ed.Edit(w)&desktop.EditCommitted != 0 {
			committed = true
		}
		w.Row(40).Dynamic(1)
		if w.Button(label.T("Verify native input"), false) {
			passed = ed.Snapshot() == "café Native edit test" && committed && canvas.Snapshot() == "Canvas editor stays intact"
			window.Close()
		}
		w.Row(50).Dynamic(1)
		w.Label("Current: "+ed.Snapshot(), "LC")
		w.Row(60).Dynamic(1)
		canvas.Edit(w)
	})
	go func() { time.Sleep(45 * time.Second); window.Close() }()
	window.Main()
	if !passed {
		fmt.Printf("FAIL: native field/button: text=%q committed=%v canvas=%q\n", ed.Snapshot(), committed, canvas.Snapshot())
		os.Exit(1)
	}
	fmt.Println("PASS: native FLTK Unicode input, canvas focus isolation, Enter commit and button callback")
}
