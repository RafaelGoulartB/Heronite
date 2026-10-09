# PROGRESSO — HeroniteRust

Diário de trabalho para retomar a migração. Plano geral em `PLANO.md`.
Última atualização: 2026-10-07.

## Decisões em vigor

| Tema | Decisão |
|---|---|
| UI | GPUI via `gpui-kit = 0.7.1` (fixa `gpui-pre 0.3.8`, snapshot do Zed + `gpui-component`) |
| Dados | **Os mesmos arquivos do Heronite** em `~/.config/heroic` (electron-store JSON). Leitura e gravação compatíveis: tab, sem newline final, gravação atômica, read-modify-write, campos desconhecidos preservados (`extra` flatten), números via `JsNum` |
| Override de dados | `HERONITE_HEROIC_DIR=/caminho` aponta o app para outra pasta (testes, cópia) |
| Estado de UI próprio | `~/.config/heronite-rs/ui-state.json` (no Heronite isso fica no localStorage do Chromium) |
| Cache de imagens | Compartilha `~/.config/heroic/images-cache/<sha256(url)>` com o Heronite; miniaturas redimensionadas em `~/.cache/heronite-rs/thumbs` |
| Binário | `heronite-rs` (não conflita com `~/.local/bin/heronite`) |
| Builds | Testar: `cargo run --profile quick -p heronite` (otimizado, sem LTO, ~10 s por mudança). Instalar: `--release` (LTO fat, ~9 min). Linker `lld` em `.cargo/config.toml` |
| Plataforma | Linux primeiro; caminhos via `dirs`, código Linux-only atrás de `cfg(target_os = "linux")` |
| Janelas de teste | Abrir **só no workspace 2** do Hyprland: `/tmp/heronite-bench/ws2.sh <cmd>` → `hyprctl eval 'hl.exec_cmd(cmd, {workspace="2 silent"})'` |

## Tarefas em segundo plano

- 90 s após abrir: varredura automática de ROMs e backup agendado da config (igual ao Heronite).
  `HERONITE_DISABLE_IDLE_JOBS=1` desliga (use em testes contra dados reais).
- Após cada sessão: chave de save state e backup Ludusavi (se habilitado).

## Convivência com o Heronite

- O electron-store do Heronite mantém os dados em memória e regrava o arquivo inteiro quando
  muda algo. Se os dois apps estiverem abertos e ambos alterarem o mesmo arquivo, o último a
  gravar vence. Recomendação: não editar nos dois ao mesmo tempo. O app Rust relê o arquivo
  antes de cada gravação e observa o mtime para recarregar mudanças externas.
- O shell do T3 Code exporta `ELECTRON_RUN_AS_NODE=1`; para abrir o Heronite por aqui use
  `env -u ELECTRON_RUN_AS_NODE`.

## Linha de base do Heronite (medida em 2026-10-07)

- PSS ocioso: 565 MB (9 processos). Após iniciar: 606–678 MB (3 execuções).
- Janela aparece em 0,64–0,87 s; tela estável entre 0,9 e 6,7 s.
- Instalação: 409 MB.
- Script: `/tmp/heronite-bench/run-electron.sh` (pode sumir no reboot; lógica descrita acima).
- Referências visuais (via CDP, `--remote-debugging-port=9333`): `docs/reference/*.png`.

## Como testar visualmente (sem atrapalhar os outros workspaces)

> **Limitação descoberta:** com o workspace 2 **oculto**, o Hyprland deixa de enviar frames para a
> janela logo após ela abrir. A captura `grim -T` então mostra só os primeiros quadros (capas e
> fundo que chegam depois não aparecem). As capturas só são fiéis com o workspace 2 visível.
> Alternativa a combinar com o usuário: monitor virtual (`hyprctl output create headless`) posicionado
> longe dos monitores reais. `HERONITE_NO_ANIMATIONS=1` (usado pelo `shot.sh`) evita fades presos.

- `scripts/shot.sh /tmp/x.png [espera_s] [binário]` abre o `heronite-rs` no workspace 2 em modo
  silencioso, captura a janela com `grim -T <stableId>` (funciona mesmo fora da tela), imprime o
  PSS e fecha. `SHOT_ENV="HERONITE_DEBUG_SELECT=<key>"` pré-seleciona um jogo.
- O Heronite pode ser capturado do mesmo jeito (`grim -T` com o stableId da janela
  `com.heronite.launcher`) ou via CDP (`--remote-debugging-port=9333`,
  `/tmp/heronite-bench/cdp-shot.mjs`).
- Hyprland 0.56 usa config Lua: `hyprctl eval 'hl.exec_cmd("cmd", { workspace = "2 silent" })'`.

## Achados de paridade visual

> Decisão do usuário (2026-10-07): diálogos, menus, abas de configuração e modais **não**
> precisam ser idênticos ao Heronite (gostou do estilo atual). Paridade fina só na tela
> principal da Collection (grade, painel de foco, faixa de screenshots).


- O Heronite só carrega Rubik/Cabin **400**; o Chromium simula negrito para CSS 600/700 e
  renderiza 500 como regular. O GPUI não simula negrito, então usamos o corte real **600** para
  CSS 600/700 (`theme::W_BOLD`) e 400 para CSS 500 (`theme::W_500`). Só os pesos 400 e 600 são
  embutidos.
- `box-shadow: 0 0 0 2px` (anel de seleção) vira uma borda de 2px num div absoluto em -2px.
- Gradientes multi-stop do scrim viram faixas de gradientes de 2 stops (`stage.rs`).
- Tema real do usuário: `nord-dark` (não midnightMirage).

## Medições do app Rust (release, 2026-10-07, mesma biblioteca de 207 jogos)

| Métrica | Heronite (Electron) | heronite-rs (release) |
|---|---|---|
| Memória (PSS) após abrir | 606–678 MB (8–9 processos) | **96 MB** (1 processo) |
| Até a Collection aparecer | janela 0,64–0,87 s; estável 0,9–6,7 s | **0,24–0,26 s** (biblioteca carregada em ~6 ms) |
| Tamanho | 409 MB instalado | 38 MB (binário único, fontes/ícones embutidos) |

Medido com `scripts/shot.sh` (PSS via `/proc/<pid>/smaps_rollup` após 4 s; tempo pelo log
`library on screen after`). Ainda sem otimizações finas (driver NVIDIA responde por ~55 MB).

## Primeiras medições GPUI

- Hello world em **debug**: 179 MB PSS. Composição: driver NVIDIA ~55 MB, camada de validação
  Vulkan 22 MB (só debug), binário debug 31 MB, heap 31 MB. Medir release depois.

## Estrutura

```
Cargo.toml            workspace
app/                  binário heronite-rs (UI GPUI)
crates/data/          heronite-data: caminhos, json_store, models, JsNum (+ lógica da Collection)
docs/reference/       screenshots do Heronite para paridade visual
```

## Checklist

### Fase 0/1 — base
- [x] Workspace Cargo + GPUI compilando e abrindo janela no Wayland/NVIDIA
- [x] Linha de base do Heronite medida + screenshots de referência
- [x] `heronite-data`: paths, json_store (formato electron-store), JsNum, models de todos os arquivos
- [x] Teste de ida e volta com cópia dos dados reais (`HERONITE_FIXTURE=/tmp/heronite-fixture`)
- [ ] Montagem da biblioteca (sideload + legendary + gog + nile − DLC − ocultos) + view model
- [ ] Lógica da Collection: status, grupos, facetas, busca, ordenação, relacionados, cadeias de arte
- [ ] `heronite-images`: resolução de fontes, cache, decodificação no tamanho de exibição, blur do stage, LRU
- [ ] `heronite-launch`: Steam URI, nativo, emulador, Proton/umu, legendary; sessões e playtime

### Fase 2 — UI da Collection
- [x] Tema (nord-dark, fontes Rubik/Cabin 400+600, ícones MUI/FontAwesome em SVG)
- [x] Stage + scrim (layout), header (estático), lista virtual agrupada com cabeçalho fixo, cards
      (anel, barra de hover animada, playtime, pulso "jogando")
- [x] Painel de foco: cabeçalho, overview, relacionados, detalhes com chips, HLTB, rodapé
- [x] Ligar aos dados reais (`heronite-data`) e às imagens (`heronite-images`, LRU de texturas,
      stage pré-borrado)
- [x] Busca, filtro de instalação, ordenação, agrupar por status, filtro por facetas (flyout)
- [x] Jogar / parar / instalar (Steam) via `heronite-launch`; estado "jogando"; recarga ao sair
- [x] Menu de contexto (status, jogar, favorito, pasta, ocultar, remover dos recentes)
- [x] Recarrega quando o Heronite grava (poll de mtime a cada 2 s)
- [x] Importa o estado de UI do localStorage do Heronite na 1ª execução
- [x] Overview com HTML da Steam (parser próprio), Esc limpa seleção, scrollbars finas
- [x] Screenshots: faixa no foco, galeria (páginas de 50), visualizador (zoom/pan/fit/lixeira),
      aba de configurações, sessão de captura ligada ao lançamento (captura real não testada)
- [x] Diálogos: configurações (Aparência, Screenshots, Emulators, Backup, Save backup), gerenciar
      status, Playnite (importar/mesclar/exportar), Scan ROMs, escolher disco, save state
- [x] Diálogos de metadados (busca IGDB/Lutris, escolha por campo), assistente em lote com chip de
      progresso, configurações do jogo (Imagens/Detalhes/Launch), busca de imagens na web,
      escolher screenshot como arte, aba Metadata, auto-preenchimento ocioso
- [x] Busca dentro do flyout de facetas; arrastar scrollbar
- [ ] Testar ao vivo (cliques reais): jogar/parar, salvar diálogos, captura por atalho, Ludusavi
- [ ] Verificação visual final com o workspace 2 visível (ver limitação acima)
- [x] Conquistas no painel de foco (GOG); botão "abrir página de detalhes"
- [ ] Build Windows (código portável; falta toolchain: máquina Windows ou `cargo-xwin`)
- [ ] Unificar `ui/fields.rs` e `dialogs/form.rs` (helpers de input duplicados)

### Fase 3 — paridade de recursos com o Heroic (iniciada 2026-10-07)
Pedido do usuário: trazer Wine Manager, configurações por jogo (Wine/Other/Advanced/Gamescope/Cloud saves),
login na Epic, biblioteca Epic integrada e instalação/download direto pelo app. Console mode e recursos
secundários ficam de fora por enquanto. A Collection continua sendo a interface; telas novas entram pelo
menu ☰ (canto superior esquerdo), sem sidebar.
- [x] Navegação ☰ (`app/src/pages/`, `collection/nav.rs`): Collection, Downloads, Wine Manager, Epic Games,
      Settings. `HERONITE_DEBUG_PAGE=downloads|wine|epic`, `HERONITE_DEBUG_DROPDOWN=nav`.
- [x] Crates esqueleto `heronite-wine` e `heronite-epic` (contratos usados entre agentes).
- [x] Epic: login, biblioteca, instalar/atualizar/reparar/mover/desinstalar, fila de downloads, cloud saves
      (agente "epic", brief em `docs/agents/epic.md`).
- [x] Wine Manager + detecção de Wine/Proton (agente "wine", `docs/agents/wine.md`).
- [x] Configurações por jogo e globais estilo Heroic (agente "settings", `docs/agents/settings.md`).
- [x] Paridade do lançamento: cloud saves automáticos, "Play anyway", preparo do prefixo (DXVK/VKD3D/NVAPI,
      runtimes EAC/BattlEye, umu), download das ferramentas (`docs/agents/launch-parity.md`).
- [ ] Testar ao vivo (cliques): login Epic, instalar/pausar/cancelar, Wine Manager, abas novas.
- [ ] Falta: seletor de plataforma (`legendarySetup`, `known_fixes`, EA/Ubisoft e atalhos já entraram).
- [x] Fase 3b (2026-10-08): itens restantes da Collection, Add game (sideload), `--launch`/`heroic://`/`heronite-rs://`,
      instância única, atalhos (desktop/menu/Steam), bandeja. Janelas de teste agora no **workspace 7**.
- Release após a fase 3: 537 testes passando, PSS ~100–106 MB (malloc_trim após a carga), binário 47 MB
      + `heronite-epic-login` 1 MB.

### Diferenças visuais conhecidas (aceitas)
- Cabeçalho de grupo fixo: sem blur real atrás (gradiente mais escuro no lugar).
- Painéis de vidro: sem blur extra de 18/22px sobre o stage (o stage já é borrado).

### Estado dos crates de backend
- [x] `crates/media` — matching de pastas (idêntico ao match.ts), lista, miniaturas no mesmo cache
      do Heronite, lixeira, serviço de captura (portal/Hyprland/grim; partes com efeito colateral
      não testadas ao vivo).
- [x] `crates/emulation` — catálogo/detecção, scanners, import de ROMs, disc-id, save states,
      Ludusavi, backup agendado, idle jobs.
- [x] `crates/metadata` — IGDB/Steam/Lutris, merge, bulk, arte, heroes, busca de imagens, HLTB
- [x] `crates/playnite` — LiteDB, importar/mesclar/exportar (conferido contra o original em Node)

### UI em andamento (agentes, ver `docs/UI-CONVENTIONS.md`)
- Screenshots: faixa no foco, galeria, visualizador, aba de configurações, captura.
- Emulação/backups: Scan ROMs, gerenciador de emuladores, abas Emulators/Backup/Save backup,
  escolha de ROM e save state ao jogar, Ludusavi pós-jogo, idle jobs.

### Backends em andamento (agentes)
- `crates/media` (screenshots + captura), `crates/metadata` (IGDB/Steam/Lutris/arte/HLTB/busca
  de imagens), `crates/playnite` (LiteDB, importar/mesclar/exportar), `crates/emulation` (ROMs,
  disc-id, save states, Ludusavi, backup).
- [ ] Menu de contexto, diálogos de status e aparência
- [ ] Comparação com `docs/reference`

## Log

- 2026-10-07 (noite): todos os módulos portados (dados, imagens, lançamento, screenshots,
  metadados, Playnite, emulação/backups) e respectivas telas. 381 testes passando. Release:
  PSS 95–96 MB, Collection na tela em ~0,25–0,28 s, binário 45 MB.
- 2026-10-07 (tarde): UI da Collection montada com dados de demonstração; layout bate com a
  referência quase ao pixel. Agentes em paralelo: lógica de dados (`crates/data`), imagens
  (`crates/images`), lançamento (`crates/launch`).
- 2026-10-07: workspace criado; GPUI validado; linha de base medida; camada de dados compatível com
  testes de ida e volta nos arquivos reais.

## Wine Manager (agente "wine")

Port do Wine Manager do Heroic (`backend/wine/manager/**`, `utils/compatibility_layers.ts`,
`screens/WineManager`). Código em `crates/wine` (backend, sem UI) e `app/src/pages/wine_manager.rs`.

- **Detecção** (`heronite_wine::list_installations(&HeroicPaths)`): mesma ordem do `getLinuxWineSet`:
  `tools/proton`, `compatibilitytools.d` das bibliotecas Steam (`libraryfolders.vdf` do
  `defaultSteamPath`, mais `/usr/share/steam`), `steamapps/common` se `showValveProton`, depois
  `tools/wine/*/bin/wine`, runners do Lutris, `wine` do PATH (`wine --version`) e
  `customWinePaths`. Pastas ocultas e `UMU-Latest*` são ignoradas. Só lê, nunca grava.
- **Repositórios**: os dois que o Heroic oferece no Linux (Proton-CachyOS e GE-Proton), com os
  mesmos filtros de asset (x86_64, `.sha512sum`) e a entrada sintética `<tipo>-latest`.
  `repos.rs` também sabe interpretar Proton (Valve) e Wine-Lutris, mas a UI não os mostra.
- **Cache** `store/wine-downloader-info.json` (formato idêntico: tabs, `wine-releases`, campos
  desconhecidos preservados). `store::merge_releases` é o port do `updateWineVersionInfos`
  (mantém `installDir`/`isInstalled`/`disksize`, marca `hasUpdate` quando o checksum do `-latest`
  muda, renomeia `Proton-GE-latest`, ordena por data). Atualização só sob demanda (botão), quando
  o cache está vazio ou o último fetch tem mais de 24 h (carimbo em `~/.config/heronite-rs/wine-releases-fetched`).
  Sem fetch quando `HERONITE_WINE_OFFLINE=1`. Limite do GitHub sem token: 60 req/h (1 req por aba por refresh).
- **Instalação** (`install.rs`): baixa em streaming para `<arquivo>.part` calculando sha512, confere
  com o `.sha512sum`, extrai com o `tar` do sistema (`--strip-components=1`; gz/xz/zst) numa pasta
  oculta `.<versão>.extracting` e só então renomeia (queda ou cancelamento nunca deixa instalação
  pela metade; atualização mantém a versão antiga até a nova estar completa). Destino: `tools/proton`,
  `tools/wine` ou, com `downloadProtonToSteam` e `steam.sh` válido, `<steam>/compatibilitytools.d`.
  Remoção só apaga pastas dentro de `tools/{wine,proton,game-porting-toolkit}` ou de um `compatibilitytools.d`.
  O botão "Install Proton into Steam" grava `downloadProtonToSteam` em `config.json` (2 espaços) e na cópia `store/config.json`.
- **Jobs** (`manager::WineManager`): threads `std`, estado compartilhado (progresso, cancelar, eventos);
  vive na entidade da página, que o `CollectionView` mantém viva, então o download continua ao trocar de página.
- **Página**: abas por repositório, busca, atualizar, lista (versão, data, tamanho/progresso com
  velocidade e ETA, ações Install/Cancel/Update/abrir pasta/remover com confirmação), notas de versão
  via `xdg-open` ao clicar no nome, erros e resultados como toast.
- **Verificação**: 21 testes unitários (parsing com fixtures reais, merge, formato do cache, install
  com servidor HTTP local incluindo checksum errado/cancelamento/atualização com rollback, remoção
  segura, detecção em tempdir) + `tests/real_machine.rs` (detecção na máquina real: acha
  `GE-Proton-latest` e `Proton-CachyOS Latest`) + `tests/real_network.rs` (`--ignored`, só contra
  `/tmp/heronite-fixture*`): fetch real do GitHub, download+checksum+extração `.tar.xz` reais de
  `cachyos-10.0-20250702-slr` (≈290 MB, 9 s) e remoção; fixture apagada depois.
- **Depuração**: `HERONITE_DEBUG_PAGE=wine`, `HERONITE_DEBUG_WINE=progress,confirm,ge,install=<versão>`.
- **Limitações**: cada refresh pode usar até 2 das 60 requisições/h; extração mostra "Unzipping"
  sem percentual (como o Heroic); sem retomar download interrompido; `customWinePaths` é editado na
  tela de configurações, não aqui; só Linux (macOS/GPTK/Crossover fora do escopo).

## Configurações do jogo e globais (agente "settings")

Port de `Settings/**` do Heroic: abas **Wine / Other / Advanced / Cloud saves / Gamescope** no diálogo
"Game settings" de cada jogo instalado (título `<jogo> (Settings)`) e abas **General** e **Defaults**
no diálogo "Collection settings". Tudo grava na hora que muda ("Settings are saved automatically").

- **Dados** (`crates/data/src/game_config.rs`, + `json_store::{write_indented, update_value_indented, INDENT_2}`):
  `GamesConfig/<app>.json` (`{"<app>": {...}, "version": "v0", "explicit": true}`) e `config.json`
  (`defaultSettings`), ambos com `JSON.stringify(.., null, 2)`. Só a chave alterada é gravada (leitura-
  modificação-gravação do documento cru: demais chaves e ordem intactas, bytes idênticos). Jogo sem arquivo
  recebe o conjunto completo de padrões que o Heroic gravaria; mudança global também é espelhada em
  `store/config.json` → `settings` quando essa cópia existe. Os 94 `GamesConfig` reais regravam byte a byte iguais.
- **Ferramentas** (`crates/launch/src/wine_tools.rs`): comandos de winecfg / winetricks (GUI) / "Run EXE on
  Prefix" (port de `callTool`, `runWineCommand`, `Winetricks.runWithArgs`; umu, Proton direto com o `wine` de dentro do
  Proton e `pfx`, ou Wine). Só constrói o comando (testado com `FakeProbe`); o botão da UI roda numa thread.
- **UI** (`app/src/dialogs/heroic_settings/`): um `HeroicSettingsPanel` reutilizável (alvo: jogo ou padrões
  globais). `logic.rs` (regras puras testadas: quais abas cada jogo tem, nomes de Wine, validações),
  `parts.rs` (linhas), um arquivo por aba. Wine só para jogos que rodam via Wine/Proton (Epic/GOG/sideload
  Windows), Cloud saves só para legendary com `cloud_save_enabled`; jogos Steam/emulador/não instalados
  não têm abas do Heroic. Escritas em segundo plano, em ordem; campos de texto gravam com debounce de 600 ms
  e ao perder o foco. Aba Legacy dobrada no fim de Advanced.
- **Pontos de ligação em arquivos compartilhados**: `app/src/dialogs/mod.rs` (+`pub mod heroic_settings`),
  `app/src/collection/context_menu.rs` (item "Settings"), `app/src/collection/mod.rs` (braços do
  `HERONITE_DEBUG_DIALOG`), `crates/data/src/library.rs` (o leitor enxuto agora mantém `cloud_save_enabled`
  em `GameInfo::rest`), `crates/data/src/lib.rs` e `crates/launch/src/lib.rs` (uma linha `pub mod` cada).
- **Depuração**: `HERONITE_DEBUG_DIALOG=gamesettings:wine|other|advanced|gamescope|saves`,
  `settings:general`, `settings:defaults[:other|advanced|gamescope]`; `HERONITE_DEBUG_SELECT=<runner_app>`;
  `HERONITE_DEBUG_SETTINGS_SET='chave=valor,gamescope.campo=valor'` (grava como se o usuário tivesse mexido;
  vírgula como separador porque o `shot.sh` passa por shell); `HERONITE_DEBUG_GAMESCOPE=1` (finge gamescope
  instalado); `HERONITE_DEBUG_DIALOG_H=<px>` (diálogo mais alto). Com `HERONITE_DEBUG_DIALOG` a lista de Wine
  carrega na hora (as capturas só mostram os primeiros quadros).
- **Verificação**: 7 testes em `game_config` (formato, bytes idênticos, mirror, arquivo novo), 11 em
  `wine_tools`, 7 em `logic`; capturas de cada aba (Calluna = Proton/legendary, Bloodborne = nativo,
  MGSV = sideload Wine) e escrita real contra cópia em `/tmp/heronite-fixture-settings*`.
- **Limitações**: os toggles DXVK/VKD3D/DXVK-NVAPI só gravam a configuração (não instalam/removem DLLs no prefixo
  na hora, como o Heroic faz); BattlEye/EAC só avisam quando o runtime não está instalado (sem download);
  não há seletor de "Launch options" nem Msync/macOS/CrossOver além do campo de bottle; "Sync" de saves usa os 3
  modos de `heronite_epic::saves::SyncMode` (Both / ForceDownload / ForceUpload); winecfg/winetricks/run-exe
  nunca foram executados de verdade (só testes de construção do comando); cliques reais não testados (sem mouse).

## Epic Games (agente "epic")

Port da loja Epic do Heroic, tudo via `legendary` como o Heroic faz, nos mesmos arquivos de `~/.config/heroic`:
login, biblioteca na Collection, instalar/atualizar/verificar/mover/desinstalar/importar, fila de downloads e
saves na nuvem. O `legendaryConfig/**` só é gravado pelo próprio legendary.

- **Backend** (`crates/epic`, 63 testes + `tests/real_legendary.rs` opt-in): `cmd` (acha o binário como o
  Heronite, `LEGENDARY_CONFIG_PATH`, saída por linha com `\n`/`\r`, cancelar = SIGTERM e depois SIGKILL no grupo;
  o filho recebe SIGTERM se o app morrer); `user` (`auth --code/--delete`, `userInfo` em `store/config.json`,
  aceita código puro, JSON da página ou URL); `library` (`list --third-party` e **reconstrução do
  `store_cache/legendary_library.json`** a partir de `metadata/*.json` + `installed.json`; as 99 entradas reais saem
  idênticas byte a byte às do Heronite, inclusive a ordem das chaves; detecção de update comparando `assets.json` com
  `installed.json`); `info` (`legendary info --json` com cache de 6 h em `legendary_install_info.json`, SDL e overrides
  da API do legendary, cache de 7 dias); `ops` (install/update com os mesmos argumentos do Heroic, retry com
  `--max-shared-memory 5000`, verify/repair com progresso, uninstall, move via rsync/mv + `legendary move --skip-move`,
  import, remoção do prefixo com as mesmas travas, config Wine no GamesConfig); `queue` + `manager` (fila em
  `store/download-manager.json` no formato `DMQueueElement`, um download por vez numa thread, pausar/retomar/cancelar
  — cancelar remove os DLCs enfileirados junto e opcionalmente apaga os arquivos baixados —, reordenar, histórico);
  `progress` (regexes do Heroic: ETA, Downloaded, Download, Disk; percentual como o Heroic com retomada); `saves`
  (`sync`/`sync_flag`/`detect_path`; `detect_path` devolve o `save_path` que o legendary já guardou, senão roda
  `sync-saves --skip-upload --skip-download --accept-path` com o ambiente Wine do jogo); `disk` (espaço livre, gravável).
- **Login embutido** (`crates/epic-login`, binário `heronite-epic-login`, wry + GTK/WebKit, fora do binário principal):
  abre `epicgames.com/id/login?responseType=code` com o user-agent do Heroic (sessão anônima), pega o código do redirect
  `https://localhost/...?code=` ou do JSON de `/id/api/redirect` e o imprime. O app o procura ao lado do executável.
  `--smoke` carrega uma página local (nunca abre rede); verificado: abre no workspace 2. Alternativa sempre disponível
  na página Epic: abrir `legendary.gl/epiclogin` no navegador e colar o código ou o JSON.
- **Serviço** (`app/src/services/downloads.rs`): entidade global `Downloads` (fila, conta, "update disponível",
  operações avulsas com progresso e cancelar, atualização da biblioteca). Como o Heroic: fila inicia 5 s depois de abrir,
  verificação de updates (rede) 15 s depois se `checkForUpdatesOnStartup`, `autoUpdateGames`/`ignoreGameUpdates`
  respeitados; tudo isso desligado por `HERONITE_DISABLE_IDLE_JOBS=1`.
- **Páginas**: **Downloads** (download atual com barra, velocidade, ETA, pausar/retomar, parar mantendo ou apagando
  arquivos; fila com subir/descer/remover; histórico com limpar) e **Epic Games** (login, conta, estatísticas,
  atualizar biblioteca, importar jogo, sair).
- **Diálogos** (`app/src/dialogs/epic_*.rs`): instalar (caminho com espaço livre, tamanhos, DLCs — todos marcados como
  no Heroic —, SDL, configuração Wine no Linux, botão "Import Game"), importar (escolher o jogo, depois a pasta),
  confirmações (desinstalar com "remove prefix"/"erase settings", mover, verify, "Game Needs Update") e saída de texto.
- **Collection** (`app/src/collection/epic.rs`): botão Install abre o diálogo, jogo na fila abre Downloads, jogo com
  update pergunta antes de jogar; card com faixa de progresso ("Installing 37%", "Queued", "Verifying 12%") e selo
  UPDATE; rodapé do foco com progresso + pausar/parar e botão Update; menu de contexto com Update, Verify and repair,
  Move install, Cloud saves sync, Uninstall e Import game. Toda operação recarrega a biblioteca.
- **Pontos de ligação em arquivos compartilhados**: `app/src/main.rs` (`mod services;`), `app/src/dialogs/mod.rs`
  (`pub mod epic_*`), `app/src/collection/mod.rs` (campo `epic`, `epic_primary` em `primary_action`, `drive_epic`,
  `epic_card`/`epic_focus` ao montar card/foco, braço do `HERONITE_DEBUG_DIALOG`), `card.rs` (campo `epic` + overlay),
  `focus.rs` (campo `epic` + conteúdo do rodapé), `context_menu.rs` (uma chamada), `runtime.rs` (mensagem do ramo
  legendary), `scripts/install.sh` (compila e instala o `heronite-epic-login` ao lado do app).
- **Depuração**: `HERONITE_DEBUG_PAGE=downloads|epic`; `HERONITE_DEBUG_DIALOG=epic-install|epic-import|epic-uninstall|
  epic-move|epic-verify|epic-update:<appName>` e `epic-message`; `HERONITE_DEBUG_FAKE_DOWNLOAD=<appName>` (fila falsa com
  progresso sem baixar nada); `HERONITE_DEBUG_FAKE_UPDATE=<appName>`; `HERONITE_DEBUG_QUEUE_INSTALL=<appName>`
  (+ `HERONITE_DEBUG_INSTALL_DIR`, `HERONITE_DEBUG_INSTALL_DLCS`) enfileira uma instalação de verdade, para usar com um
  legendary falso em `HERONITE_LEGENDARY_BIN`. Com `HERONITE_DEBUG_DIALOG` o diálogo de instalar usa o cache de
  `legendary info` na hora (as capturas só mostram os primeiros quadros).
- **Verificação**: testes unitários com um `legendary` falso (script) para argumentos, progresso, retry de memória,
  cancelar/pausar/retomar, fila persistida, mover/importar/prefixo; `real_legendary.rs` contra a conta real (só
  `list`/`info`/API, com `legendaryConfig` da fixture como link para o real): 99 jogos reconstruídos, tamanhos de
  download, SDL; teste ponta a ponta no app com legendary falso (fila -> progresso -> `installed.json` -> biblioteca
  reconstruída -> Collection recarregada -> histórico `done`); capturas de todas as telas e diálogos.
- **Limitações**: instalar/atualizar/verificar/mover/desinstalar/importar/sync de saves **nunca rodaram com o
  legendary real** (só contra o falso); cliques reais não testados (sem mouse; o login embutido nunca foi usado com
  credenciais); jogos EA/Ubisoft: ver a seção "EA app / Ubisoft Connect" no fim; sem seletor de plataforma (só Windows no Linux),
  sem branches/builds (GOG), sem atalhos de desktop/Steam, sem notificações do sistema, sem checagem de "Epic offline";
  `detect_path` não descarta o `save_path` já guardado (o Heroic edita o `installed.json`); "Play anyway" no aviso de
  update pode ser recusado pelo legendary (o plano de lançamento ainda não passa `--skip-version-check`).

## Paridade de lançamento Wine/Proton + Epic (agente "launch")

O que o Heroic faz *em volta* de um lançamento (`prepareLaunch`, `prepareWineLaunch`, `launchEventCallback`) agora
também acontece quando o jogo é iniciado pelo app Rust. Tudo roda na thread de lançamento (nunca na UI) e avisa por
`LaunchEvent::Status { message, warning }`, que a Collection mostra como toast.

- **Plano** (`crates/launch/src/plan.rs`): `ProcessPlan.prep: PrepSpec` descreve o que o lançamento precisa
  (`umu` a baixar, `wine` = configurações efetivas do jogo, `cloud_saves` = `savesPath`). Planos de Wine (sideload,
  emuladores `.exe`) e de Epic preenchem; Epic agora também cria o prefixo que falta (`pre_commands`, como o
  `verifyWinePrefix`), antes de instalar o DXVK. Se o umu é desejado e não está instalado, o plano usa umu e o
  lançamento o baixa (antes caía para Proton direto); sem rede, o erro manda ligar "Disable UMU".
- **Ordem** (`runtime.rs::run_process`): saves da nuvem (`--skip-upload`, só `autoSyncSaves` + `savesPath` + online) ->
  umu -> criação do prefixo ("Preparing prefix...") -> DXVK/DXVK-NVAPI/VKD3D no prefixo (só Wine, conforme
  `autoInstallDxvk`/`autoInstallDxvkNvapi`/`autoInstallVkd3d`) -> runtimes EAC/BattlEye (`eacRuntime`/`battlEyeRuntime`)
  -> jogo -> upload dos saves (`--skip-download`; não conta como tempo de jogo; não roda após "force clear").
  Parar durante o preparo cancela sem iniciar o jogo nem gravar sessão. Falhas de saves/DXVK/runtimes são avisos
  (toast de erro) e o jogo inicia, como no Heroic; só a falta do umu aborta.
- **Módulos novos** (`crates/launch/src/`): `prepare.rs` (orquestra; trait `CloudSync` implementada em
  `app/src/runtime.rs` com `heronite_epic::saves::sync_flag`, porque o crate epic depende do launch), `tools.rs`
  (`installOrUpdateTool`/`DXVK.getLatest`: baixa o release do GitHub para `tools/<tool>/<release>` + `latest_<tool>`,
  extração em pasta oculta e rename atômico; winetricks; escolha DXVK 1.10.3/VKD3D 2.6 se a GPU só tem Vulkan 1.1/1.2 via
  `vulkaninfo`), `runtimes.rs` (API do Lutris: umu, `eac_runtime`, `battleye_runtime`, `current_version`), `dxvk.rs`
  (`DXVK.installRemove`: copia DLLs para system32/syswow64, overrides `native,builtin` via `reg add`, `current_<tool>`,
  nvngx.dll para DLSS; restore = `reg delete` + remoção + `wineboot -u`; só DLLs, ignora os `*-tests.exe`), `net.rs`
  (`Net`: online, JSON, download; `extract_tar`; `FakeNet` para testes). `wine.rs` ganhou `prepare_for_launch` e
  `wine_tools.rs` `wine_command`.
- **App** (`app/src/runtime.rs`): `GameRuntime::play_skip_version_check` ("Play anyway" do diálogo "Game Needs Update"
  passa `--skip-version-check` ao legendary); serviços de preparo (rede, nuvem via Epic); thread de startup que
  atualiza DXVK/VKD3D/DXVK-NVAPI/winetricks 2,5 s após abrir (no máximo 2x por dia, selo `tools/.heronite-last-update-check`;
  desligada por `HERONITE_DISABLE_IDLE_JOBS=1`). `Prep::toggle_tool` (instalar/restaurar no prefixo) é chamado
  pelos toggles das configurações (ver abaixo).
- **Pontos de ligação em arquivos compartilhados**: `app/src/collection/mod.rs` (um braço `LaunchEvent::Status` ->
  `show_toast`), `app/src/collection/epic.rs` (Play anyway -> `play_skip_version_check`), `Cargo.lock` (`ureq` no crate
  launch).
- **Verificação**: 106 testes unitários do crate (tempdirs, `FakeNet`, spawner instantâneo, scripts falsos);
  `tests/runtime.rs` (ordem/eventos/cancelamento com `FakeSpawner`); `tests/launch_flow.rs` lança um jogo Epic de
  verdade com processos reais mas `legendary`/`wine` falsos (script) e confere a ordem: saves ↓, `wineboot --init`,
  `reg add` do DXVK, jogo, saves ↑, e o `--skip-version-check`; `tests/real_network.rs` (`--ignored`) baixou de
  verdade DXVK/VKD3D/NVAPI/winetricks/umu/EAC/BattlEye para um tempdir e o layout é idêntico ao de
  `~/.config/heroic/tools`; o app abre na fixture `/tmp/heronite-fixture-launch` e a atualização de startup instalou as
  ferramentas lá.
- **Não verificado / limitações**: nada rodou com wine/proton/umu/legendary reais nem com um jogo; o toast de status só
  foi coberto por teste (sem captura); o `legendarySetup` do Heroic (redistribuíveis/pré-requisitos do Epic, EOS overlay,
  `installed_games` do prefixo) e os `installFixes` (winetricks/`runInPrefix` do known_fixes) **não** foram portados;
  CrossOver/Mac fora de escopo; o upload de saves após o jogo mantém o jogo como "em execução" na UI até terminar;
  `isOnline` é um teste TCP em IPs fixos (o Heroic usa o monitor do Electron).
- **Toggles DXVK / DXVK-NVAPI / VKD3D** (`app/src/dialogs/heroic_settings/wine.rs`, `app/src/runtime.rs::toggle_tool_blocking`,
  `crates/launch/src/dxvk.rs::toggle_action`): ao salvar o toggle, instala (ligou) ou remove (desligou) a ferramenta no
  prefixo numa thread, com o progresso na linha de aviso da aba; só Wine para DXVK, VKD3D/NVAPI exceto Proton/CrossOver (as
  mesmas condições dos componentes do Heroic); se falhar o toggle volta atrás, como no Heroic. Testado só a lógica de decisão
  (tempdirs, sem prefixo real); a interface não foi exercitada com cliques. O upload de saves pós-jogo continua mantendo o
  estado "jogando" até acabar (o toast "Uploading cloud saves..." já avisa), sem mudança.

## Sistema: lançar de fora, atalhos, bandeja (agente "system")

Código em `crates/shortcuts` (atalhos `.desktop` e `shortcuts.vdf` da Steam, sem GPUI) e `app/src/system/**`
(`cli`, `single_instance`, `hub`, `tray`, `settings`, `shortcuts`), mais `app/src/collection/system_glue.rs`.

- **Lançar sem a UI** (`heronite-rs --launch <runner>_<app>` | `--launch <runner> <app>` | `--launch <app>`, URLs
  `heroic://launch/[runner/]app`, `heroic://launch?appName=..&runner=..[&arg=..&altExe=..&gui=false]` e as mesmas com
  `heronite-rs://`; `--no-gui`; `--help`). Parser em `system/cli.rs` (port de `protocol.ts`). Sem runner, procura em
  legendary, gog, nile, sideload, zoom. `arg=`/`altExe=` são lidos mas **ignorados** (o runtime não tem essa opção ainda).
- **Instância única**: socket unix `$XDG_RUNTIME_DIR/heronite-rs-<hash da pasta de dados>.sock` (a pasta de fixture nunca
  fala com a instância do usuário). Segunda invocação envia uma linha JSON e sai; socket velho (crash) é recolhido.
  `HERONITE_NO_SINGLE_INSTANCE=1` desliga (o `shot.sh` já define).
- **Janela/ciclo de vida** (`system/hub.rs`, `QuitMode::Explicit`): a janela é criada por função (`show_window`) e destruída
  ao fechar; o runtime de jogos é único e compartilhado (`attach_runtime`), então fechar a janela com jogo rodando não
  perde o estado (eventos `Exited` perdidos são reentregues à próxima janela). Regras: `--no-gui`, `gui=false` ou
  `hideWindowOnProtocolLaunch` => não mostra janela e lança direto (headless; legendary com `--skip-version-check` como
  o Heroic em `--no-gui`); se o app foi iniciado só para isso e nenhuma janela foi mostrada, **sai quando o jogo termina**
  (o Heroic só faz isso com `--no-gui`; aqui vale também para URL/atalho, porque o Steam precisa que o processo dure o jogo).
  Jogo não instalado => mostra a janela e abre o fluxo de instalar. Com janela visível, a requisição vai pelo botão
  Play da Collection (prompt de update Epic, escolha de ROM...). Com tray vivo, uma requisição "hide" fecha a janela aberta.
  Código de saída 1 se o jogo não foi achado / falhou.
- **Bandeja** (`ksni` 0.3, API bloqueante, `async-io` — mesmo executor do zbus que o gpui já usa): ícone `icon-light/dark.png`
  do Heroic (`darkTrayIcon`), menu = jogos recentes (`store/config.json` `games.recent`, até `maxRecentGames` ou 5),
  Show/Hide, Quit; clique esquerdo = alterna (com `exitToTray`) ou mostra. Suporta `exitToTray` (fechar a janela a destrói e
  o app continua — libera a memória da janela), `startInTray`, `noTrayIcon`, `minimizeOnLaunch`, `darkTrayIcon`; sem
  watcher StatusNotifier o tray não sobe e tudo isso é ignorado (fechar a janela encerra). `HERONITE_NO_TRAY=1` desliga.
- **Atalhos** (`crates/shortcuts`, `system/shortcuts.rs`): `.desktop` em `~/Desktop` (XDG) e `~/.local/share/applications`
  (`Exec=<binário> --launch <runner> <app>`, ícone `icons/<app>.(jpg|png)` do Heroic ou PNG 256 gerado da capa); "Add to Steam"
  em `userdata/<id>/config/shortcuts.vdf` de **todos** os usuários (VDF binário genérico, re-serialização byte a byte; entrada
  no layout nativo da Steam; `Exe="<binário>"`, `LaunchOptions=--no-gui "heronite-rs://launch?appName=..&runner=.."`; arte
  em `grid/` `<id>p.jpg`, `<id>.jpg`, `<id>_hero.jpg`, `<id>_logo.jpg` (PNG transparente como o Heroic); reconhece também entradas
  criadas pelo Heroic (`heroic://launch`)). Com a Steam aberta: ver "Add to Steam com a Steam aberta" no fim. Raízes: `HERONITE_DESKTOP_DIR`,
  `HERONITE_APPLICATIONS_DIR`, `HERONITE_STEAM_DIR`.
- **UI**: itens "Add/Remove shortcuts" e "Add to / Remove from Steam" no menu de contexto (jogos instalados, não Steam-URI;
  estado sondado em segundo plano), toggles na aba General (Hide System Tray Icon, Exit to System Tray, Start Minimized, Use Dark
  Tray Icon, hide window on protocol launch, Add desktop/menu/Steam shortcuts automatically). Instalação Epic concluída chama
  `on_game_added` (toast com o resultado).
- **API para outras features** — `system::shortcuts` (bloqueante; usar `std::thread`/`background_spawn`):
  `on_game_added(&HeroicPaths, runner, app_name) -> Outcome` (respeita as 3 configs; rápido se todas off; o jogo precisa já estar
  nos arquivos da biblioteca) e o helper `spawn_on_game_added(cx, &heroic, runner, app, |this, outcome, cx| ..)` que faz tudo em
  segundo plano e chama o callback na UI só se houver mensagem. `Outcome::summary()` junta as mensagens num toast. Também:
  `add_shortcuts`, `remove_shortcuts`, `shortcuts_exist`, `add_to_steam`, `remove_from_steam`, `is_added_to_steam`.
- **install.sh**: `Exec=... %U`, `MimeType=x-scheme-handler/heronite-rs` e `xdg-mime default` só para `heronite-rs` (não toma `heroic://`).
- **Pontos de ligação em arquivos compartilhados**: `app/src/main.rs` (reescrito: CLI, instância única, `system::init`),
  `collection/mod.rs` (`mod system_glue;`, `attach_runtime` no lugar de `GameRuntime::new`, `runtime,`), `collection/context_menu.rs`
  (1 linha), `services/downloads.rs` (1 bloco no `Finished`), `dialogs/heroic_settings/general.rs` (`system_rows`),
  `app/Cargo.toml` (`heronite-shortcuts`, `dirs`, `ksni`, `tempfile` dev, `image` com jpeg/png), `scripts/install.sh`.
- **Depuração**: `HERONITE_DEBUG_FAKE_LAUNCH=1` (loga em vez de lançar; **sempre use junto com qualquer teste de `--launch`**),
  `HERONITE_DEBUG_START_HIDDEN`, `HERONITE_DEBUG_SHOW_AFTER_MS`, `HERONITE_DEBUG_CLOSE_AFTER_MS` (como se o usuário fechasse),
  `HERONITE_DEBUG_QUIT_AFTER_MS`, `HERONITE_DEBUG_TRAY_CMDS=show@500,toggle@1500,launch:<app>@2000,quit@3000`.
- **Verificação**: 22 testes em `app` (`system::`: CLI/URLs, socket de instância única real em tempdir com socket velho,
  mensagens, configurações, linhas do menu da bandeja, pixmaps ARGB, chaves de busca, mais um e2e opcional com
  `HERONITE_FIXTURE_SYSTEM=<cópia dos dados>` que cria/remove `.desktop`, ícone, entrada no `shortcuts.vdf` de uma Steam falsa e a
  arte, tudo em tempdir) e 21 em `crates/shortcuts` (VDF byte a byte com o arquivo real da Steam e as 7 fixtures do Heroic, add/remove
  com renumeração, Steam aberta, arquivo corrompido, usuário novo, arte existente). Testado ao vivo (fixture, `shot.sh`):
  `--no-gui --launch` sem janela e saindo ao fim; jogo inexistente (saída 1); encaminhamento para instância viva (com e sem
  `--no-gui`); janela fechada com/sem tray (sem tray encerra; com `exitToTray` fica e `Show` recria; PSS 165->151 MB no debug);
  `startInTray`; tray real registrado no quickshell do usuário (ícone por poucos segundos) com Show/Toggle/Quit simulados;
  capturas da aba General e do menu de contexto.
- **Não verificado / limitações**: o menu do tray e o clique no ícone nunca foram exercitados com cliques reais (só os comandos via
  `TRAY_CMDS`); `Add to Steam` só contra Steam falsa em /tmp (nunca a real, que está aberta); lançamento real headless (sem
  `FAKE_LAUNCH`) só aconteceu por acidente (ver relatório); `arg=`/`altExe=` ignorados; ícone de GOG do Heroic (`goggame-*.ico`) não
  usado; `.desktop` da Área de Trabalho não é marcado como confiável (`gio set metadata::trusted`); pós-jogo (Ludusavi/captura)
  com a janela fechada roda quando a próxima janela abre; macOS/Windows fora de escopo.

## Sobras da Collection (agente "collection")

Brief em `docs/agents/collection-extras.md`. Código novo: `app/src/collection/{extras,achievements,sessions}.rs`,
`app/src/dialogs/{playtime,remove_game,ludusavi_backup}.rs`, `crates/data/src/{playtime,sessions,details_url}.rs`,
`crates/metadata/src/{remove,achievements}.rs`, `scrollbar_draggable` em `app/src/ui/mod.rs`.

- **Editar playtime** (menu de contexto, só se o jogo não está rodando): grava `totalPlayed` em `store/timestamp.json` como o
  `setPlaytimeMinutes` (firstPlayed/lastPlayed preservados, `""` se não havia entrada). Heronite só tem isso no menu.
- **Remover da Collection** (só runner `sideload`, confirmação com o texto do Heronite): apaga a entrada em
  `sideload_apps/library.json`, meta/sessões/metadata/arte/playtime e recentes/favoritos/ocultos (`remove.ts`). Os atalhos
  (`.desktop` e Steam não-Steam) saem pelas funções do agente "system" (`system::shortcuts`) antes da remoção.
- **Backup save (Ludusavi)** por jogo (menu de contexto): diálogo que roda `backup_for_game(Manual)` numa thread e mostra
  os mesmos resultados do Heronite (ocupado / sem match / erro / salvo + "Show in folder"). Testes só com runner falso.
- **Histórico de sessões** no painel de foco (coluna lateral, últimas 12, `HH:MM · Heroic/Playnite`). No Heronite isso fica
  na página do jogo; aqui fica no painel. Lê as sessões já carregadas com a biblioteca.
- **Conquistas**: no Heroic só existem para GOG (`gog/games.ts`); cache em `store_cache/gog_achievements.json` (lido na
  hora) ou API `gameplay.gog.com` com o token de `gog_store/auth.json`, numa thread. O token **não é renovado** (só o gogdl
  faz isso; não mexemos no `auth.json`): token vencido = sem conquistas novas. Outros runners: painel oculto, igual ao Heronite.
- **Botão "abrir detalhes"** (e item "Details" do menu): abre no navegador `store_url` (Epic/GOG), senão a página da Steam
  do jogo, senão o primeiro site dos metadados; sem nada, toast "No store page is known for this game.".
- **Busca no flyout de facetas** ("Filter list", substring sem diferenciar maiúsculas; reinicia ao trocar de faceta) e
  **scrollbar arrastável** (lista e painel de foco: arrastar o polegar, clicar na trilha salta). 
- **Steam não encontrada**: instalar/jogar um jogo Steam sem cliente abre o diálogo "Steam not found"; também adicionei
  "Uninstall" (confirma e abre `steam://uninstall`) para jogos Steam instalados, como o Heronite.
- Hooks: `HERONITE_DEBUG_DIALOG=playtime|remove|ludusavi|ludusavi:done|busy|error|steam-missing` (jogo de
  `HERONITE_DEBUG_SELECT`), `HERONITE_DEBUG_SCROLL=list:<0..1>|focus:<0..1>`.
- Verificado: testes unitários (data, metadata incl. servidor HTTP falso, app) e capturas do foco (sessões, conquistas),
  menu de contexto, flyout de facetas e dos diálogos, com cópia de fixture e Ludusavi falso. **Não verificado**: arrastar a
  scrollbar e digitar na busca das facetas (sem cliques; com o workspace oculto o Hyprland para de mandar frames, então
  estados posteriores ao primeiro quadro não aparecem), ícones das conquistas na captura, GOG real (sem login), Ludusavi real.
- Aviso para o agente "system": `app/src/system/hub.rs` tem um `#[test]` que falha ao compilar o binário de testes do
  app (`recursion limit reached while expanding #[test]`: o glob `use super::*` traz o macro `test` do gpui). Importar os
  itens explicitamente no módulo de testes resolve.

## Adicionar / editar jogo sideload (agente "addgame")

Porta o "Add game" do Heroic (`SideloadDialog` + seletores de plataforma e de Wine do `InstallModal`). Programa nativo
Linux, programa Windows (Wine/Proton) ou jogo de navegador entram em `sideload_apps/library.json` do mesmo jeito que o
Heroic grava, e podem ser editados/removidos depois.

- **Onde**: ☰ → "Add game"; menu de contexto → "Edit game" (jogos sideload que não sejam Steam/emulador).
  `HERONITE_DEBUG_DIALOG=add-game[:estado]` e `edit-game:<appName>[:estado]` (estados separados por `+`:
  `windows`, `linux`, `browser`, `shared`, `platform-open`, `wine-open`, `remove`, `picker`, `art=<capa>[,<hero>]`,
  `save`, `remove-now`; `HERONITE_DEBUG_NO_SEARCH=1` não busca imagens na web). Cuidado no zsh: `$APP:remove` vira modificador, use `${APP}`.
- **Código**: `crates/data/src/sideload.rs` (formato dos arquivos, ids, meta do overlay, prefixo Wine, testes),
  `app/src/dialogs/add_game/{mod,logic,picker}.rs` (diálogo, regras puras, busca de imagens), glue em
  `app/src/collection/add_game.rs`.
- **O que grava** (igual ao `addNewApp` + hooks do Heronite): entrada `GameInfo` na ordem de chaves do Heroic
  (`runner, app_name, title, install{executable,platform,is_dlc}, folder_name, art_cover, is_installed, art_square,
  canRunOffline, browserUrl, customUserAgent, launchFullScreen`, `is_linux_native` no fim como o
  `syncLocalExecutablePlatform`); `app_name` = `short-uuid` (UUIDv4 em base58 flickr, 22 chars, conferido contra a lib
  JS); `local_library/library.json` (`applySideloadAppToLocalMeta`: `playniteId heroic_<id>`, `source manual`,
  `launchKind executable`, `remappedExecutable`); `GamesConfig/<id>.json` (prefixo, Wine, bottle) para jogos Windows,
  pelo mesmo `set_game_settings` das configurações. Edição = merge raso `{...atual, ...novo}` (campos desconhecidos,
  posição das chaves e arte já existente preservados). A plataforma escolhida fica fixa via `runAs` do overlay quando
  difere do que a detecção do arquivo diz (ELF/`#!`/AppImage → Linux, MZ/.exe → Windows).
- **Diálogo**: título (sem caracteres especiais, máx. 40), arte quadrada e capa/hero (URL, arquivo local → `file://`,
  clique na imagem ou botões abrem a busca web DuckDuckGo/Bing/Google; "Find art" usa Steam CDN/SteamGridDB (com chave)/
  Bottles; ao sair do título busca sozinho se não há arte), plataforma, executável (pasta de instalação derivada),
  seção Wine (prefixo compartilhado ou `defaultWinePrefixDir/<título>`, versão do Wine), URL/User-Agent/tela cheia
  para navegador, "Run Installer First" (roda um .exe no prefixo escolhido), "Remove game" (usa
  `heronite_metadata::remove::remove_collection_game` do agente collection + opção de apagar o prefixo Wine com as
  checagens do `removePrefix` do Heroic).
- **Atalhos**: após adicionar chama `crate::system::shortcuts::on_game_added` (agente system); mensagens só vão para o log
  (o diálogo fecha antes de haver toast).
- **Verificado**: 15 testes do módulo de dados (bytes preservados num cópia da biblioteca real, ordem das chaves,
  edição, meta, config de jogo, checagens de prefixo) + 8 de lógica do diálogo; capturas do diálogo vazio, Windows
  preenchido (seção Wine), Linux com select aberto, navegador, edição, confirmação de remoção, busca de imagens, menu ☰ e
  menu de contexto; fluxos `save` / `edit save sem mudanças` / `remove-now` rodados no app contra uma cópia da biblioteca
  (adicionar → arquivos esperados; editar sem mexer → arquivos idênticos byte a byte; remover → biblioteca volta aos bytes
  originais).
- **Não verificado**: cliques reais, seletor de arquivos nativo, busca de imagens na web (só o estado vazio), "Run
  Installer First" e a pré-visualização das imagens (o workspace oculto não entrega os quadros tardios; os logs mostram o
  carregamento ok), uso real com Wine/Proton. Sem macOS. O picker do SteamGridDB (grids/heroes da API) não foi portado:
  a busca web ocupa o lugar dele.

## Gerenciador de emuladores (agente "emulators")

Brief em `docs/agents/emulators.md`. Página nova ☰ → **Emulators** (`HERONITE_DEBUG_PAGE=emulators`), o único lugar para
gerenciar emuladores e pastas de ROM. Os arquivos continuam os do Heronite (`local_library/emulators.json` e
`rom-scanners.json`, mesmo formato; campos desconhecidos preservados).

- **Código**: `app/src/pages/emulators.rs` (página), diálogos `app/src/dialogs/{emulator_edit,emulator_detect,rom_folder,
  emulator_confirm}.rs`, backend `crates/emulation/src/{discover,manager}.rs` + adições em `store.rs`, `detect.rs`, `import.rs`.
- **Lista**: um cartão por emulador (nome, tipo native/flatpak/wine, saúde — executável existe / é executável / Flatpak
  instalado —, jogos que o usam, plataformas dos perfis) com Editar, Duplicar, Remover e as pastas de ROM dele.
- **Vários emuladores**: "Add emulator" (catálogo ou Custom) sempre cria uma entrada nova (`insert_user_emulator`, sem o dedup
  do `upsert`), então dá para ter vários do mesmo tipo/executável. Catálogo: escolhe a definição, o nome, os perfis e o
  executável (achado pela detecção, ou seletor de arquivo) vêm preenchidos. Perfis: nome, plataforma, argumentos (placeholders
  `{ImagePath} {ImageName} {ImageNameNoExt} {ImageDir} {EmulatorDir} {CorePath} {Name} {InstallDir}`), extensões, core do
  RetroArch, pasta de trabalho; adicionar/remover perfil ("Add catalog profiles" repõe os do catálogo). Não deixa remover o
  último perfil nem um perfil usado por uma pasta de ROM.
- **Remover emulador** = `removeUserEmulator` do Heronite: tira o emulador e as pastas de ROM dele; os jogos ficam na
  Collection (lançam com o executável gravado neles). A confirmação diz quantos jogos/pastas. Heronite **não** remove jogos
  cuja ROM sumiu, então aqui também não.
- **Detecção** (`discover.rs`): PATH + Flatpak (como antes) + AppImages/binários por nome (pcsx2*, duckstation*, ppsspp*,
  rpcs3*, dolphin-emu/Dolphin*.AppImage, cemu, ryujinx, xemu, melonds, mgba, flycast, mame, scummvm, vita3k, shadps4,
  azahar/citra, retroarch; forks do yuzu — yuzu/suyu/sudachi/eden/citron — viram "custom" com perfil Switch) em
  `~/Applications ~/Apps ~/AppImages ~/.local/bin ~/.local/opt ~/bin ~/Games ~/Emulation ~/Downloads /opt /usr/local/bin` e nas
  pastas (e pastas irmãs) dos emuladores já configurados. "Scan a folder for emulators…" busca recursivamente (profundidade 6,
  60 mil entradas, 6 s, pula `games/roms/saves/bios/...` e pastas com >3000 itens). Casa por nome + bit de execução (ou
  `.AppImage`/`.exe`); **nunca executa nada**. Resultados com checkbox; já configurados (mesmo executável, mesmo com outra
  definição, como o "custom" do PCSX2 atual) aparecem marcados "Already added".
- **Pastas de ROM** (várias por emulador): diretório, perfil, subpastas, juntar discos, escanear no refresh; "Scan now"
  (importa direto, com resumo na página), "Review…" (abre o diálogo de preview/import existente já preenchido), Editar, Remover,
  "Add ROM folder" e "Scan all folders now" (inclui as com auto-scan desligado). Salvar com auto-scan ligado já escaneia.
  Pasta que não existe aparece em vermelho e o scan dela não derruba as outras (`run_rom_scanners` devolve um relatório).
- **Refresh automático**: o botão de refresh do header (`hdr-refresh`) agora roda `run_rom_scanners` fora da thread de UI
  (spinner já existente), recarrega a biblioteca e mostra "Added N new game(s)"; também roda ~4 s depois de abrir a Collection
  (respeita `HERONITE_DISABLE_IDLE_JOBS`); o job ocioso de 90 s continua (backup agendado). Igual ao Heronite
  (`handleRefreshAll` + `scheduleIdleMaintenance`).
- **Settings → Emulators**: virou um resumo + botão "Open emulator manager" (`DialogRequest::EmulatorManager`).
- **"Add emulated games"**: o select de perfil mostra o nome do perfil quando a plataforma é "Other" (`profile_label`).
- **Ganchos em arquivos compartilhados**: `pages/mod.rs` (`Page::Emulators`, `PageEvent::ScanRoms`), `collection/nav.rs` (página +
  uma linha para `ScanRoms`), `collection/header.rs` (refresh → `refresh_all`), `collection/emulation.rs` (`refresh_all`, scan
  de início, `DialogRequest::EmulatorManager`, debug `refresh`), `dialogs/mod.rs` (módulos + `DialogRequest::EmulatorManager`).
- **Debug**: `HERONITE_DEBUG_EMU_PAGE=add[:<def>]|edit[:<id>]|edit-profile[:<id>]|detect|folder|folder-edit|remove|remove-folder|
  notice` e ações que gravam (só com fixture!): `add-save:<def>,<exe>[,<nome>]`, `edit-save:<exe>`, `dup`, `scan-all`,
  `remove-yes`, `remove-folder-yes`, `folder-save:<dir>`; `HERONITE_DEBUG_DIALOG=refresh`; `HERONITE_DEBUG_SYNC=1` faz os
  diálogos carregarem dados na hora (a janela oculta não repinta resultados tardios).
- **Verificado**: 13 testes novos no backend (nomes → catálogo, caminhada limitada, marcação de já configurados, perfis por
  core, insert/replace/duplicate, relatório de scan com pasta quebrada, saúde, contagem de jogos) + 6 no app; teste read-only
  contra o `/data/Games/Emulation` real (acha DuckStation, RPCS3, shadPS4 ×2 e reconhece o PCSX2 configurado, sem entrar em
  `games/`); capturas (workspace 7) da página (1 e 4 emuladores, saúde, pasta inexistente), diálogos add/edit/perfil/RetroArch/
  Flatpak/detect/ROM folder/confirmações/aviso, aba Settings e menu ☰; fluxos reais contra cópias da fixture: add (catálogo e
  custom), duplicar, editar, pasta de ROM com scan imediato (+4 jogos), "scan all", remover pasta e emulador, e o refresh do
  header (+4 jogos, biblioteca 207 → 211).
- **Não verificado / limites**: cliques reais e o seletor de arquivo nativo (sem mouse); toast "Added N new game(s)" na captura
  (frame tardio); `flatpak info` real (Flatpak não instalado aqui); emuladores `.exe` com Wine; macOS/Windows. Nenhum emulador
  ou jogo foi executado. O diálogo "Add emulated games" ainda tem "Detect on this PC"/"Custom…" próprios (só PATH/Flatpak).

## Epic: pré-requisitos, fixes conhecidos, EOS Overlay e Modify Installation (agente "epic2")

Port do que o Heroic faz na **primeira vez que um jogo roda num prefixo** (`prepareWineLaunch` -> `legendarySetup` +
`installFixes`), do EOS Overlay (`legendary/eos_overlay/eos_overlay.ts`) e do `ModifyInstallModal` do Epic.

- **Primeiro lançamento no prefixo** (`crates/launch/src/first_launch.rs`, chamado em `runtime.rs::run_process` depois de
  criar o prefixo e antes de DXVK/VKD3D): o prefixo guarda um `installed_games` (JSON compacto, lista de app names, o mesmo
  arquivo do Heroic; arquivo estragado recomeça). Se o jogo não está nele, é gravado **antes** de qualquer coisa (como o Heroic,
  uma falha não repete) e roda: `reg add HKEY_CLASSES_ROOT\com.epicgames.launcher /f`; os **pré-requisitos** do jogo
  (`manifest.prerequisites` do `legendary info`, só plataformas Windows/Win32, não EA/Ubisoft; `path` relativo à pasta do jogo,
  `args` separados como shell; `\` do manifesto vira `/`); instalador do EA App / Ubisoft Connect de `tools/redist/legendary`
  (só avisa se o arquivo não existe, quem baixa é a instalação de terceiros, ainda não suportada); **EOS Overlay** ligado para o
  prefixo se o overlay está instalado e ainda desligado (só Linux, `legendary eos-overlay info/enable --prefix`, `pfx` no Proton);
  depois os **fixes** do `known_fixes.json` (`epic[app]`): `winetricks -q <verbo>` por verbo (umu: `umu-run winetricks <verbo>`;
  baixa o script se faltar) e `runInPrefix` (programas relativos à pasta do jogo). Cada comando Wine usa o Wine/Proton do jogo
  (`wine_command_for`: mesmo ambiente do `runWineCommand`, com `STEAM_COMPAT_INSTALL_PATH`) e espera o `wineserver --wait`. Toda
  falha é aviso e o jogo inicia. Toasts: "Installing Redistributables (EPIC)", "Applying Winetricks fixes", "Installing
  Redistributables (FIXES)" (textos do Heroic). Parar durante o preparo cancela sem iniciar o jogo.
- **Ponte com o legendary**: o crate `launch` não depende do `epic`, então o `Launcher` recebe um `EpicSetup`
  (`Launcher::set_epic_setup`; implementação `EpicLaunchSetup` em `app/src/services/downloads.rs`, ligada em `app/src/runtime.rs`).
- **known_fixes.json** (`crates/launch/src/known_fixes.rs`): o Heroic o baixa de
  `raw.githubusercontent.com/Heroic-Games-Launcher/known-fixes/main/known_fixes.json` quando o MD5 local difere do
  `knownFixesSha` do `releases-info/.../release-data.json`; o app faz o mesmo 2,5 s depois de abrir (mesma thread da
  atualização das ferramentas, desligada por `HERONITE_DISABLE_IDLE_JOBS=1`), via arquivo temporário validado como JSON e rename
  (um download quebrado nunca troca o arquivo). MD5 próprio (sem dependência nova). As variáveis de ambiente do known_fixes já
  eram aplicadas (`legendary.rs::known_fixes_env`).
- **EOS Overlay** (`crates/epic/src/eos_overlay.rs`, 6 testes): status (`overlay_install.json`), última versão
  (`overlay_version.json`, `legendary status` se faltar), instalar/atualizar (primeiro sem `-y` para ler "Download size", depois
  `eos-overlay install -y --path tools/eos_overlay` com progresso em %), remover, `enable/disable [--prefix]`, `info`
  ("Overlay enabled: Yes/No"). **Página Epic Games**: seção "EOS Overlay" (instalada/não, versão atual e última, Check for
  updates / Update / Uninstall com confirmação / Install / Cancel, barra de progresso). **Menu de contexto** (jogos Epic
  instalados, Linux, não EA/Ubisoft): "Enable/Disable EOS Overlay" (estado lido em segundo plano; sem overlay instalado pergunta
  "The EOS Overlay is not installed. Do you want to install it now?" e, instalado, liga para o jogo). Heroic coloca o toggle por
  jogo no submenu do jogo e a instalação nas configurações avançadas; aqui ficaram no menu e na página Epic.
- **Modify Installation** (`app/src/dialogs/epic_modify.rs`): item "Modify installation" no menu de contexto de jogos Epic
  instalados com `dlcList` (lido do arquivo da biblioteca pelo serviço, porque o modelo enxuto da Collection o descarta). Lista as
  DLCs que a conta possui (`owned_dlc` do `legendary info`) com tamanho (`info` de cada DLC, uma por vez como o Heroic); por DLC:
  Install (entra na fila de downloads com a pasta e a plataforma do jogo base, fecha o diálogo como o Heroic), Uninstall (com
  confirmação, bloqueado enquanto há download rodando) ou Cancel (tira da fila / para o download). Exatamente o `DLCList` do
  Heroic: **o Heroic não tem edição de SDL/componentes para jogos Epic instalados** (só DLCs), então isso não foi inventado.
- **Pontos de ligação em arquivos compartilhados**: `app/src/runtime.rs` (uma linha `set_epic_setup` + chamada do updater de
  known_fixes), `app/src/dialogs/mod.rs` (`pub mod epic_modify`), `crates/launch/src/{plan,wine,runtime,prepare}.rs` (campo
  `PrepSpec::first_launch`, `Launcher::set_epic_setup`, `Prep::notice` público no crate). Nenhum arquivo de `dialogs/settings/**`
  nem `heroic_settings/**`; `collection/mod.rs` não precisou de braço de debug (o `epic_debug_dialog` já pega `epic-*`).
- **Depuração**: `HERONITE_DEBUG_DIALOG=epic-modify:<app>` e `epic-eos-install:<app>`; `HERONITE_DEBUG_EPIC_PAGE=eos-remove`;
  `HERONITE_DEBUG_EPIC_OP=eos-install|eos-remove|eos-enable:<app>|eos-disable:<app>|install-dlc:<dlc>:<pasta>`;
  `HERONITE_DEBUG_MENU=legendary_<app>` abre o menu de contexto.
- **Verificação**: crate launch 122 testes (+ `tests/launch_flow.rs` com processos reais e wine/legendary falsos: ordem
  prefixo -> `reg add` -> pré-requisito -> `runInPrefix` -> DXVK -> jogo -> saves, e a segunda execução não repete nada);
  crate epic 69 testes; capturas com um `legendary` falso (`/tmp/epic2-fake/legendary`, python) e a fixture
  `/tmp/heronite-fixture-epic2`: diálogo Modify (3 DLCs, uma instalada), menu com "Modify installation" e "Enable/Disable EOS
  Overlay", seção EOS (não instalado, instalando 30%, instalado com update e confirmação), prompt "Overlay not installed";
  `install-dlc` enfileirou `install <dlc> --platform Windows --base-path <pasta> --skip-dlcs -y --skip-sdl`;
  `eos-enable` rodou `eos-overlay enable --prefix <prefixo>/pfx`. `legendary eos-overlay info` real (só leitura): "No
  Legendary-managed installation found" quando não instalado.
- **Não verificado / limitações**: nenhum jogo, wine, winetricks ou legendary de escrita reais rodaram (só scripts falsos); o
  formato exato da linha `Overlay enabled:` com o overlay instalado não foi visto (o Heroic só procura "Overlay enabled" + "Yes");
  cliques reais não testados (sem mouse); ligar/desligar global do overlay no Windows e macOS fora de escopo; o estado do overlay por
  jogo no menu é lido uma vez por sessão (o auto-enable do primeiro lançamento só aparece depois de reabrir o app); instalar
  EA/Ubisoft continua sem suporte (o instalador só é usado se já estiver em `tools/redist/legendary`).

## Settings global e jogos ocultos (agente "settings2")

Brief em `docs/agents/global-settings.md`. ☰ → **Settings** agora é uma página (`Page::Settings`, `HERONITE_DEBUG_PAGE=settings[:secao[:aba]]`)
com a configuração GLOBAL do app; a engrenagem do header continua abrindo o diálogo "Collection settings" (Appearance, Screenshots,
Metadata, Emulators, Backup, Save backup). As abas General e Defaults saíram do diálogo (o código foi reaproveitado, não duplicado).

- **Seções** (lista à esquerda, conteúdo à direita): *General* (pastas padrão, bandeja, atalhos, Wine/Proton, caminhos Wine
  customizados — o `HeroicSettingsPanel` com `GlobalView::General`), *Game defaults* (Wine/Other/Advanced/Gamescope sobre
  `config.json`), *Library*, *Downloads* (`maxWorkers`, `altLegendaryBin`, `downloadNoHttps`, `autoUpdateGames`), *Advanced*
  (`disableLogs`, `disableGOGPresence`, abrir pasta de config/logs/dados do app, Clear Cache, copiar settings sem API keys), *About*
  (versão, pastas, legendary, nº de Wine/Proton, SO/CPU/RAM/GPU lidos de `/proc`, `/sys` e `pci.ids`, sem spawnar nada).
- **Hidden games**: o flag "Show hidden games" vive no `ui-state.json` (`showHidden`; o Heroic guarda em localStorage, não dá para
  compartilhar). Ligado, `Library::load_with(paths, true)` mantém os jogos de `games.hidden` marcados (`CollectionGame.hidden`): entram
  em grupos e busca, cartão com capa cinza/translúcida e etiqueta "Hidden", item do menu de contexto vira "Unhide". Atalho no menu do
  filtro de instalação do header ("Show hidden games") e na seção Library. `Library::load` continua sem os ocultos (tray, atalhos etc.).
- **Library**: lista de ocultos (Unhide / Unhide all), favoritos (Remove / Clear), `maxRecentGames` (1–10, default 5; usado pelo tray)
  e "Clear recent". Tudo via `config_games` (read-modify-write do `store/config.json`, formato e chaves desconhecidas preservados).
- **Clear Cache** (`heronite_data::maintenance`): só o que o app reconstrói — `wikigameinfo.json`, `legendary_{library,gameinfo,install_info}.json`
  e o cache de miniaturas do app. Não mexe nos caches de GOG/Amazon, logins, jogos, settings nem dados da Collection. Pede confirmação.
- **UiState::save** agora faz merge (preserva chaves desconhecidas do `ui-state.json`).
- **Ganchos em arquivos compartilhados**: `pages/mod.rs` (`Page::Settings`, `PageEvent::ShowHidden`, `from_debug`), `collection/nav.rs`
  (slot da página, item ☰ Settings navega, `ShowHidden` → `set_show_hidden`, `refresh()` ao voltar), `collection/mod.rs` (`load_all(heroic,
  include_hidden)`, remoção do debug `settings:general|defaults`), `collection/context_menu.rs` (Hide/Unhide), `collection/header.rs`,
  `card.rs`, `data.rs`, `vm.rs`, `dialogs/heroic_settings/{mod,general}.rs` (`GlobalView`, `new_global`), `dialogs/settings/mod.rs`.
- **Debug** (só com fixture!): `HERONITE_DEBUG_SETTINGS_ACTION=unhide:<app>|unhide-all|fav-clear|recent-clear|max-recent:<n>|show-hidden:on|off|clear-cache`.
  Dica: `AppPaths` segue `XDG_CONFIG_HOME`/`XDG_CACHE_HOME`; use-os numa cópia junto de `HERONITE_HEROIC_DIR` para não tocar `~/.config/heronite-rs`.
- **Verificado**: testes (data: show_hidden, `load_with`, listas/clear, manutenção, merge do ui-state; app: snapshot da Library, sysinfo,
  cópia de settings sem chaves); capturas de todas as seções, ☰, menu do filtro, cartões com etiqueta "Hidden" e menu "Unhide"; ações
  rodadas contra a fixture (unhide, unhide all, clear recent, maxRecentGames, show-hidden, clear cache) com diff dos arquivos: só mudam as
  chaves esperadas, indentação (tab / 2 espaços) e chaves extras preservadas; 208 → 214 jogos ao desocultar.
- **Não verificado / limites**: cliques reais e seletor de arquivo (sem mouse); o visual cinza das capas ocultas (as capas desses jogos
  não chegam na janela do workspace oculto; a etiqueta aparece); "Open … folder" (xdg-open) não foi disparado. `autoUpdateGames` e
  `disableLogs` só gravam a chave compartilhada (o Heronite honra; este app ainda não atualiza sozinho nem grava log em arquivo);
  `checkForUpdatesOnStartup`, tema/idioma, Discord RPC, modo console e features experimentais ficaram de fora (não se aplicam aqui).

## Logs de jogo e do launcher + visualizador (agente "logs")

Brief em `docs/agents/logs.md`. Porta o logger do Heroic (`src/backend/logger/**`) e o visualizador (`LogSettings`).

- **Onde ficam** (igual ao Heroic 2.22 / Heronite, então os dois apps enxergam os mesmos arquivos): `$XDG_STATE_HOME/Heroic/logs`
  (`~/.local/state/Heroic/logs`; macOS/Windows como o `getBaseLogPath`). Log de jogo: `games/<appName>_<runner>/launch.log` (+ `.log.old`
  do lançamento anterior). O `GamesConfig/<app>-lastPlay.log` é de versões antigas do Heroic: o app deixou de escrever nele, mas o
  visualizador ainda lista os que existem ("Older Heroic log"). `HERONITE_LOG_DIR=<dir>` troca a base; com `HERONITE_HEROIC_DIR`
  (fixture/testes) os logs vão para `<dir>/logs` e `<dir>/logs/launcher`, nunca para as pastas reais.
- **Log do jogo** (`crates/launch/src/log_writer.rs`, `GameLog` = `LogWriter` do Heroic): o primeiro write arquiva o arquivo anterior em
  `.old`; linhas `(hh:mm:ss) [INFO]:    msg`; termina com `============= End of log =============`. Cabeçalho como o
  `prepareLaunch`: `Launching "Título" (runner)`, `Native?`, `Installed in:`, `System Info:` (CPU/RAM/GPUs/SO; o app injeta o texto com
  `Launcher::set_system_info`), `Game Settings:` (JSON filtrado, chaves de API/token ocultas) + `Stored at:`, notas do plano, avisos do
  preparo (DXVK, prefixo...), `Launching <título>: ENV=... programa args` (variáveis com TOKEN/SECRET/PASSWORD ocultas) e `Game Output:`.
  Depois vem a saída (stdout+stderr do jogo/legendary/umu/wine) bruta, e `Game process exited with code N`. Erros de spawn/preparo viram
  `[ERROR]` e o log é fechado em todos os caminhos (parar antes de iniciar, falha, Steam).
- **`disableLogs` / `verboseLogs`**: `disableLogs` (config.json global) ou `verboseLogs` desligado do jogo (padrão ligado, como no Heroic)
  desligam o log; sem `verboseLogs` o Heroic escreve (forçado) "IMPORTANT: Logs are disabled" + "Enable verbose logs in Game's
  settings > Advanced tab…", e fechar o log escreve o End of log nesse caso. Com `disableLogs` não se escreve nada (nem rotaciona).
- **Sem bloquear nem crescer sem limite**: o pump da saída lê o pipe em blocos de 16 KB e grava direto no arquivo (memória constante);
  o pipe é sempre drenado, mesmo com o log desligado; um log de jogo para em 512 MB (aviso no arquivo, o resto é descartado).
  `Spawner::spawn_logged` é o novo ponto (o `spawn(cmd, path)` antigo continua); `FakeSpawner` o ignora.
- **Log do launcher** (`LauncherLogger`, `app/src/main.rs::env_logger_init`): o `log` crate continua no stderr e também vai para
  `~/.config/heronite-rs/logs/heronite-rs-AAAA-MM-DD.log` (um arquivo por dia, mantém os 7 mais recentes, passa de 32 MB vira `.old`,
  várias instâncias anexam no mesmo). Linha de início com versão/pid, "System Information" numa thread (fora do caminho de
  inicialização), panics também entram. Respeita `disableLogs` (só o aviso forçado). Escolhi esse caminho em vez do `heroic.log`
  porque o `heroic.log` é reescrito/arquivado pelo Heronite ao iniciar.
- **Visualizador** (`app/src/dialogs/log_viewer.rs`, painel reutilizável): lista à esquerda (This app por dia, Heroic/Heronite,
  runners, jogos com seus títulos — launch/install/…, mais recentes primeiro), conteúdo à direita em fonte mono (escolhe uma família
  instalada: o GPUI não resolve `monospace`), linhas coloridas por nível, quebra de linha, scroll virtualizado (`list`) com barra
  arrastável. Lê só o fim do arquivo (512 KB; "Load earlier" ×4 até 32 MB; acima disso, "Open file"), remove ANSI e progresso com `\r`.
  Botões: Latest / Previous run (`.old`), Refresh, Copy, Open folder, Open file (xdg-open).
- **Onde abrir**: ☰ → Settings → **Logs** (`HERONITE_DEBUG_PAGE=settings:logs`, `HERONITE_DEBUG_LOG=launcher|<appName>`,
  `HERONITE_DEBUG_LOG_EARLIER=<n>` aperta "Load earlier" n vezes) e item **View log** no menu de contexto do jogo
  (`LogViewerDialog`; `HERONITE_DEBUG_DIALOG=viewlog` + `HERONITE_DEBUG_SELECT=<runner_app>`). O botão "Open logs folder" de
  Settings → Advanced agora abre a pasta de logs real.
- **Ganchos em arquivos compartilhados**: `pages/settings.rs` (seção Logs, largura maior só nela), `pages/settings/advanced.rs` (pasta de
  logs), `collection/context_menu.rs` (um bloco "View log" + `open_game_log`), `collection/mod.rs` (uma linha no debug `viewlog`),
  `dialogs/mod.rs`, `runtime.rs` do app (`set_system_info`), `crates/launch/src/{runtime,lib}.rs`.
- **Verificado**: testes (launch: formato das linhas, caminhos, rotação `.old`, desligado/forçado, limite, leitura do fim em arquivo
  grande, listagem, launcher log com poda; `tests/game_log.rs` com processos `sh -c` reais: cabeçalho na ordem do Heroic, saída
  completa de ~6 MB, segundo lançamento move o primeiro para `.old`, sem verbose só o aviso, disableLogs não cria nada, erro de
  spawn); capturas (shot.sh, workspace 7, fixture) da página Logs, de um log de 76 MB (fim exibido, "Load earlier"), do diálogo
  "View log" e do item no menu de contexto.
- **Não verificado / limites**: cliques reais (sem mouse; a preservação da posição ao "Load earlier" foi só exercitada pelo hook de
  debug, onde o layout ainda não aconteceu); nenhum jogo real foi lançado; logs de instalação/atualização/reparo (`install.log` etc.) e
  `runners/*.log` do legendary ainda não são escritos por este app (o visualizador os mostra se o Heronite os criou); sem upload de log.

## Auto-updates Epic e tarefas pós-jogo sem janela (agente "updates")

- **Serviço pós-jogo independente da janela** (`app/src/services/session.rs`, entidade GPUI global criada pelo hub): o `hub::on_event`
  entrega todo `LaunchEvent` a ele antes do Collection. Faz, com ou sem janela: chave de save state de jogos emulados
  (`remember_save_state_key`), backup Ludusavi após o jogo (`backup_after_play`) e a sessão de captura de screenshots
  (`CaptureService` agora mora aqui; o Collection só repassa a config e recebe `SessionEvent::Capture/Toast`). Playtime/sessão local e
  upload de cloud save já eram feitos pela thread do launcher (`crates/launch`), então sempre rodaram sem janela. O Collection só
  observa: toast do resultado, `refresh_shots` quando uma captura é salva e `reload()` da biblioteca no `Exited`.
  O hub não guarda mais eventos `late` para reentregar (não é mais preciso).
- **O que o pós-jogo precisa saber** vem do runtime: `GameRuntime` guarda um `SessionInfo` (título, runner, aliases, steam id, ROM
  jogada) quando lança; o serviço lê/retira no `Launching`/`Exited`. Um lançamento headless (`--no-gui`) não sai até o backup
  terminar (`session::busy()` no `maybe_exit_idle`).
- **Notificações do sistema** (`app/src/system/notify.rs`, `notify-rust` que a árvore já tinha pelo crate media; thread própria;
  `HERONITE_DEBUG_NO_NOTIFY=1` só loga): backup salvo (sempre), falha de backup, erro de lançamento e avisos de cloud save (só sem
  janela), "Update/Installation Finished/Failed" da fila de downloads e "Updating X in the background" (sem janela).
  A captura de tela já notificava por conta própria (`feedback.rs`).
- **Auto-update Epic** (`crates/epic/src/autoupdate.rs`, puro e testado + `Downloads` em `app/src/services/downloads.rs`): o
  serviço Epic agora é criado pelo hub no início (não só quando a janela abre), então fila, checagem de updates na partida e
  auto-update funcionam com o app na bandeja (não em lançamento headless de uma vez só). Checagem 20 s após iniciar
  (`checkForUpdatesOnStartup`, padrão ligado; também roda se `autoUpdateGames` estiver ligado) e **periódica a cada 3 h** quando
  `autoUpdateGames` está ligado (extensão: o Heroic só checa na partida e no refresh; `HERONITE_DEBUG_UPDATE_INTERVAL_SECS` para teste).
  Checagem em segundo plano é silenciosa (sem toast de "library refreshed"); offline não checa. Regras do plano
  (`autoupdate::plan`): `autoUpdateGames` desligado, Heronite rodando (`heroic` em /proc — compartilha a fila), offline ou conexão
  medida (NetworkManager `Metered` via `busctl`) bloqueiam tudo; `ignoreGameUpdates` pula o jogo; jogo em execução é adiado e entra
  na fila quando sai (`Downloads::game_running`, avisado pelo hub); já na fila não duplica; update que falhou não é re-tentado
  automaticamente até reiniciar. Leituras de config/biblioteca saíram da thread da UI.
- **Pontos em arquivos compartilhados**: `app/Cargo.toml` (+`notify-rust`), `collection/mod.rs` (subscribe no `Session`, removidas
  as chamadas `shots_launch_event`/`emu_session_ended`), `collection/{emulation,screenshots,system_glue}.rs` (removido o código
  movido; `on_session_event`), `runtime.rs` do app (`SessionInfo`; `HERONITE_DISABLE_TOOLS_UPDATE` desliga só a atualização de
  DXVK/winetricks para testar o resto das idle jobs), `system/hub.rs`.
- **Depuração**: `HERONITE_DEBUG_FAKE_SESSION=<app_name>:<segundos>@<atraso_ms>` simula um jogo rodando (nada é lançado) e dispara
  todo o pós-jogo; `HERONITE_DEBUG_START_HIDDEN` + `..._SHOW_AFTER_MS` para testar sem janela; `HERONITE_LEGENDARY_BIN` com um
  script falso; Ludusavi falso via `binaryPath` + `useInstalledConfig:false` nas settings da fixture.
- **Verificado**: testes (`autoupdate`: plano, prioridade dos bloqueios, valores de `Metered`; `session`: notícia de backup,
  backup após o jogo com Ludusavi falso, Ludusavi desligado/sessão curta, binário ausente). Ao vivo com fixture (shot.sh, ws 7,
  legendary e Ludusavi falsos): sem janela — check na partida, update enfileirado e concluído com notificações; jogo "rodando" na hora
  do check adia o update e o enfileira ao sair (com backup Ludusavi e fim da sessão de captura no mesmo momento); Heronite "rodando"
  (processo `heroic` falso) bloqueia; loop periódico (12 s) re-checa; `--no-gui` só sai depois do backup; janela reaberta depois
  funciona; com janela aberta o toast do backup aparece (captura). Uma notificação real foi enviada uma vez (daemon do quickshell,
  sem erro).
- **Incidente de teste (para registro)**: no primeiro teste de sessão falsa a fixture ainda apontava para o Ludusavi **real** e
  rodou `ludusavi backup` do jogo "Control: AWE" (falhou com "No info for these games", nada foi gravado) e, como a fixture tinha
  captura ligada, registrou/removeu o bind temporário F12 do Hyprland. Depois disso a fixture passou a usar Ludusavi falso e captura
  desligada.
- **Não verificado / limitações**: conexão medida real (só o parser; NetworkManager do usuário não é medido); notificação vista na
  tela (só confirmado que o daemon aceitou); chave de save state de emulador no caminho sem janela (código movido sem mudança, sem
  teste novo); o `heroic://` com argumentos continua ignorado.

## Refactor: helpers de formulário unificados (agente "refactor")
- `app/src/ui/fields.rs` e `app/src/dialogs/form.rs` viraram **um só módulo**, `app/src/ui/fields/` (`text.rs`, `input.rs`,
  `select.rs`, `tags.rs`; todos reexportados por `crate::ui::fields`). Apagados: `dialogs/form.rs`, `controls::select_box`,
  `select_menu`/`select_with_menu`/`text_state`/`textarea_state`. API única: `new_input`/`new_textarea`, `input_box(&i, disabled)`,
  `textarea_box`, `labeled`, `text_field`, `path_field`, `search_box` (era duplicado em web_image e add_game/picker),
  `select_field` (os 3 selects de metadata/game_settings/appearance migraram para ele), `TagField`.
- Pequenas duplicatas fundidas em `ui/controls.rs`: `mini_checkbox` (estava em scan_roms e importado por emulator_detect),
  `page_heading` (epic e emulators tinham a mesma `heading`), `section_title` (downloads e heroic_settings/parts tinham cópias).
- Mudanças visuais intencionais (look único = o de `fields.rs`): nos diálogos de game settings/metadata os inputs passam a usar a
  fonte Rubik e padding 10 (antes 12), o label fica colado ao campo (sem gap de 5.6px) e os selects usam o `select_field` (texto
  branco, menu de largura do campo, itens 16px) em vez do `select_box` ciano. O textarea mantém a fonte do corpo.
- Teste: `HERONITE_DEBUG_SELECT_OPEN=1` abre o menu "Run as" da aba Launch no game settings (só captura).
- **Verificado**: `cargo build` e `cargo test -p heronite` (71 ok, 3 novos de tags); ~55 capturas antes/depois (shot.sh, ws 7, fixture
  copiada com Ludusavi/captura desligados) de game settings (todas as abas), metadata, web image, add/edit game, settings (todas as
  abas), página Settings (todas as seções), Wine, Epic, emuladores, scan ROMs, Playnite, Epic install/import/modify: idênticas
  pixel a pixel exceto as mudanças acima e ruído de carregamento assíncrono (capas, rede, tamanho da janela).
- **Não verificado**: menu aberto do select de Appearance (fica abaixo da dobra do diálogo) e do select Source do diálogo de metadata
  (mesmo código do `select_field`, verificado aberto em Add game e Run as).

## Processos auxiliares e escritas seguras (agente "proc-safety")
- **Um só jeito de lançar helpers**: `heronite_data::proc` (`guard(&mut Command)` = grupo de processos próprio + `PR_SET_PDEATHSIG(SIGTERM)`
  no Linux; `Run` = executa bloqueando, captura saída, `on_line`, cancelamento por `Cancel`/`poll`, timeout, SIGTERM e depois SIGKILL
  do grupo inteiro após a tolerância). Usado por: legendary (`epic/cmd.rs` agora só monta o `Command`), rsync/mv/cp (move), ludusavi
  (`SystemRunner`), tar (wine `install.rs`, `launch/net.rs` → tools/runtimes) e o helper de login do Epic (`pages/epic.rs`, só `guard`).
  Jogos, emuladores e abridores destacados (xdg-open etc.) NÃO usam. PDEATHSIG vale para a *thread* que deu spawn: todos os chamadores
  ficam bloqueados esperando o filho.
- **Mover instalação Epic retomável** (`crates/epic/src/moves.rs`): journal `epic-move.json` na pasta de config do app (via
  `AppPaths::for_heroic`; pasta Heroic de fixture/teste usa `<pasta>.heronite-rs/config/`, nunca o config real) escrito antes de mexer
  em qualquer arquivo, fase `copying` → `recording`, apagado ao concluir ou quando a falha não moveu nada. `resume_interrupted_move`
  decide pelo estado das duas pastas (`decide`): origem completa + destino sem arquivos = nunca começou (só descarta); arquivos nas
  duas = repete o rsync (idempotente) e registra; só destino = `legendary move --skip-move`; nenhuma = descarta e avisa. Tentar o
  mesmo move de novo na mão também retoma (sem o erro "already exists"). Journal inválido (caminhos relativos, perto da raiz, destino
  fora do novo pai...) é descartado sem tocar em nada. Bug corrigido: `mv` aninhava a pasta dentro de um destino vazio já existente.
  Ligado em `Downloads::new` (`resume_interrupted_move` + fila só inicia depois, via `run_op_then`).
- **Pequenos itens**: `WineConfig::set_download_proton_to_steam` usa `json_store::update_value_indented(INDENT_2)`; `first_launch` só
  grava o jogo em `installed_games` DEPOIS do setup (escrita atômica; passo que nem conseguiu rodar = tenta de novo no próximo launch,
  instalador que roda e sai com erro continua sendo só aviso); `json_store` usa temp único (`.<nome>.heronite-rs-<pid>-<n>.tmp`,
  `heronite_data::temps::write_atomic`) e `CacheStore::clear` passa pelo caminho com lock; `shortcuts.vdf` ganha cópia
  `shortcuts.vdf.heronite-bak` antes de cada escrita + temp único + fsync; `.desktop` também com temp único + fsync.
- **Limpeza de temporários no startup**: `heronite_shortcuts::sweep_stale_temps_at_startup(&heroic, &app_paths)` (chamar numa thread
  de fundo no início do `main`). Só remove nomes estritos: `.*.heronite-rs-<pid>[-<n>].tmp` com pid morto ou com mais de 1 h,
  `shortcuts.vdf.tmp` ao lado de um `shortcuts.vdf` e `*.desktop.tmp` (formatos antigos, só se tiverem mais de 1 h).
- **Verificado**: testes unitários (grupo do guard, PDEATHSIG ao morrer a thread, cancelamento mata o grupo inclusive worker em
  background, SIGKILL após a tolerância, timeout; decisões do journal com rsync falso que morre no meio; `mv` sem aninhar; cp-merge;
  varredura só nos nomes certos; backup do vdf; marker de `installed_games` só depois do setup). `cargo test --workspace` (com
  `HERONITE_FIXTURE`, pulando `real_machine_detection_and_ludusavi` que roda o Ludusavi real) verde.
- **Não verificado**: a retomada ao vivo dentro do app (precisaria rodar sem `HERONITE_DISABLE_IDLE_JOBS`); rsync real só em pasta
  temporária pelo teste. O journal inválido/morto de outro jogo não bloqueia novo move, um pendente válido sim (mensagem no erro).

## Desligamento ordenado, jogos independentes e sessão (agente "shutdown")
- **Confirmação ao sair com jogo rodando** (`app/src/dialogs/quit_confirm.rs`, hub): o botão de fechar da janela (`on_window_should_close`,
  antes de fechar) e o "Quit" da bandeja checam `State.running` e `session::busy()`. Sem nada pendente: sai (pelo caminho gracioso).
  Com jogo/pós-jogo: abre "A game is running" na janela principal (mostrada antes se estava escondida) com "Cancel" / "Keep running in
  tray" / "Quit anyway" (sem bandeja: "Keep open" / "Quit anyway"). `exitToTray` ligado continua só fechando a janela, sem perguntar.
  O fecho headless (`--launch`, `maybe_exit_idle`) não muda.
- **Saída graciosa** (`hub::graceful_quit`, idempotente, nunca bloqueia a UI): tira o socket de instância única, desliga a bandeja, grava
  o tempo de jogo dos jogos rodando (`Launcher::checkpoint_all`, em background), pausa a fila Epic pelo caminho de pausa
  (`DownloadManager::pause_for_shutdown`: o elemento continua em `queue` e retoma no próximo start; NÃO usa `shutdown()`, que vira Abort) e
  espera até 5 s o worker ficar ocioso (`is_idle`), depois `cx.quit()`. Watchdog de 15 s força a saída se algum passo travar.
- **Sinais** (`system/signals.rs`, `signal-hook`): SIGTERM/SIGINT/SIGHUP = quit sem diálogo. A thread de sinais decide a escalada sozinha:
  segundo sinal (ou sinal com um quit já em andamento) = `exit(128+sinal)` na hora (130/143/129).
- **Jogos não dependem do launcher** (`launch/runtime.rs`, `log_writer.rs`): stdout/stderr do jogo vão direto para o `launch.log`
  (descritor `O_APPEND` do próprio `GameLog::child_stdio`, `Stdio::null()` se logs desligados), sem pipe nem thread de bombeamento;
  as linhas do launcher (cabeçalho, "exited with code", "End of log") continuam no mesmo arquivo, na ordem. O fim da sessão não depende
  mais do EOF do pipe: `poll()` só devolve "saiu" quando o grupo de processos do jogo inteiro sumiu (mesma semântica do `close` do Node,
  e launchers que fazem fork continuam sendo esperados). Perda conhecida: o teto de 512 MB do log só vale para linhas do launcher.
- **Checkpoints de tempo de jogo** (`launch/record.rs` `Checkpointer`): a cada 2 min (e no quit/sinal) grava `timestamp.json`
  (`lastPlayed`, `totalPlayed` = floor(base + minutos da sessão), base lida uma vez do arquivo) e atualiza no lugar a linha da sessão em
  `sessions.json` (mesmo `startedAt`); `games.recent` é tocado uma vez. `finish()` usa o mesmo `Checkpointer`, então o final SUBSTITUI os
  checkpoints (nada contado em dobro). Crash/kill -9/queda de energia perde no máximo ~2 min.
- **Segunda instância** (`single_instance.rs`, `main.rs`): sem socket / conexão recusada = obsoleto, vira primária como antes. Se o
  primário existe mas não responde em 5 s ou responde erro: NÃO abre outra instância; imprime erro no stderr, manda notificação
  ("Heronite is already running but is not responding") e sai com código 1. O servidor só responde `ok` se a mensagem foi entregue ao
  canal (`err` senão, `closing` depois do quit começar) e o socket é removido no início do quit (`close_active`), sem apagar o de uma
  instância mais nova no `Drop`.
- **Verificado**: testes unitários (checkpoints sem dupla contagem, crash entre checkpoints, `checkpoint_all` com fake spawner, stdio
  direto no arquivo `/proc/<pid>/fd/1`, ordem das linhas, fila Epic pausada no quit e retomada em outro manager, respostas do servidor
  de instância única: timeout/hangup/err/closing, decisão dos sinais, textos do diálogo). Ao vivo, só em fixture
  (`/tmp/heronite-fixture-shutdown`, ludusavi desligado, jogo falso `fakegame.sh`): SIGTERM/SIGINT/SIGHUP na instância própria = log
  "quitting gracefully", sessão gravada, socket removido, jogo continua rodando e escrevendo no log; `kill -9` do app com o jogo vivo;
  checkpoint periódico aos 120 s (sessions.json + `totalPlayed`); app parado com SIGSTOP + segunda invocação = erro, código 1, cerca de
  5 s, nenhuma segunda instância; diálogo capturado com `scripts/shot.sh` (`HERONITE_DEBUG_FAKE_SESSION` +
  `HERONITE_DEBUG_REQUEST_QUIT_AFTER_MS` / `HERONITE_DEBUG_USER_CLOSE_AFTER_MS`).
- **Não verificado**: variante com bandeja do diálogo (não abri bandeja real); clique real no X / Alt+F4 (o gancho do GPUI é o mesmo dos
  `*_AFTER_MS`, mas sem clique); pausa de um legendary de verdade no quit (só `FakeJob`); sessão Steam (checkpoint usa o mesmo ledger,
  não exercitado).

## Paleta de comandos (Ctrl K) — agente "palette"

- **Código**: `app/src/collection/palette/` — `matching.rs` (dobra caixa/acentos, tiers exato > prefixo > início de palavra > substring >
  fuzzy limitado), `sources.rs` (páginas, seções de Settings, ações, jogos, verbos por jogo), `rank.rs` (consulta -> grupos, puro e
  testado), `view.rs` (overlay: entrada, grupos com legenda, linha selecionada com barra de 2px, rodapé com dicas e filtro), `mod.rs`
  (cola com `CollectionView`: abrir/fechar, teclas, executar). `CollectionView::open_palette` é pública para o menu ☰.
- **Teclas**: um interceptador de teclas (`cx.intercept_keystrokes`) pega as teclas antes de qualquer campo: Ctrl K abre/fecha em
  qualquer lugar (inclusive páginas ☰ e com foco na busca); aberta: ↑ ↓ PgUp PgDn, Tab / Shift Tab (filtro Todos/Pages/Actions/Games),
  Enter, Esc; clique fora fecha. Globais com a paleta fechada (e sem diálogo/galeria abertos): Ctrl 1..5 páginas, Ctrl `,` Settings,
  Ctrl N Add game, F5 refresh (mesmo caminho do botão, com scan de ROMs).
- **Fontes**: Go to (6 páginas; seções de Settings só ao digitar ou no filtro Pages), Actions (Add game, Refresh + ROMs, Scan ROMs,
  Check for Epic updates = `Downloads::refresh_library`, Open logs folder, Quit = `system::request_quit` com a confirmação de sempre),
  Games (título + título da loja como nome alternativo). Vazio: 5 jogos recentes + Go to. O melhor jogo do resultado vem com
  "Play X", "X settings", "Open X folder"; também `play <jogo>`, `<jogo> settings`, `open <jogo>` (verbo no começo ou no fim).
  Jogar passa por `primary_action` (caminho normal de lançamento).
- **Ganchos em arquivos compartilhados**: `collection/mod.rs` (`mod palette`, campo `palette`, `Host::new` no construtor,
  `drive_palette` e `render_palette` no render); `system/mod.rs` (reexporta `request_quit`); `pages/settings.rs`
  (`request_section` + global `PendingSection`, consumido em `SettingsPage::new` e `refresh`, para abrir Settings numa seção).
- **Debug**: `HERONITE_DEBUG_PALETTE=<consulta>[@pages|actions|games]` (`-` = vazia), `HERONITE_DEBUG_PALETTE_KEYS=down,tab,...`.
- **Verificado**: 28 testes de palette (matching, ranking, verbos, filtros, 2000 títulos); capturas com `scripts/shot.sh` (fixture
  `/tmp/heronite-fixture-palette`, ludusavi/capturas desligados) da consulta vazia, `wi`, `control`, filtro Games com rolagem por
  teclas; Collection principal não mudou. PSS com a paleta aberta ~+4 a +10 MB (debug, 210 jogos), liberado ao fechar.
- **Não verificado**: o interceptador de teclas com teclado real (só dá para simular via `HERONITE_DEBUG_PALETTE_KEYS`, que usa o mesmo
  `palette_key`); executar cada ação ao vivo (Quit/Play/abrir pasta não foram acionados por segurança); estilo ainda local
  (`palette/style.rs`) — trocar por `glass_panel`/`kbd` da fundação quando ela commitar; entrada no menu ☰ (`nav.rs` é da fundação):
  chamar `this.open_palette(window, cx)` no clique.

## Fundação do design "Vidro" (glass) + menu ☰ — agente "glass foundation"

- **Código**: `app/src/theme.rs` (tokens glass: preenchimentos, bordas, raios `R_*`, textos, toggle/botão/tabs/input), `app/src/ui/glass.rs`
  (novo: `glass_panel`, `glass_menu_surface`, `glass_fullscreen`, `veil`, `set_backdrop`), `ui/controls.rs` (toggle/`toggle_row`/`checkbox`/
  `mini_checkbox`, botões pílula `ButtonKind::{Glass,Primary,Ghost,Danger}` + legados mapeados, `tabs` segmentadas, `card`/`row`/`caption`,
  `kicker`, `kbd`, `close_button`, `nav_pill`, `dialog_title`), `ui/dialog.rs` (`dialog` glass + novo `drawer`), `ui/dropdown.rs` (menus glass),
  `ui/fields/*` (inputs 40px raio 12 com anel de foco, `path_field` "Clear/Browse", select com lista rolável), `collection/nav.rs` (menu ☰ em
  grade de tiles com meta ao vivo, Settings `Ctrl ,` e Command palette `Ctrl K`), `collection/context_menu.rs` (menu de contexto glass),
  `collection/header.rs` (só os toggles dos menus), `docs/UI-CONVENTIONS.md` (seção "Glass design system").
- **Blur**: `CollectionView::glass_image` (collection/mod.rs) pede `Variant::Glass` (já existia no crate de imagens; blur 30px, saturação 1.5,
  ~1/5 da janela, cache no `ImageStore`) e publica o resultado por frame em `ui::glass::set_backdrop` (thread-local). Os painéis pintam a
  imagem alinhada à janela e recortada nos cantos arredondados (`Window::paint_image` aceita sub-retângulo + raios), sob um escurecimento
  (`glass_art_dim`) e o preenchimento translúcido; sem arte, preenchimento quase opaco. Memória (debug, fixture, 3 rodadas, `/tmp/memcmp2.sh`):
  antes Pss_Anon 61/62/65 MB, depois 62/62/66 MB (~+1 MB); Pss total 159-163 MB antes, 160-164 MB depois.
- **Ganchos em arquivos compartilhados**: `collection/mod.rs` (campo `shown_glass`, método `glass_image`, duas linhas no render:
  `set_backdrop` e `sync_nav_meta`); `collection/epic.rs` (`menu_row` agora delega a `context_menu::item`); `collection/extras.rs` e
  `collection/system_glue.rs` (cor/tamanho do ícone dos itens do menu de contexto, uma linha cada).
- **Meta do menu ☰**: contagem de jogos (`lib`), Downloads (`services::try_downloads`: "N active · P%" / "Paused" / "N queued" / "Idle"),
  Epic (`account().logged_in`), Wine padrão e nº de emuladores lidos em segundo plano por `sync_nav_meta` (ao iniciar e ao abrir o menu, no
  máximo a cada 5 s), nada de I/O no render.
- **Verificado** (fixture `/tmp/heronite-fixture-glass`, ludusavi/capturas desligados, `HERONITE_DISABLE_IDLE_JOBS=1`): menu ☰, menu de
  contexto com flyout de status, dropdown Status, diálogo Manage statuses, diálogos Game settings (Images/Wine/Other) e Add game, página
  Settings (General) e Wine Manager, com `animations` ligadas e desligadas; a tela principal da Collection ficou idêntica pixel a pixel à do
  binário anterior (mesma janela 3428x1402); 110 testes passam.
- **Não verificado / limitações**: select aberto e `TagField` (sem gancho de debug), `drawer` (sem chamador ainda), anel de foco dos inputs
  (só compilado; não há como clicar), animação do knob em movimento (só vi estados finais). Páginas e diálogos mantêm layout antigo
  (restyle por tela é trabalho dos próximos agentes); o blur é da arte, não do que há de fato atrás (cards, scrim), por isso o
  escurecimento fixo. Para capturar com `scripts/shot.sh` numa workspace escondida a janela só redesenha depois de uma primeira
  captura (acorda o compositor): usei uma cópia do script com duas capturas, e a fixture precisa de `images-cache` copiado.

## Glass "game": drawer de configurações do jogo — agente "glass-game"

- **Código**: `app/src/dialogs/game_settings.rs` agora usa o `drawer` (primeiro chamador): cabeçalho com capa 52×70 (raio 10), kicker
  "Game settings", título, linha "loja · Installed · tamanho", botão de fechar redondo e abas segmentadas; corpo rolável com cards
  legendados; rodapé "Use defaults" (ghost, só na aba Images: apaga as imagens personalizadas de uma vez, `reset_all_images`) e "Done"
  (primário; na aba Details aparece "Save" antes). Abas Images (capa/fundo em cards com pílulas Change / Search web / Screenshot / Reset),
  Details (cards General / Tags / Links and notes) e Launch (cards Program / How to run it).
- **heroic_settings/** (continua independente do contêiner, usado também por Settings → Game defaults): `parts.rs` ganhou `Rows` (`rows()`,
  `.child`, `.field`, `.field_help`, `.card()`), `toggle_row` agora devolve uma linha de card (label + descrição + switch), `hint`/`help`/
  `heading` no estilo glass; todas as abas (`wine`, `other`, `advanced`, `gamescope`, `saves`, `general`) foram reescritas em cards
  legendados, sem mudar chaves nem escritas. Tabelas de wrappers/env e caminhos Wine personalizados viram linhas de card com ícones redondos.
- **Diálogos aninhados** (`web_image`, `screenshot_art`, `metadata`, `metadata_wizard`, `launch_state`): botões pílula (Cancel = ghost),
  texto de apoio em `help_text`, erros em caixa `danger_fill`, tiles com raio 12, switches do wizard dentro de um card.
- **Verificado** (fixture `/tmp/heronite-fixture-glassgame`, ludusavi/capturas desligados, `HERONITE_DEBUG_DIALOG=gamesettings[:details|:launch|
  :wine|:other|:advanced|:gamescope|:saves]`, jogo Epic e jogo Playnite/Windows): todas as abas, `webimage`, `metadata`, `wizard`. 
- **Não verificado**: cliques (Use defaults, Reset, tools Wine), `launch_state` e `screenshot_art` só compilados (sem gancho de debug que os
  abra com dados), janela estreita (a janela do shot.sh tem tamanho fixo; o drawer usa `min(580, 96% da largura)`).

## Glass: Configurações globais e da Coleção (agente glass-settings)

- `app/src/pages/settings.rs`: a página ☰ → Settings agora é uma folha de vidro (`glass_panel`, raio 26) sobre a arte
  desfocada: coluna esquerda de 230px com título (engrenagem + "Settings") e pílulas (`nav_pill`), conteúdo à direita com
  título 26px + texto de apoio e botão de fechar redondo (volta à Coleção via `PageEvent::Navigate(Collection)`).
  `request_section` / `PendingSection` e os hooks `HERONITE_DEBUG_PAGE=settings:<secção>` continuam iguais.
- `pages/settings/{library,advanced,about}.rs`: legendas + cartões de linhas (toggle, botões pílula, listas de jogos
  ocultos/favoritos como linhas de cartão). General/Downloads/Advanced/Game defaults/Logs só embrulham os painéis de
  `heroic_settings` e o `LogViewer` (não editados).
- `dialogs/settings/*` (diálogo da engrenagem do cabeçalho): modal de vidro mais largo, abas segmentadas, cada aba em
  cartões (sliders com a cor de destaque, campos de caminho/ select de vidro, toggles), rodapé "Done" primário.
  Novo hook de debug: `HERONITE_DEBUG_SETTINGS_TAB=appearance|screenshots|metadata|emulators|backup|saves`
  (com `HERONITE_DEBUG_DIALOG=settings`).
- Verificado com `scripts/shot.sh` em fixture própria (ludusavi e captura neutralizados); sem mudança na Coleção.

## Glass: páginas ☰ e demais diálogos (agente glass-pages)

- **Páginas** (`pages/{downloads,wine_manager,epic,emulators}.rs`): cada uma é uma folha de vidro (`pages::sheet`, novo em `pages/mod.rs`:
  título Rubik 26 + texto de apoio, ações no cabeçalho, botão de fechar redondo `page_close` que volta à Coleção, corpo rolável com
  coluna centrada) com legendas + cartões de linhas. Helpers novos em `pages/mod.rs`: `list_item`, `icon_action`, `status_pill`,
  `tag_pill`, `caption_with`, `empty_row`. Downloads: item em andamento com barra de progresso arredondada, fila e histórico como linhas
  de cartão com capa, ações em botões redondos; confirmação de parar inline (ghost / glass / danger). Wine Manager: busca, abas
  segmentadas, toggle "Install Proton into Steam" em cartão, lista como cartão (versão + pílula Installed/Update available, data,
  tamanho/progresso com barra fina, ações). Epic: conta, biblioteca, importar e EOS Overlay em cartões; login por janela ou código.
  Emulators: um cartão por emulador (nome, pílulas Native/Ready, plataformas, botões Edit/Duplicate/Remove) com as pastas de ROM como
  linhas (Scan now / Review… / Edit / Remove). Nenhum comportamento, dado ou gancho de debug mudou.
- **Diálogos** (`add_game/**`, `emulator_*`, `epic_*`, `log_viewer`, `ludusavi_backup`, `manage_statuses`, `pick_rom`, `playnite_*`,
  `playtime`, `quit_confirm`, `remove_game`, `rom_folder`, `scan_roms`): já usavam `dialog()`, então herdaram o modal de vidro; esta
  passada trocou botões por pílulas semânticas (Cancel = ghost, confirmar = primário, destrutivo = danger, nada de "Yes/No" em
  maiúsculas), listas e toggles em cartões (`card`, `toggle_row`), linhas com raio 10, textos/cores dos tokens do tema (fim de
  `rgb(0xcaf3fd)` e `navbar_*`), pílulas de status, e o log viewer ganhou abas segmentadas Latest/Previous run, cartão de lista e
  cabeçalhos de grupo sem maiúsculas.
- **Verificado** (fixtures `/tmp/heronite-fixture-glasspages[-in]`, ludusavi/capturas desligados, `HERONITE_DEBUG_PAGE`, `_DIALOG`,
  `_EMU_PAGE`, `_EPIC_PAGE`, `_WINE`, `HERONITE_DEBUG_FAKE_DOWNLOAD`): as 4 páginas (Epic logada e deslogada), statuses, playtime, remove,
  ludusavi, steam-missing, scan ROMs, playnite import/merge, epic install/uninstall/modify/message, emulador add/edit/detect/folder/remove,
  add-game (windows, wine aberto, select aberto, browser, remover), log viewer dentro de Settings → Logs. `cargo test -p heronite`: 110 ok.
- **Não verificado**: janela estreita (o tamanho da janela do shot.sh é fixo; as colunas largas usam `max_w` e `flex_wrap`), cliques,
  confirmação de parar download e de remover versão Wine (sem gancho que abra), epic_install/modify com dados reais (legendary sem
  login na fixture só mostra o erro), `pick_rom` e `epic_import` só compilados.

## Settings unificado (agente settings-merge)

- O diálogo "Collection settings" (`dialogs/settings/**`) foi removido e o botão de engrenagem do cabeçalho também: há UM só lugar de
  configuração, a página ☰ Settings (Ctrl ,). A navegação da esquerda agora tem legendas: **App** (General, Appearance, Advanced),
  **Games** (Game defaults, Downloads, Library), **Collection** (Metadata, Screenshots, Backups), **Help** (Logs, About).
  `Section::GROUPS` descreve os grupos; `Section::ALL` segue a mesma ordem.
- As abas do diálogo viraram seções da página, movidas com `git mv` para `pages/settings/{appearance,metadata,screenshots,saves,backup}.rs`
  (estado e gravação adaptados a `SettingsPage`; mesmos arquivos, chaves e formato — `settings.json` via `heronite_data::settings::*` e
  `set_ludusavi_settings`). Backups = dois cartões de uma seção (Ludusavi saves + cópia agendada da configuração do Heroic). O texto de
  apoio de cada aba subiu para o `blurb` da seção. `SettingsPage::saved` guarda o `EffectiveSettings` devolvido e emite
  `PageEvent::LibraryChanged` (a Coleção recarrega, como no antigo `DialogEvent::Changed`); erros aparecem sob o corpo da seção.
- **Aba Emulators**: só resumia emuladores/pastas de ROM e abria o gerenciador, ou seja, duplicava a página ☰ Emulators e não tinha
  nenhuma preferência própria; foi removida sem criar seção em Settings e sem mover nada para `pages/emulators.rs`.
- Ganchos mínimos fora da pasta de settings: `collection/nav.rs` (2 linhas em `ensure_page`: `offer_settings_data` + `refresh(window, cx)`),
  `collection/palette/rank.rs` (teste que contava 7 seções agora usa `Section::ALL.len()`, mais um teste de keywords).
  A Coleção entrega à página o `EffectiveSettings` já carregado e a cadeia de arte do jogo em foco (preview ao vivo do Appearance) pelo
  global `pages::settings::offer` — a página não lê arquivo ao ser criada (só se a biblioteca ainda não carregou). A detecção do Ludusavi
  (`ludusavi --version`) e a leitura do cache de screenshots passaram a rodar só ao abrir a seção (antes rodavam ao abrir o diálogo).
- Paleta Ctrl K: as 11 seções com ajuda e palavras-chave próprias (`Section::keywords`: "blur", "igdb", "ludusavi", "screenshots"...).
- Debug: `HERONITE_DEBUG_PAGE=settings:<general|appearance|advanced|defaults|downloads|library|metadata|screenshots|backups|logs|about>`;
  `HERONITE_DEBUG_DIALOG=settings[:<seção>]` agora navega para a página (preview do Appearance com arte real). `HERONITE_DEBUG_SETTINGS_TAB`
  foi removido.
- **Verificado**: capturas com `scripts/shot.sh` (fixture `/tmp/heronite-fixture-settingsmerge`, Ludusavi com binário falso, captura e chaves
  IGDB zeradas) de Appearance, Metadata, Screenshots, Backups, e da Coleção sem a engrenagem; `cargo test -p heronite`: 110 ok.
- **Não verificado**: cliques/gravações em tempo real (sem como clicar; o código de gravação é o mesmo das abas), janela estreita, o
  wizard "Download metadata…" aberto a partir da página (só compilado; o diálogo é hospedado pela Coleção como antes).
- Resíduo: `DialogEvent::Open`, `DialogRequest::{ScanRoms,EmulatorManager}` ficaram sem construtor (só a aba Emulators os criava);
  gera aviso de dead code em `dialogs/mod.rs`, sem efeito funcional.
- **Library** (pedido extra): as legendas com botão (`pages::caption_with`, agora centrada, com o mesmo espaço acima de uma legenda comum e
  um respiro antes do cartão) não encostam mais no cartão. Listas Hidden games e Favourites: 5 linhas, botão "Expand"; expandida pagina de
  10 em 10 com "21–30 of 34 · Page 3 of 4", setas e "Collapse" (`Pager` em `pages/settings/library.rs`, estado só na página; o índice é
  limitado ao recarregar após Unhide/Remove). Debug: `HERONITE_DEBUG_LIBRARY_PAGER=<hidden|fav>:<página>`. Verificado em fixture com 34
  ocultos e 8 favoritos (capturas das páginas 1 e 3); teste unitário do `Pager`.

## Remoção do Playnite (2026-10-08)

O import/merge/export do Playnite saiu do app: botão ⇄ do topo da Collection, diálogos `playnite_import`/`playnite_merge`,
`collection/playnite_menu.rs` e o crate `crates/playnite`. Nenhum dado foi alterado: os jogos importados
(`sideload_playnite_*`), as sessões com `source: "playnite"`, os metadados vindos do Playnite e os campos `playniteId`
continuam nos arquivos e são lidos pelo código genérico. A fonte "Playnite" deixou de ser oferecida nos seletores de
metadados (nada a alimentava no app em Rust), mas continua nas prioridades salvas, que o Heronite lê.

## Appearance dividida: fundo da Collection x fundo da interface (2026-10-08)

Settings → Appearance agora tem dois grupos independentes (`app/src/pages/settings/appearance.rs`):
- **Collection background** (arte do jogo atrás da lista/detalhes): liga/desliga, Blur, Darkening, Colour intensity, Edge shading continuam em
  `local_library/settings.json` → `background` (chaves e significado iguais aos do Heronite). Novo: **Opacity** (0-100, padrão 100 = visual
  de antes), que fica no `appearance.json` do app Rust (o Heronite não tem esse campo).
- **Interface background** (menus, ☰, menus de contexto, dropdowns, diálogos, gaveta, páginas, paleta): modo **Transparent** (vidro, padrão) ou
  **Solid**; Transparent tem Opacity 50-100 (padrão 84, = `glass_fill`; a escurecida da arte sobe de .5 a 1 acima de 84, então 100 = opaco) e
  Blur 0-60 px em passos de 5 (padrão 30); Solid tem 4 cores (Slate padrão, Graphite, Midnight, Plum). Prévia: um menu de exemplo.
- **Onde fica**: `~/.config/heronite-rs/appearance.json` (`AppPaths::appearance()`, ao lado de `ui-state.json`; para fixtures, `<pasta>.heronite-rs/config/`)
  com `collectionOpacity`, `interfaceMode`, `interfaceOpacity`, `interfaceBlur`, `interfaceSolidTint` (`crates/data/src/appearance.rs`, leitura
  leniente, números limitados, chaves desconhecidas preservadas).
- **Runtime**: `theme::appearance()/set_appearance()` (valor global do processo) alimenta `glass_fill`, `glass_fill_solid`, `glass_art_dim`,
  `page_backdrop` (antes fixo .55), `glass_blur_px`, `glass_shows_art`. Sliders aplicam ao vivo e gravam ao soltar (fora da thread de UI).
- **Independência / memória**: a textura de vidro (`g:`) é gerada sempre que a interface é Transparent (< 100%), mesmo com a arte da Collection
  desligada; Solid ou 100% não gera nem mantém nada (`ImageStore::forget_prefix`), e com a arte da Collection desligada a textura do palco
  (`s:`) também é liberada. Medido (debug, `/tmp/memcmp2.sh`, fixture): Pss_Anon 62-63 MB padrão vs 60 MB Solid+off (a maior parte da
  textura fica na GPU, não aparece no Pss).
- **Ganchos em arquivos compartilhados**: `collection/nav.rs` (1 linha: fundo das páginas = `theme::page_backdrop()`), `images.rs`
  (`forget_prefix`), `collection/mod.rs` (carrega o `Appearance`, `glass_image`/`stage_image`, e `AppPaths::detect()` -> `for_heroic(&heroic)`,
  que antes gravava o `ui-state.json` REAL mesmo com `HERONITE_HEROIC_DIR` apontando para uma fixture), `pages/settings.rs` (texto e palavras-chave).
- **Verificado**: testes (appearance: padrões, arquivo ausente/corrompido, limites, round-trip com chaves desconhecidas; theme: tokens por
  modo), suíte completa (782 ok), capturas com `scripts/shot.sh` da seção Appearance, Settings com arte ON+Transparent, arte OFF+Transparent,
  arte ON+Solid, diálogo Manage statuses em Solid e a Collection com Opacity 60. **Não verificado**: arrastar os sliders (sem cliques), então o
  caminho ao vivo e o salvamento ao soltar só foram compilados; a prévia "Live preview" da Collection fica vazia quando a página é aberta direto
  por `HERONITE_DEBUG_PAGE` (sem o jogo focado da Coleção), como antes.

## Passada de qualidade de código (2026-10-08)
Refatoração sem mudança de comportamento. Regras novas em `docs/CODE-STYLE.md` (tamanho de arquivos, divisão de views GPUI em
diretórios, o que comentar e o que não comentar, `crate::env` para `HERONITE_*`, clippy zerado) e mapa do projeto em `docs/ARCHITECTURE.md`.
- **Arquivos grandes divididos** (maior arquivo sem testes: 1483 → ~520 linhas): `services/downloads.rs` → `services/downloads/` (10 arquivos),
  `system/hub.rs` → `system/hub/` (7), `collection/mod.rs` (1186 → 333, `render` de ~330 → ~60 linhas) mais `screenshots/`, `epic/`, `focus/`,
  `dialogs/{game_settings,add_game,emulator_edit,scan_roms,log_viewer,epic_install}`, `pages/{emulators,wine_manager,settings/library}`,
  `theme.rs` → `theme/`, `launch/{runtime,plan,log_writer}`, `emulation/{save_states,ludusavi}`, `data/models`, `metadata/service`,
  `media/capture`. Os caminhos usados por outros módulos continuam iguais (`pub use`).
- **Comentários**: réguas `// ----` 177 → 7, removidos rótulos óbvios, histórico ("antes era", "foi removido") e referências a mockups/briefs.
- **Clippy**: 91 avisos → 0; lints do workspace permitem só `type_complexity` e `too_many_arguments` (callbacks GPUI).
- **Código morto**: funções de tema/ui sem uso, `DialogEvent::Open`/`DialogRequest`, 7 ícones sem uso (e os SVGs), dependência `indexmap` do app;
  `ButtonKind` reduzido a `Primary/Glass/Ghost/Danger`; `section_title` → `caption`.
- **Duplicações unificadas**: `text_of` dos diálogos, `js_round` (`heronite_data::num`), `unique()` e timestamp ISO na emulação, superfície de
  vidro (`glass_panel`/`glass_menu_surface`).
- **`crate::env::{flag, value, parse}`**: todas as leituras `HERONITE_*` do app. Diferença só em ganchos de debug/teste: `""` e `"0"` agora
  contam como desligado (antes alguns valiam só por existir).
- **Teste**: `real_data.rs` só executa o `ludusavi` real (somente leitura, `--version`/`config path`) com `HERONITE_TEST_REAL_LUDUSAVI=1`.
- **Verificado**: clippy zerado, suíte completa (782 ok, nenhum teste perdido), capturas da Collection, painel de foco, Game settings,
  Settings → Appearance e Emulators numa cópia neutralizada da fixture.
- **Corrigido**: `sanitize_filename` tinha duas versões (criar atalho em `shortcuts/desktop.rs`, apagar em `metadata/remove.rs`) que
  divergiam em `COM0`/`LPT0` e no caractere DEL; agora as duas usam `heronite_data::text::sanitize_filename` (a do pacote npm).
- **Pendências vistas**: escrita atômica em 3 lugares (`data/temps`,
  `metadata/images`, `images/fetch`); `extract_tar` quase igual em `launch/net.rs` e `wine/install.rs`; erros como `Result<_, String>`
  (67 lugares) misturados com `anyhow`.

## Desempenho: memória, CPU e tamanho (2026-10-08)
Medido com `scripts/idle-cost.sh <binário> <fixture>` (CPU, wakeups por thread, PSS e VRAM via `nvidia-smi`), numa cópia
neutralizada da fixture. Ganchos novos: `HERONITE_DEBUG_BROWSE=<n>@<ms>` (seleciona n jogos seguidos) e `HERONITE_DEBUG_FAKE_TRAY=1`
(age como se houvesse bandeja, para medir a janela fechada sem pôr um ícone na barra).
- **Memória ao navegar** (60 jogos): anônima 248 → 62 MB. O cache de texturas (`app/src/images.rs`) agora tem orçamento de 48 MB em
  bytes (antes: 160 itens, qualquer tamanho) e guarda no máximo 2 fundos de palco e 2 de vidro. A maior parte do ganho veio do
  alocador: `main::tune_allocator` fixa `M_MMAP_THRESHOLD` (128 KiB), `M_TRIM_THRESHOLD` e 2 arenas, então os buffers das imagens
  decodificadas voltam ao sistema ao serem liberados.
- **Janela fechada na bandeja**: anônima 47 → 36 MB (`malloc_trim` logo após fechar). O contexto de GPU do GPUI continua vivo
  (~22 MiB de VRAM e as threads do driver): limitação do GPUI.
- **CPU durante um jogo**: as threads de acompanhamento esperam numa condvar (`heronite_launch::wake::Wake`) em vez de checar
  flags em laço: processo do jogo checado 1×/s (antes 4×/s), o checkpoint de tempo de jogo dorme até o próximo (antes 4×/s), e o
  acompanhamento de jogos da Steam deixou de acordar a cada 25 ms (40×/s). Parar e abortar continuam imediatos.
- **Binário**: 50 → 44 MB com `lto = "fat"` e `opt-level = "s"` só para crates frios (D-Bus, codecs que não usamos, TLS, SVG,
  markdown). Abertura igual (~225 ms até o primeiro quadro). Custo: build release ~9 min (antes ~3).
- **Não mudou**: o fundo do palco já era gerado com no máximo 960 px (a pipeline compensa o desfoque); render da Collection
  ~0,34 ms; vigilância de disco a cada 2 s faz ~12 `stat` (irrelevante).
- **Pendente**: com a janela aberta (e depois de fechada para a bandeja) a thread principal acorda ~100×/s (CPU ~0,07 %). É um
  pingue-pongue entre a thread principal e um worker do GPUI (eventfd do calloop); uma janela GPUI mínima às vezes mostra o mesmo.
  Escondido desde o início: 2×/s.

## Estatísticas e modo console (2026-10-09)
- **Estatísticas** (☰ › Statistics, Ctrl 6): semana / mês / ano / tudo, a partir das sessões já carregadas pela biblioteca
  (`heronite_data::stats`; sessões < 2 min são lançamentos que falharam e ficam de fora). Totais, gráfico de atividade por jogo,
  mais jogados e mapa dia × hora.
- **Modo console** (☰ › Console mode, ou Ctrl K): modo extra para TV e controle, no estilo da proposta "2 · Console". Lembrado
  entre aberturas (`ui-state.json` `consoleMode`); o desktop não muda. Controles lidos com `gilrs` numa thread que só existe no
  modo console (só contam com a janela em foco); o teclado faz as mesmas funções. Biblioteca redesenhada (Início com prateleiras,
  Biblioteca em grade com filtros e busca), página do jogo, menu lateral, e versões console de todas as páginas (Downloads, Wine
  Manager, Epic, Emulators, Statistics, Settings) e dos diálogos usados nesses fluxos (instalar/importar/modificar Epic,
  configurações do jogo, Add game, emuladores e pastas de ROM, confirmações, logs). Teclado na tela e seletor de pastas próprios
  (o seletor do portal não funciona com controle). Convenções em `docs/UI-CONVENTIONS.md` (Console mode).
- **Testes**: `HERONITE_DEBUG_CONSOLE=1|game:<chave>` e `HERONITE_DEBUG_PAD=<sequência>`; capturas com `/tmp/shot-console.sh`
  (acorda a janela no workspace oculto durante a espera). Não testado com um controle físico (nenhum conectado).
- **Fora do console** (precisam do desktop): escolher arte da web nas configurações do jogo (só "voltar ao padrão"), o assistente
  de metadados, editar status, digitar a senha da Epic na janela de login (o código de autorização funciona pelo teclado na tela).

## Add to Steam com a Steam aberta e jogos EA / Ubisoft (2026-10-09)

**Steam aberta** (port de `nonesteamgame/{steamClient,steamUrlHandler}.ts`): a Steam lê `shortcuts.vdf` só ao abrir e o regrava ao
sair, então com ela aberta o arquivo não serve. Ordem do Heroic:
1. **API do cliente** (`crates/shortcuts/src/steam_client.rs`): depurador CEF da Steam em `127.0.0.1:8080` (só existe se o arquivo
   `.cef-enable-remote-debugging` estava na pasta da Steam quando ela abriu; nesta máquina ele já existe). `GET /json` acha o alvo
   `SharedJSContext` e um WebSocket roda `Runtime.evaluate` com `SteamClient.Apps.AddShortcut/SetShortcut*/RemoveShortcut/
   SetCustomArtworkForApp` e a lista de `appStore`/`appDetailsStore` (os mesmos scripts do Heroic). HTTP e WebSocket mínimos escritos
   à mão (TCP local, sem TLS, sem dependência nova); o WebSocket sempre vai ao endereço configurado, nunca ao host que a lista indicar.
   Efeito imediato, sem reiniciar a Steam; a arte vai para o `grid/` de todos os usuários e também pela API.
2. **Sem a API**: cria o arquivo-flag (vale no próximo início da Steam) e usa `steam steam://addnonsteamgame/<wrapper>` com um
   `.desktop` em `~/.local/share/applications/heronite-rs-steam-shortcuts/<runner>-<app>.desktop` (+ `/tmp/addnonsteamgamefile`);
   espera a Steam gravar a entrada (até 10 s) para pôr a arte (`steam_running.rs`). Remover sem a API não dá: erro pedindo para fechar
   a Steam ou reiniciá-la uma vez.
- `is_added_to_steam` pergunta ao cliente quando ele responde. Atalhos casam pelo link de lançamento (nosso e do Heroic) ou pelo wrapper
  (nosso ou `heroic-steam-shortcuts`). No app: `system/shortcuts.rs` (`add_to_steam`, `remove_from_steam`, `is_added_to_steam`).
- Testes: `HERONITE_STEAM_CEF=<host:porta>` e `HERONITE_STEAM_BIN` trocam o depurador e o comando `steam`. 35 testes no crate, incluindo
  um depurador CEF falso (lista, add, remove, arte, exceções, frames grandes), arte para todos os usuários e o URL handler com `steam`
  falso. **Não verificado com a Steam real** (estava fechada).

**EA app / Ubisoft Connect** (port de `thirdParty.ts`, `installEA`/`installUbisoft`, `ThirdPartyDialog`): o diálogo de instalar mostra o
aviso do Heroic e só as opções de Wine (sem tamanho, pasta, DLC ou importar). O job da fila não chama o legendary: baixa
`EAappInstaller.exe` (2 MB) ou `UbisoftConnectInstaller.exe` (264 MB, com progresso e cancelar, `.part` + rename) para
`tools/redist/legendary` e grava `legendaryConfig/legendary/third-party-installed.json` (`[["app","Windows"]]`, sem duplicar). No
primeiro lançamento o instalador roda no prefixo (já existia em `first_launch`) e o legendary abre o jogo com `--origin`/`--ubisoft`;
o plano de lançamento não exige mais pasta de instalação para esses jogos. Desinstalar só tira o jogo do arquivo (o jogo vive no
prefixo; "remove prefix" o apaga). Menu de contexto sem Import / Update / Verify / Move, como no Heroic. Outras lojas
(`thirdPartyManagedApp` desconhecido) continuam sem suporte, com mensagem clara. O leitor enxuto da biblioteca agora mantém
`thirdPartyManagedApp` em `GameInfo::rest`.
- Verificado (fixture `/tmp/heronite-fixture-3p`, legendary falso): diálogo desktop e console, menu antes e depois de instalar, job real
  do EA (baixou o instalador verdadeiro, gravou o arquivo, biblioteca reconstruída com o jogo instalado, nenhuma chamada ao legendary)
  e uninstall. **Não verificado**: rodar o instalador da EA/Ubisoft no Wine e jogar (nenhum jogo foi lançado).

