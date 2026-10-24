; Inno Setup script for the Windows installer. Built by `cargo xtask dist`, which passes the
; version and the folder assembled in target/dist/Recto:
;   ISCC /DVersion=0.1.0 /DSource=target\dist\Recto /DOutput=target\dist packaging\windows\recto.iss
;
; Installs for the current user by default (no administrator rights), or for everyone when
; chosen. Registers Recto under "Open with" for PDF files without taking over the default
; viewer: Windows lets only the user choose that.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\target\dist\Recto"
#endif
#ifndef Output
  #define Output "..\..\target\dist"
#endif

[Setup]
; Never change: Windows knows the installed app by it.
AppId={{5B0E7A54-1F0B-4C47-9D0E-7C2F3A9E4D21}
AppName=Recto
AppVersion={#Version}
AppVerName=Recto {#Version}
AppPublisher=Evan Pluchart
AppPublisherURL=https://evanpluchart.fr
DefaultDirName={autopf}\Recto
DefaultGroupName=Recto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
OutputDir={#Output}
OutputBaseFilename=Recto-{#Version}-windows-x64-setup
SetupIconFile=..\..\assets\brand\recto.ico
UninstallDisplayIcon={app}\Recto.exe
UninstallDisplayName=Recto
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Every running Recto holds this mutex (decision 0006): Setup waits for them to close rather than
; replacing files in use, and never closes one itself, since it may hold unsaved work.
AppMutex=Local\Recto.Running
CloseApplications=no

[Languages]
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\Recto.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\ocr-models\*"; DestDir: "{app}\ocr-models"; Flags: ignoreversion recursesubdirs
Source: "{#Source}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\Recto"; Filename: "{app}\Recto.exe"
Name: "{autodesktop}\Recto"; Filename: "{app}\Recto.exe"; Tasks: desktopicon

[Registry]
; A program id for PDF files opened with Recto.
Root: HKA; Subkey: "Software\Classes\Recto.PDF"; ValueType: string; ValueName: ""; ValueData: "PDF"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Recto.PDF\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Recto.exe"",0"
Root: HKA; Subkey: "Software\Classes\Recto.PDF\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Recto.exe"" ""%1"""
; Listed under "Open with" for .pdf.
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "Recto.PDF"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\Recto.exe\SupportedTypes"; ValueType: string; ValueName: ".pdf"; ValueData: ""; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Recto.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Recto.exe"" ""%1"""
; Offered in Settings > Default apps.
Root: HKA; Subkey: "Software\Recto"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Recto\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Recto"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Recto\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "PDF viewer and editor"
Root: HKA; Subkey: "Software\Recto\Capabilities\FileAssociations"; ValueType: string; ValueName: ".pdf"; ValueData: "Recto.PDF"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Recto"; ValueData: "Software\Recto\Capabilities"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\Recto.exe"; Description: "{cm:LaunchProgram,Recto}"; Flags: nowait postinstall skipifsilent
; An update from the app (decision 0006) runs silently and asks for Recto to start again.
Filename: "{app}\Recto.exe"; Parameters: "--updated"; Flags: nowait; Check: RelaunchAsked

[Code]
const
  RectoMutex = 'Local\Recto.Running';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{5B0E7A54-1F0B-4C47-9D0E-7C2F3A9E4D21}_is1';

{ Started by Recto's update button. }
function RelaunchAsked(): Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;

{ An update from the app: the Recto that asked quits right after starting this installer, so
  give it a few seconds. Another Recto still open is never closed from here (it may hold
  unsaved work): stop, and start the Recto that asked again, so the reader is not left with
  none. }
function InitializeSetup(): Boolean;
var
  Waited, Code: Integer;
  Location: String;
begin
  Result := True;
  if RelaunchAsked() then
  begin
    Waited := 0;
    while CheckForMutexes(RectoMutex) and (Waited < 10000) do
    begin
      Sleep(250);
      Waited := Waited + 250;
    end;
    Result := not CheckForMutexes(RectoMutex);
    if (not Result) and RegQueryStringValue(HKCU, UninstallKey, 'InstallLocation', Location) then
      Exec(AddBackslash(Location) + 'Recto.exe', '', '', SW_SHOWNORMAL, ewNoWait, Code);
  end;
end;
