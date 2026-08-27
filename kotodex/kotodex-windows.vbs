' Start Kotodex with no console window at all. What the Start Menu shortcut runs.
'
' A shortcut straight to powershell.exe shows a console for as long as the script
' runs, and the launcher waits up to thirty seconds for the server to answer - so
' a reader gets a terminal sitting on screen printing paths at them, which looks
' like something has gone wrong. -WindowStyle Hidden does not fix it: the console
' belongs to powershell.exe and is allocated before the script can hide it.
'
' WScript.Shell's Run with an intWindowStyle of 0 never creates one.
Set shell = CreateObject("WScript.Shell")
here = Left(WScript.ScriptFullName, InStrRev(WScript.ScriptFullName, "\"))
shell.Run "powershell.exe -ExecutionPolicy Bypass -File """ & here & "kotodex-windows.ps1""", 0, False
