; Ganchos do instalador NSIS do CapIA (Tauri `bundle.windows.nsis.installerHooks`).
;
; Regra inegociável: instalar, atualizar, reparar e DESINSTALAR nunca apagam projetos do usuário.
;  - projetos (.capia) ficam onde o usuário os criou (nunca no diretório de instalação);
;  - o desinstalador do Tauri remove só o que o instalador colocou (e, se o usuário marcar a caixa, a
;    pasta de dados do app: preferências/logs/banco do app — nunca projetos nem mídia).
; Este gancho é rede de segurança: se alguém salvou um .capia DENTRO do diretório de instalação, ele é copiado
; para Documentos\CapIA-recovered-projects antes da remoção.

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  FindFirst $0 $1 "$INSTDIR\*.capia"
  StrCmp $1 "" capia_recover_done
  CreateDirectory "$DOCUMENTS\CapIA-recovered-projects"
  capia_recover_loop:
    CopyFiles /SILENT "$INSTDIR\$1" "$DOCUMENTS\CapIA-recovered-projects\$1"
    FindNext $0 $1
    StrCmp $1 "" capia_recover_done
    Goto capia_recover_loop
  capia_recover_done:
  FindClose $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
