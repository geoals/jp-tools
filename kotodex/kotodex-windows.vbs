' Start Kotodex with no console window at all. What the Start Menu shortcut runs.
'
' A shortcut straight to powershell.exe shows a console for as long as the script
' runs, and the launcher waits up to thirty seconds for the server to answer - so
' a reader gets a terminal sitting on screen printing paths at them, which looks
' like something has gone wrong. -WindowStyle Hidden does not fix it: the console
' belongs to powershell.exe and is allocated before the script can hide it.
'
' WScript.Shell's Run with an intWindowStyle of 0 never creates one.
'
' Through cmd so the launcher's own output reaches a file. Hidden, it would
' otherwise go nowhere - and what it says is which components started, and the
' stderr of any that died, which is the first thing worth reading when a reader
' says nothing happened.
'
' q rather than doubled quotes: this line needs quotes inside quotes two levels
' deep, and "" inside a VBScript literal is unreadable enough to get wrong.
Set shell = CreateObject("WScript.Shell")
q = Chr(34)
here = Left(WScript.ScriptFullName, InStrRev(WScript.ScriptFullName, "\"))
log = shell.ExpandEnvironmentStrings("%LOCALAPPDATA%") & "\kotodex\launcher.log"

' cmd /s takes everything between the first and last quote after /c verbatim,
' which is what makes the quoted paths inside it survive.
command = "cmd /d /s /c " & q & _
    "powershell.exe -ExecutionPolicy Bypass -File " & q & here & "kotodex-windows.ps1" & q & _
    " > " & q & log & q & " 2>&1" & q
shell.Run command, 0, False
