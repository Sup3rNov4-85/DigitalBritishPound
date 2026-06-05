; NSIS installer for the Windows DBC launcher UI + node binary.
; Installs into $LOCALAPPDATA to avoid requiring admin rights.
;
; Build (run makensis):
;   "C:\Program Files (x86)\NSIS\makensis.exe" "D:\dbc-node\installer\dbc-installer.nsi"

!include "MUI2.nsh"

!define PRODUCT_NAME "Digital British Coin (DBC) Launcher"
!define STARTMENU_FOLDER "Digital British Coin (DBC)"

Name "${PRODUCT_NAME}"
OutFile "..\dbc-installer.exe"

RequestExecutionLevel user

InstallDir "$LOCALAPPDATA\DigitalBritishPound\DBC"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Use default install dir, but allow user to override.
Function .onInit
  StrCpy $INSTDIR "$LOCALAPPDATA\DigitalBritishPound\DBC"
FunctionEnd

Section "Install"
  SetOverwrite on
  SetOutPath "$INSTDIR"

  ; Core binaries + data required by the UI.
  File "..\release-package\dbc-ui.exe"
  File "..\release-package\dbc-node.exe"
  File "..\release-package\genesis.json"
  File "..\release-package\README.txt"
  File "..\release-package\DBC_Node_README.pdf"
  File "..\release-package\SHA256SUMS.txt"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Start Menu shortcuts.
  CreateDirectory "$SMPROGRAMS\${STARTMENU_FOLDER}"
  CreateShortCut "$SMPROGRAMS\${STARTMENU_FOLDER}\DBC Launcher.lnk" "$INSTDIR\dbc-ui.exe"
  CreateShortCut "$SMPROGRAMS\${STARTMENU_FOLDER}\Uninstall DBC Launcher.lnk" "$INSTDIR\uninstall.exe"

SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\${STARTMENU_FOLDER}\DBC Launcher.lnk"
  Delete "$SMPROGRAMS\${STARTMENU_FOLDER}\Uninstall DBC Launcher.lnk"
  RMDir  "$SMPROGRAMS\${STARTMENU_FOLDER}"

  Delete "$INSTDIR\dbc-ui.exe"
  Delete "$INSTDIR\dbc-node.exe"
  Delete "$INSTDIR\genesis.json"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\DBC_Node_README.pdf"
  Delete "$INSTDIR\SHA256SUMS.txt"
  Delete "$INSTDIR\uninstall.exe"

  RMDir "$INSTDIR"
SectionEnd

