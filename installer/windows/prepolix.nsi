; Windows-Installer für prepolix, gebaut mit NSIS 3 (makensis, läuft auch unter Linux).
; Normalerweise ruft scripts/build_windows_installer.sh das Skript auf und übergibt:
;   VERSION  Programmversion, z. B. 0.1.0
;   STAGE    Ordner mit allen Dateien der Installation
;   OUTFILE  Pfad des fertigen Installers

Unicode true
SetCompressor /SOLID lzma

!macro REQUIRE NAME
  !ifndef ${NAME}
    !error "${NAME} fehlt: VERSION, STAGE und OUTFILE mit -D angeben, siehe scripts/build_windows_installer.sh"
  !endif
!macroend
!insertmacro REQUIRE VERSION
!insertmacro REQUIRE STAGE
!insertmacro REQUIRE OUTFILE

!define APP "prepolix"
!define PROGID "prepolix.Projekt"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define SOURCE_URL "https://github.com/KwentiN-ui/prepolix"

!include "MUI2.nsh"
!include "x64.nsh"

Name "${APP} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "Software\${APP}" "InstallDir"
RequestExecutionLevel admin
ManifestDPIAware true
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1031 "ProductName" "${APP}"
VIAddVersionKey /LANG=1031 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1031 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1031 "FileDescription" "${APP} Installer"
VIAddVersionKey /LANG=1031 "LegalCopyright" "GNU GPL 3.0 oder neuer"

!define MUI_ICON "${STAGE}\prepolix.ico"
!define MUI_UNICON "${STAGE}\prepolix.ico"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC

!define MUI_WELCOMEPAGE_TEXT "Dieser Assistent installiert ${APP} ${VERSION}, einen Prä- und Postprozessor für CalculiX.$\r$\n$\r$\nDer Solver CalculiX (ccx) gehört nicht dazu. Seinen Pfad trägst du nach der Installation unter Werkzeuge > Einstellungen ein.$\r$\n$\r$\n$_CLICK"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "${APP} starten"
!define MUI_FINISHPAGE_RUN_FUNCTION StartWithoutAdminRights
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "German"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "${APP} läuft nur auf 64-Bit-Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

; The installer runs with administrator rights; prepolix itself should not, so it is started
; through Explorer, which runs it as the logged-in user.
Function StartWithoutAdminRights
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\prepolix.exe"'
FunctionEnd

Section "${APP} (erforderlich)" SecApp
  SectionIn RO
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  File "${STAGE}\prepolix.exe"
  File "${STAGE}\gmsh-4.15.dll"
  File "${STAGE}\prepolix.ico"
  File "${STAGE}\LICENSE.txt"
  File "${STAGE}\THIRD-PARTY.txt"
  File "${STAGE}\SOURCE.txt"
  SetOutPath "$INSTDIR\licenses"
  File "${STAGE}\licenses\*.txt"
  SetOutPath "$INSTDIR"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\prepolix.ico"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${APP}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "${SOURCE_URL}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  SectionGetSize ${SecApp} $0
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" $0

  ; "Open with" offers prepolix for the files it reads.
  WriteRegStr HKLM "Software\Classes\Applications\prepolix.exe" "FriendlyAppName" "${APP}"
  WriteRegStr HKLM "Software\Classes\Applications\prepolix.exe\shell\open\command" "" '"$INSTDIR\prepolix.exe" "%1"'
  !macro SUPPORTED EXT
    WriteRegStr HKLM "Software\Classes\Applications\prepolix.exe\SupportedTypes" "${EXT}" ""
  !macroend
  !insertmacro SUPPORTED ".plx"
  !insertmacro SUPPORTED ".inp"
  !insertmacro SUPPORTED ".frd"
  !insertmacro SUPPORTED ".step"
  !insertmacro SUPPORTED ".stp"
  !insertmacro SUPPORTED ".iges"
  !insertmacro SUPPORTED ".igs"
  !insertmacro SUPPORTED ".brep"
SectionEnd

Section "Verknüpfung im Startmenü" SecStartMenu
  SetShellVarContext all
  CreateShortCut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\prepolix.exe" "" "$INSTDIR\prepolix.ico"
SectionEnd

Section /o "Verknüpfung auf dem Desktop" SecDesktop
  SetShellVarContext all
  CreateShortCut "$DESKTOP\${APP}.lnk" "$INSTDIR\prepolix.exe" "" "$INSTDIR\prepolix.ico"
SectionEnd

Section ".plx-Projekte mit ${APP} öffnen" SecAssociation
  WriteRegStr HKLM "Software\Classes\.plx" "" "${PROGID}"
  WriteRegStr HKLM "Software\Classes\.plx\OpenWithProgids" "${PROGID}" ""
  WriteRegStr HKLM "Software\Classes\${PROGID}" "" "${APP}-Projekt"
  WriteRegStr HKLM "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\prepolix.ico"
  WriteRegStr HKLM "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\prepolix.exe" "%1"'
  ; SHCNE_ASSOCCHANGED: Explorer picks up the new icon and program right away.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "Das Programm mit der Gmsh-Bibliothek für Geometrie-Import und Vernetzung."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStartMenu} "Eintrag im Startmenü für alle Benutzer."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Symbol auf dem Desktop für alle Benutzer."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAssociation} "Doppelklick auf eine .plx-Datei öffnet sie in ${APP}."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"

  Delete "$INSTDIR\prepolix.exe"
  Delete "$INSTDIR\gmsh-4.15.dll"
  Delete "$INSTDIR\prepolix.ico"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY.txt"
  Delete "$INSTDIR\SOURCE.txt"
  RMDir /r "$INSTDIR\licenses"
  ; The work directory prepolix creates next to itself when it may write there.
  RMDir /r "$INSTDIR\Temp"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; Only remove the .plx association if it still points to prepolix.
  ReadRegStr $0 HKLM "Software\Classes\.plx" ""
  ${If} $0 == "${PROGID}"
    DeleteRegValue HKLM "Software\Classes\.plx" ""
  ${EndIf}
  DeleteRegValue HKLM "Software\Classes\.plx\OpenWithProgids" "${PROGID}"
  DeleteRegKey /ifempty HKLM "Software\Classes\.plx\OpenWithProgids"
  DeleteRegKey /ifempty HKLM "Software\Classes\.plx"
  DeleteRegKey HKLM "Software\Classes\${PROGID}"
  DeleteRegKey HKLM "Software\Classes\Applications\prepolix.exe"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

  DeleteRegKey HKLM "${UNINSTALL_KEY}"
  DeleteRegKey HKLM "Software\${APP}"
  ; Settings in %APPDATA%\prepolix stay, like those of most programs.
SectionEnd
