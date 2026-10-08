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
- [ ] Diálogos de metadados/arte/detalhes/launch + aba Metadata (agente em andamento)
- [ ] Busca dentro do flyout de facetas; arrastar scrollbar

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

- 2026-10-07 (tarde): UI da Collection montada com dados de demonstração; layout bate com a
  referência quase ao pixel. Agentes em paralelo: lógica de dados (`crates/data`), imagens
  (`crates/images`), lançamento (`crates/launch`).
- 2026-10-07: workspace criado; GPUI validado; linha de base medida; camada de dados compatível com
  testes de ida e volta nos arquivos reais.
