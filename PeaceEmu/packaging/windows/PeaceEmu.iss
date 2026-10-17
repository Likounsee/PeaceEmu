#define AppName "PeaceEmu"
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppPublisher "Likounsee"
#define AppExeName "desktop-app.exe"

#ifndef IconFile
  #error "IconFile must point to the generated PeaceEmu.ico."
#endif

#ifndef StageDir
  #error "StageDir must point to the directory containing the staged application executable."
#endif

[Setup]
; Select the installer's UI language from Windows first, falling back to the
; language selector only when the operating-system language is unavailable.
LanguageDetectionMethod=uilanguage
AppId={{D9DDF20D-9F91-4D31-A7C0-8E0F4C1E32D7}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\PeaceEmu
DefaultGroupName=PeaceEmu
DisableProgramGroupPage=yes
OutputBaseFilename=PeaceEmu-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\PeaceEmu.ico
CloseApplications=yes
RestartApplications=no
ArchitecturesInstallIn64BitMode=x64
Uninstallable=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
; Keep the executable as a required source so packaging fails if staging is broken.
Source: "{#StageDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; The shortcut and uninstaller icons must exist inside the installed directory.
Source: "{#IconFile}"; DestDir: "{app}"; DestName: "PeaceEmu.ico"; Flags: ignoreversion
; Bundle staged runtime DLLs, native dependencies, and resources as well. The
; installer must not rely on files from the repository or a developer's PATH.
; Debug symbols are intentionally not shipped.
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist; Excludes: "{#AppExeName},*.pdb,*.ilk"

[Icons]
Name: "{group}\PeaceEmu"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\PeaceEmu.ico"
Name: "{autodesktop}\PeaceEmu"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\PeaceEmu.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch PeaceEmu"; Flags: postinstall nowait skipifsilent

[Code]
var
  RemoveUserSettings: Boolean;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  RemoveUserSettings := False;
  if not UninstallSilent then
    RemoveUserSettings :=
      MsgBox(
        'Do you also want to delete legacy settings, logs and cache? ROMs and files outside the existing settings/log/cache folders will not be deleted.',
        mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

procedure DeleteTemporaryPreferenceFiles(RoamingDir: String);
var
  FindRec: TFindRec;
begin
  { Preference writes use unique names to avoid truncating an existing temp
    file. Remove only files matching that exact app-owned naming convention. }
  if FindFirst(RoamingDir + '\desktop.conf.tmp-*', FindRec) then
  begin
    try
      repeat
        if (FindRec.Name <> '.') and (FindRec.Name <> '..') then
          DeleteFile(RoamingDir + '\' + FindRec.Name);
      until not FindNext(FindRec);
    finally
      FindClose(FindRec);
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  RoamingDir, LocalDir: String;
begin
  if (CurUninstallStep = usPostUninstall) and RemoveUserSettings then
  begin
    RoamingDir := ExpandConstant('{userappdata}\PeaceEmu');
    LocalDir := ExpandConstant('{localappdata}\PeaceEmu');

    { Delete only the known preferences file and known log/cache folders.
      Never recursively remove the user's entire application-data directory. }
    DeleteFile(RoamingDir + '\desktop.conf');
    DeleteFile(RoamingDir + '\desktop.conf.bak');
    DeleteTemporaryPreferenceFiles(RoamingDir);
    { The app falls back to LocalAppData if AppData is unavailable. }
    DeleteFile(LocalDir + '\desktop.conf');
    DeleteFile(LocalDir + '\desktop.conf.bak');
    DeleteTemporaryPreferenceFiles(LocalDir);
    DelTree(RoamingDir + '\logs', True, True, True);
    DelTree(RoamingDir + '\cache', True, True, True);
    DelTree(LocalDir + '\logs', True, True, True);
    DelTree(LocalDir + '\cache', True, True, True);
  end;
end;
