; The Windows installer. Built by build-installer.ps1, which stages the tree
; first - everything here installs one already-assembled directory.
;
;   ISCC.exe /DStage=<dir> /DVersion=<v> /DOut=<dir> kotodex.iss
;
; No administrator: this installs under the user's own LocalAppData, so there is
; no UAC prompt at all. The friend this exists for should not have to think about
; elevation, and nothing here needs to write outside the user's profile - the two
; databases already live in LOCALAPPDATA.

#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppName=Kotodex
; Spelt out rather than left to default to AppName, which is what 0.2.0 shipped
; with: the id is what makes a new version an upgrade of the old one instead of a
; second entry in Installed apps, so it must not move when the name does.
AppId=Kotodex
AppVersion={#Version}
AppPublisher=Kotodex
DefaultDirName={localappdata}\Programs\Kotodex
DefaultGroupName=Kotodex
PrivilegesRequired=lowest
OutputDir={#Out}
OutputBaseFilename=kotodex-{#Version}-windows-setup
; LZMA2/max because most of the payload is Qt and Chromium DLLs, which compress
; hard - it is the difference between a download a friend will start and one he
; will not.
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
DisableProgramGroupPage=yes
UninstallDisplayName=Kotodex
; The .ico rather than an exe: kotodex-server.exe carries no embedded icon, so
; every shortcut pointing at it for one came out blank.
SetupIconFile={#Stage}\kotodex\icons\kotodex.ico
UninstallDisplayIcon={app}\kotodex\icons\kotodex.ico

[InstallDelete]
; An upgrade replaces the two frozen trees wholesale rather than writing over
; them. PyInstaller onedir is about a thousand files, and a freeze against a new
; PySide6 or Python leaves the old DLLs and .pyd files sitting beside the new
; ones, still importable. Nothing else under {app} is touched, so the
; dictionaries and system_full.dic survive and setup.ps1 skips its download.
Type: filesandordirs; Name: "{app}\overlay"
Type: filesandordirs; Name: "{app}\source"

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
; wscript, not powershell: a shortcut to powershell.exe shows a console for as
; long as the launcher runs, and the launcher waits for the server.
Name: "{autoprograms}\Kotodex"; Filename: "wscript.exe"; \
    Parameters: """{app}\kotodex\kotodex-windows.vbs"""; \
    WorkingDir: "{app}"; IconFilename: "{app}\kotodex\icons\kotodex.ico"; \
    Comment: "Kotodex - the ledger, the reader and the overlay"
Name: "{autodesktop}\Kotodex"; Filename: "wscript.exe"; \
    Parameters: """{app}\kotodex\kotodex-windows.vbs"""; \
    WorkingDir: "{app}"; IconFilename: "{app}\kotodex\icons\kotodex.ico"; \
    Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"

[Run]
; The dictionaries and SudachiDict are not redistributed here, so first run
; fetches them - about 175 MB, and the reason this needs a network connection
; once. -NoShortcut because the [Icons] section above owns the shortcuts and the
; uninstaller has to know about them.
Filename: "powershell.exe"; \
    Parameters: "-ExecutionPolicy Bypass -File ""{app}\setup.ps1"" -NoShortcut"; \
    WorkingDir: "{app}"; StatusMsg: "Downloading dictionaries (about 175 MB, once)..."; \
    Flags: waituntilterminated runhidden
Filename: "wscript.exe"; Parameters: """{app}\kotodex\kotodex-windows.vbs"""; \
    WorkingDir: "{app}"; Description: "Start Kotodex"; Flags: postinstall nowait skipifsilent

[Code]
// Windows will not replace a running executable, and Kotodex has no quit that
// stops the server - the overlay's X closes the overlay alone. So an upgrade
// installed over a running copy failed halfway through the tree. Restart Manager
// cannot ask any of these to close either: all three run windowless. Stopping
// them outright is safe - both databases are WAL and survive it, which a
// half-written overlay tree does not.
//
// The overlay and the source first, so neither is left posting to a dead ledger.
procedure StopKotodex;
var
  I, Code: Integer;
  Names: array[0..2] of String;
begin
  Names[0] := 'kotodex-overlay.exe';
  Names[1] := 'kotodex-source.exe';
  Names[2] := 'kotodex-server.exe';
  for I := 0 to 2 do
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ' + Names[I], '',
      SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopKotodex;
  Result := '';
end;

[UninstallDelete]
; Written after installation by the first run, so Inno does not know about them.
Type: filesandordirs; Name: "{app}\dictionaries"
Type: files; Name: "{app}\system_full.dic"
Type: files; Name: "{app}\setup-server.log"
; The databases and the overlay's storage stay: they are the reading history, and
; an uninstall is not a reason to throw that away. LOCALAPPDATA\kotodex holds them.
