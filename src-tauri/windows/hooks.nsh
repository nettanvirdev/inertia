; Repoint the Windows "Installed apps" entry at the themed uninstaller.
;
; Tauri's template writes UninstallString pointing at its own uninstall.exe at
; the end of the install section; NSIS_HOOK_POSTINSTALL runs after that, so
; this overwrites it. inertia-uninstall.exe is shipped as an app resource
; mapped to the install root, so it is already in place by the time this runs.
;
; QuietUninstallString deliberately still points at the raw NSIS uninstaller:
; anything invoking that wants no UI at all, and handing it a window would be
; the wrong answer.
!macro NSIS_HOOK_POSTINSTALL
  ${If} ${FileExists} "$INSTDIR\inertia-uninstall.exe"
    WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\inertia-uninstall.exe$\""
    WriteRegStr SHCTX "${UNINSTKEY}" "QuietUninstallString" "$\"$INSTDIR\uninstall.exe$\" /S"
  ${EndIf}
!macroend
