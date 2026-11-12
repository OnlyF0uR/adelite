; Adelite's installer for Windows (Inno Setup 6): for the one person, with no
; administrator asked, into %LOCALAPPDATA%\Programs\Adelite.
;
; The release workflow gives it the version, the folder holding adelite.exe and
; the DLLs that go beside it, and the icon packaging/windows/make-ico.ps1 made:
;
;   iscc /DAppVersion=1.2.3 /DStagingDir=C:\path\to\stage /DSetupIcon=C:\path\to\adelite.ico packaging\windows\adelite.iss
;
; Adelite's updater runs it with /VERYSILENT /SUPPRESSMSGBOXES /NORESTART as the
; old Adelite exits, and a silent install starts the new one with --after-update.

#ifndef AppVersion
  #error Give the version: /DAppVersion=X.Y.Z
#endif
#ifndef StagingDir
  #error Give the folder holding adelite.exe and its DLLs: /DStagingDir=...
#endif

[Setup]
; Never changes: Windows, and every later installer, know the installed Adelite by it.
AppId={{9B049FFB-2457-41B6-9A28-0351B09E0E8D}
AppName=Adelite
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersion}
; Per user, so neither installing nor updating asks for an administrator;
; {autopf} is then %LOCALAPPDATA%\Programs.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Adelite
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern
; The updater has Adelite exit by itself, and PrepareToInstall below waits for
; it; Windows' Restart Manager must not close it halfway.
CloseApplications=no
UninstallDisplayName=Adelite
UninstallDisplayIcon={app}\adelite.exe
OutputBaseFilename=Adelite-{#AppVersion}-windows-x64-setup
#ifdef SetupIcon
SetupIconFile={#SetupIcon}
#endif
Compression=lzma2/max
SolidCompression=yes

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; adelite.exe first: should it still be in use, setup stops before it has
; replaced a DLL the running one has loaded.
Source: "{#StagingDir}\adelite.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StagingDir}\*.dll"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Adelite"; Filename: "{app}\adelite.exe"
Name: "{autodesktop}\Adelite"; Filename: "{app}\adelite.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\adelite.exe"; Description: "Start Adelite"; Flags: nowait postinstall skipifsilent
; An update is silent: bring Adelite back, and tell it why.
Filename: "{app}\adelite.exe"; Parameters: "--after-update"; Flags: nowait; Check: WizardSilent

; Uninstalling removes only what was installed above. What Adelite keeps (in
; %USERPROFILE%\.adelite) is not touched, so nothing is lost by reinstalling.

[Code]
const
  WaitMs = 30000;
  PollMs = 250;

{ True when nobody has the file open: Windows keeps a running program's
  executable from being opened for writing. }
function Unlocked(const Path: String): Boolean;
var
  Stream: TFileStream;
begin
  Result := True;
  try
    Stream := TFileStream.Create(Path, fmOpenReadWrite or fmShareExclusive);
    Stream.Free;
  except
    Result := False;
  end;
end;

{ The updater starts this setup as Adelite exits, and Adelite takes a moment
  to close its session: give it up to WaitMs, then go on either way. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Waited: Integer;
begin
  Result := '';
  Exe := ExpandConstant('{app}\adelite.exe');
  Waited := 0;
  while FileExists(Exe) and not Unlocked(Exe) and (Waited < WaitMs) do begin
    Sleep(PollMs);
    Waited := Waited + PollMs;
  end;
end;
