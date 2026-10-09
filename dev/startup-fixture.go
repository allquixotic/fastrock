//go:build ignore

// Build for Windows as codex.exe in an isolated startup-test directory.
// It checks the parent's native window before servicing either CLI invocation.
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

func main() {
	exe, _ := os.Executable()
	root := filepath.Dir(exe)
	dll := windows.NewLazySystemDLL("user32.dll")
	visible := false
	var pixel uintptr
	dll.NewProc("EnumWindows").Call(syscall.NewCallback(func(hwnd, _ uintptr) uintptr {
		var pid uint32
		dll.NewProc("GetWindowThreadProcessId").Call(hwnd, uintptr(unsafe.Pointer(&pid)))
		shown, _, _ := dll.NewProc("IsWindowVisible").Call(hwnd)
		if pid == uint32(os.Getppid()) && shown != 0 {
			visible = true
			dc, _, _ := dll.NewProc("GetDC").Call(hwnd)
			pixel, _, _ = windows.NewLazySystemDLL("gdi32.dll").NewProc("GetPixel").Call(dc, 20, 20)
			dll.NewProc("ReleaseDC").Call(hwnd, dc)
		}
		return 1
	}), 0)
	record, _ := json.Marshal(map[string]any{"args": os.Args[1:], "parent": os.Getppid(), "visible": visible, "pixel": pixel})
	f, err := os.OpenFile(filepath.Join(root, "invocations.jsonl"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		panic(err)
	}
	_, _ = fmt.Fprintln(f, string(record))
	_ = f.Close()
	b, _ := os.ReadFile(filepath.Join(root, "mode.txt"))
	mode := strings.TrimSpace(string(b))
	if len(os.Args) > 1 && os.Args[1] == "--version" {
		if mode == "blocked" {
			fmt.Fprintln(os.Stderr, "fixture: CLI temporarily blocked during update")
			os.Exit(1)
		}
		if mode == "slow" {
			time.Sleep(time.Minute)
		}
		fmt.Println("codex-cli 0.162.0")
		return
	}
	s := bufio.NewScanner(os.Stdin)
	for s.Scan() {
		var request map[string]any
		if json.Unmarshal(s.Bytes(), &request) != nil || request["id"] == nil {
			continue
		}
		response := map[string]any{"id": request["id"], "result": map[string]any{}}
		if request["method"] == "initialize" && mode == "incompatible" {
			delete(response, "result")
			response["error"] = map[string]any{"code": -32601, "message": "fixture: incompatible while updating"}
		} else {
			switch request["method"] {
			case "model/list", "thread/list", "skills/list", "mcpServerStatus/list", "plugin/list", "hook/list", "config/featureFlags/list":
				response["result"] = map[string]any{"data": []any{}}
			case "config/read":
				response["result"] = map[string]any{"config": map[string]any{}}
			}
		}
		_ = json.NewEncoder(os.Stdout).Encode(response)
	}
}
