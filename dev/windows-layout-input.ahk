#Requires AutoHotkey v2.0
; Run by windows-smoke.ps1 -Layout against local fixtures only.
SetTitleMatchMode 3
WinWait("Fastrock",, 20)
WinActivate("Fastrock")
WinWaitActive("Fastrock",, 10)
CoordMode "Mouse", "Client"

Step(name, action) {
    ready := A_ScriptDir "\layout-" name ".ready"
    Loop 1800 {
        if FileExist(ready)
            break
        Sleep 25
    }
    if !FileExist(ready) {
        FileAppend "Timed out waiting for " name "`n", A_ScriptDir "\layout-driver.log"
        ExitApp 1
    }
    WinActivate("Fastrock")
    WinWaitActive("Fastrock",, 10)
    Sleep 200
    WinGetClientPos ,, &width, &height, "Fastrock"
    FileAppend name " " width "x" height "`n", A_ScriptDir "\layout-driver.log"
    action(width)
    Sleep 300
    MouseGetPos &x, &y, &target
    FileAppend "click " x "," y " on " WinGetTitle("ahk_id " target) "`n", A_ScriptDir "\layout-driver.log"
    MouseMove width - 20, height - 20, 0
    Sleep 500
    FileAppend "", A_ScriptDir "\layout-" name ".done"
}

ClickAt(x, y, count := 1) {
    MouseMove x, y, 0
    Loop count {
        SendEvent "{Click down}"
        Sleep 60
        SendEvent "{Click up}"
        Sleep 100
    }
}

Step("hide", (width) => ClickAt(274, 74))
Step("show", (width) => ClickAt(310, 74))
Step("right", (width) => ClickAt(width - 170, 42))
Step("last", (width) => ClickAt(width - 170, 42, 2))
Step("left", (width) => ClickAt(50, 42))
Step("first", (width) => ClickAt(50, 42, 2))
