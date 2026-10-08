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
- [ ] Busca dentro do flyout de facetas; arrastar scrollbar
- [ ] Testar ao vivo (cliques reais): jogar/parar, salvar diálogos, captura por atalho, Ludusavi
- [ ] Verificação visual final com o workspace 2 visível (ver limitação acima)
- [ ] Conquistas no painel de foco; botão "abrir página de detalhes"
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
- [ ] Faltam: `legendarySetup` (redistribuíveis/EOS overlay), `known_fixes` installFixes, EA/Ubisoft,
      seletor de plataforma, atalhos de desktop.
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
  credenciais); jogos gerenciados por terceiros (EA/Ubisoft) dão erro "not supported yet" (o Heroic só grava
  `third-party-installed.json` no Linux, dentro do `legendaryConfig`); sem seletor de plataforma (só Windows no Linux),
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
  criadas pelo Heroic (`heroic://launch`)). **Steam precisa estar fechada** (checa `steam.pid` como o Heroic; aberta => erro
  explicando; o caminho "via SteamClient/CEF" do Heroic não foi portado). Raízes: `HERONITE_DESKTOP_DIR`,
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
