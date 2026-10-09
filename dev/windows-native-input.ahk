#Requires AutoHotkey v2.0
; Run beside the Windows-only binary built from dev/native-smoke.go.
savedClipboard := ClipboardAll()
OnExit((*) => A_Clipboard := savedClipboard)
SetTitleMatchMode 3
WinWait("Fastrock FLTK native smoke",, 20)
WinActivate("Fastrock FLTK native smoke")
WinWaitActive("Fastrock FLTK native smoke",, 10)
CoordMode "Mouse", "Client"
Sleep 1500
Click 160, 60
Send "^a"
expected := "caf" Chr(233) " Native edit test"
SendText expected
Sleep 500
Send "{Backspace}t"
Sleep 200
Send "^a"
A_Clipboard := ""
Send "^c"
if !ClipWait(3) || A_Clipboard != expected
    ExitApp 1
Send "{End}{Enter}"
Sleep 500
Click 180, 106
