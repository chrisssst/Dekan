Unicode True
!include "MUI2.nsh"
!include "x64.nsh"
!ifndef PAYLOAD_DIR
!error "Pass PAYLOAD_DIR"
!endif
Name "Dekan 2.1"
OutFile "Dekan-Setup-2.1-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\Dekan-2.1"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
VIProductVersion "2.1.0.0"
VIAddVersionKey "ProductName" "Dekan"
VIAddVersionKey "ProductVersion" "2.1"
VIAddVersionKey "CompanyName" "Dekan"
VIAddVersionKey "FileDescription" "Dekan 2.1 Kurulum"
VIAddVersionKey "FileVersion" "2.1"
VIAddVersionKey "OriginalFilename" "Dekan-Setup-2.1-x64.exe"
VIAddVersionKey "LegalCopyright" "See LICENSE.txt for copyright and MIT notices."
!define MUI_ICON "../assets/dekan.ico"
!define MUI_UNICON "../assets/dekan.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "Dekan 2.1 Kurulumu"
!define MUI_WELCOMEPAGE_TEXT "Dekan 2.1 uygulama önizlemesini kurar.$\r$\n$\r$\nBu paket, mevcut Dekan kurulumunun üzerine yazmaz. Oyun özellikleri için yetkili motor ve GitHub giriş yapılandırması gerekir."
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_HEADER_TEXT "Dekan 2.1 Kurulum Koşulları"
!define MUI_PAGE_HEADER_SUBTEXT "Dekan 2.1 kurulumuna devam etmeden önce aşağıdaki bilgileri okuyun."
!define MUI_LICENSEPAGE_TEXT_TOP "Dekan 2.1"
!define MUI_LICENSEPAGE_TEXT_BOTTOM "Dekan 2.1 kurulumuna devam etmek için Kabul ediyorum düğmesine tıklayın."
!define MUI_LICENSEPAGE_BUTTON "Kabul ediyorum"
!insertmacro MUI_PAGE_LICENSE "dekan-kurulum-kosullari.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Turkish"
Function .onInit
  SetShellVarContext current
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Dekan 2.1 için 64 bit Windows gerekir."
    Abort
  ${EndIf}
FunctionEnd
Section
  SetOutPath "$INSTDIR"
  File "${PAYLOAD_DIR}/dekan.exe"
  File "${PAYLOAD_DIR}/LICENSE.txt"
  File "${PAYLOAD_DIR}/THIRD-PARTY-NOTICES.md"
  File "${PAYLOAD_DIR}/ONCE_OKU.txt"
  File "${PAYLOAD_DIR}/dekan.ico"
  CreateDirectory "$SMPROGRAMS\Dekan 2.1"
  CreateShortcut "$SMPROGRAMS\Dekan 2.1\Dekan 2.1.lnk" "$INSTDIR\dekan.exe" "" "$INSTDIR\dekan.ico"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "DisplayName" "Dekan 2.1"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "DisplayVersion" "2.1"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "Publisher" "Dekan"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "DisplayIcon" "$INSTDIR\dekan.ico"
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview" "NoRepair" 1
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  Delete "$INSTDIR\dekan.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.md"
  Delete "$INSTDIR\ONCE_OKU.txt"
  Delete "$INSTDIR\dekan.ico"
  Delete "$SMPROGRAMS\Dekan 2.1\Dekan 2.1.lnk"
  RMDir "$SMPROGRAMS\Dekan 2.1"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Dekan21Preview"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
