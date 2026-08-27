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

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\Kotodex"; Filename: "powershell.exe"; \
    Parameters: "-ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\kotodex\kotodex-windows.ps1"""; \
    WorkingDir: "{app}"; IconFilename: "{app}\kotodex\icons\kotodex.ico"; \
    Comment: "Kotodex - the ledger, the reader and the overlay"
Name: "{autodesktop}\Kotodex"; Filename: "powershell.exe"; \
    Parameters: "-ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\kotodex\kotodex-windows.ps1"""; \
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
    Flags: waituntilterminated
Filename: "powershell.exe"; \
    Parameters: "-ExecutionPolicy Bypass -WindowStyle Hidden -File ""{app}\kotodex\kotodex-windows.ps1"""; \
    WorkingDir: "{app}"; Description: "Start Kotodex"; Flags: postinstall nowait skipifsilent

[UninstallDelete]
; Written after installation by the first run, so Inno does not know about them.
Type: filesandordirs; Name: "{app}\dictionaries"
Type: files; Name: "{app}\system_full.dic"
Type: files; Name: "{app}\setup-server.log"
; The databases and the overlay's storage stay: they are the reading history, and
; an uninstall is not a reason to throw that away. LOCALAPPDATA\kotodex holds them.
