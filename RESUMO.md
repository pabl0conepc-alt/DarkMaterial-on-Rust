# DarkMaterial on Rust — Resumo do projeto

Recriação do **DankMaterialShell (DMS)** para o compositor **driftwm** no **NixOS**. Ele segue o visual e as funções do original e gasta cerca de **5× menos memória**.

- **Repositório:** `pabl0conepc-alt/DarkMaterial-on-Rust`, branch `claude/gallant-ptolemy-53oewv`.
- **Comando:** `dmr`.
- **Arquitetura:** a mesma do Sheels.

---

## O que é

| Camada | Tecnologia |
|---|---|
| Backend | Daemon em **Rust** (`dmr`), runtime tokio single-thread e zbus, todo orientado a eventos |
| Interface | **EWW** com patch próprio (`nix/eww-dmr.patch`) |
| Compositor | **driftwm**: os bookmarks do canvas viram os workspaces da barra |
| Distribuição | NixOS: `install.sh` de um comando, módulo sem flake, flake e `dmr setup` |

Tamanho: ~8.600 linhas de Rust (27 módulos) e ~2.000 linhas de yuck/SCSS (10 arquivos de interface). O binário tem 6,6 MB.

---

## Componentes recriados do DMS

- **DankBar**: barra flutuante de pílulas, no topo ou embaixo.
  - **Widgets:** launcher, workspaces, janela focada, mídia, relógio, clima, bandeja, clipboard, CPU, RAM, notificações, bateria, Central de Controle, idle inhibitor e energia.
- **Spotlight**: busca de apps, ações (`>`), calculadora, esquemas de cor e wallpapers, tudo pelo teclado.
- **Central de Controle**: usuário e uptime, sliders de brilho e volume, tiles de Wi-Fi, Bluetooth e áudio que expandem listas, os 4 toggles (escuro, noturno, DND, manter acordado), perfil de energia e disco.
- **DankDash**: abas Overview (relógio, clima, calendário, mídia, notificações), Mídia (MPRIS), Wallpapers e Clima.
- **Notificações**: popups e central agrupada, com DND.
- **OSD** de volume e de brilho.
- **Menu de energia**: navegação por setas, Enter e as teclas R/X/P/L/S/D.
- **Tela de bloqueio**: hyprlock com template no estilo do lock do DMS.
- **Configurações** (janela do DMS), com estas páginas:
  - **Aparência:** Wallpaper & colors, Theme & colors, Interface style, Fonts & motion.
  - **Barra e painéis:** Bar, Bar widgets, Launcher, Dashboard.
  - **Notificações e mídia:** Notifications, OSD, Sound & media.
  - **Sistema:** Displays, Power & battery, Network.
  - **Outras:** Keyboard shortcuts, Default apps, Time & weather, About.
- **Temas**:
  - Material You gerado do wallpaper, com 9 variantes;
  - os 10 temas prontos do DMS;
  - esquemas extras (catppuccin, gruvbox, nord, rosepine…);
  - modo claro e escuro.
- **Atalhos**: os mesmos do DMS, aplicados ao driftwm.

---

## Desempenho

Medido no mesmo ambiente para os dois (sway headless → driftwm aninhado, 1920×1080, renderização por software).

| Situação | DMS original | DarkMaterial |
|---|---|---|
| Ocioso | 0,17 % CPU · 310 MiB | **0,07 % · 66 MiB** |
| Central de Controle aberta | 0,20 % · 330 MiB | 0,03 % · 71 MiB |
| DankDash aberta | 0,17 % · 359 MiB | 0,07 % · 73 MiB |
| Configurações abertas | — | 0,03 % · 82 MiB |

**Por que gasta pouco:**
- **Eventos em vez de polling.** O driftwm é lido pelo IPC `subscribe`. O áudio vem de `pactl subscribe`; NetworkManager e BlueZ, de sinais D-Bus. A mídia vem do MPRIS e as notificações chegam ao próprio daemon.
- **Relógio uma vez por minuto**, em vez do tique de 1 s do EWW.
- **CPU e RAM** da barra: duas leituras de `/proc` a cada 3 s.
- **Painéis pesados** (Central e Dash) são construídos uma vez e depois só revelados. Os que usam o teclado só existem enquanto estão abertos.
- **Barra sob medida:** o daemon gera a barra só com os widgets configurados.
- **Patch do EWW:**
  - memoiza props e loops `for` (≈8× menos CPU por atualização);
  - corrige o `:visible`;
  - adiciona `:onkeypress` e `:autofocus` no input;
  - trunca texto colado em uma linha;
  - faz o escurecimento de fundo cobrir a barra;
  - devolve memória ao sistema (`malloc_trim`) ao fechar janelas.

---

## Robustez e segurança

- **Texto do usuário nunca entra em comando de shell.** O texto vai por heredoc. Redes, dispositivos, wallpapers e notificações são referenciados por índice ou id.
- **O layout de teclado nunca é forçado para `us`.** O `dmr setup` usa o layout do sistema.
- **Sem processos órfãos.**
  - Os comandos ao EWW usam `--no-daemonize`.
  - Cada listener sai sozinho quando o EWW some.
  - Um vigia reinicia o EWW se ele cair.
- **O relógio se corrige ao acordar da suspensão** (sinal do logind).

---

## Instalação

```bash
curl -fsSL https://raw.githubusercontent.com/pabl0conepc-alt/DarkMaterial-on-Rust/main/install.sh | bash
```

- **Sem flake:** `imports = [ ".../nix/module.nix" ]; programs.darkmaterial.enable = true;`
- **Com flake:** `darkmaterial.nixosModules.default` (já inclui o driftwm).
- **Depois de instalar:** `dmr setup` instala a config do driftwm com os atalhos do DMS, cria a pasta de wallpapers e faz um check-up.

> O `install.sh` aponta para a branch `main`. Antes do merge, use `DMR_REF=claude/gallant-ptolemy-53oewv`.

---

## Como foi validado

- **Prints do DMS v2.1.2 original** rodando no mesmo ambiente. As medidas (barra de 38 px, pílulas de 30 px, cantos de 12 px, Central de 598 px, Dash de 832 px) foram copiadas pixel a pixel. As comparações estão em `docs/compare/`.
- **Testes de interação reais** (mouse e teclado via VNC) em todos os painéis e na janela de Configurações.
- **Builds:** o pacote Nix compila; o sistema NixOS completo avalia tanto pelo caminho sem flake quanto pelo flake; o `cargo clippy` não dá avisos; os 16 testes unitários passam.

### Bugs encontrados e corrigidos durante o desenvolvimento

- O EWW aplica o nome da janela como classe CSS, e a classe `.bar` pintava a janela inteira.
- A central de notificações escondida ficava como uma área invisível no centro da tela que engolia cliques.
- Comandos ao EWW criavam EWW paralelos quando o principal caía, e os listeners ficavam órfãos reconectando para sempre.
- O uso de disco era calculado errado (91% em vez de 43%).
- O daemon acordava a cada 1 s para ler os LEDs de Caps Lock e Num Lock, sobra do Caelestia.

---

## O que ficou de fora

- **Componentes do DMS sem versão aqui:** dock, notepad, gerenciador de processos, plugins, letras sincronizadas, visualizador de áudio, desktop widgets e a aba "Digital wellbeing".
- **Configurações sem equivalente no DarkMaterial:** usuários, impressoras, atualizações, greeter, regras de janela, monitores.
- **Busca de arquivos:** o Spotlight não procura arquivos.
- **Animações:** os painéis deslizam e esmaecem com curvas Material 3, sem o *spring* e o blur do Quickshell.

---

## Créditos

- **DankMaterialShell:** Avenge Media (bbedward, purian23 e contribuidores). Licença MIT.
- **driftwm:** Klim Kostiuk (malbiruk). Licença GPL-3.0.
- **Base do projeto:** Sheels; e, por tabela, Caelestia e caelestia-cli (soramanew).
- **EWW:** elkowar.
- **Outros:** Quickshell, material-colors, Material Symbols, Google Sans Flex, hyprlock, wlsunset, Open-Meteo.

Licença do projeto: **GPL-3.0**.
