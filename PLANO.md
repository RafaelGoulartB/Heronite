# HeroniteRust — Plano de reescrita da Collection em Rust

Status: em implementação desde 2026-10-07 (ver `PROGRESSO.md`).

> **Revisão de 2026-10-07 (decisão do usuário):** o app usa **os mesmos arquivos do Heronite**
> (`~/.config/heroic`), lendo e gravando no formato do electron-store, para que os dois apps
> convivam na transição. Isso substitui "SQLite" e "importação somente leitura" abaixo.
> Foco em Linux; o código fica portável para um build Windows futuro (GPUI suporta Windows). Fonte: fork `../HeroicGamesLauncher` (overlay em
`src/local-library/**`, `src/frontend/screens/LocalLibrary/**`, ~27,5 mil linhas).

## 1. Objetivo

1. App próprio, sem fork do Heroic.
2. Primeiro marco: a tela **Collection funcionando com paridade visual** com o Heronite atual.
3. Motivo de usar Rust: **menos memória, inicialização mais rápida e melhor desempenho**.

## 2. Linha de base medida (Heronite rodando, 190 jogos)

| Processo Electron | PSS |
|---|---|
| main (Node + todo o backend do Heroic) | 229 MB |
| gpu-process | 135 MB |
| renderer (React) | 139 MB |
| network/utility | 28 MB |
| zygotes + broker | 45 MB |
| **Total (9 processos)** | **≈ 565 MB** |

- Instalação: 409 MB (`~/.local/opt/Heronite`); app.asar 33 MB + 65 MB unpacked.
- Dados da Collection: 89 MB em `~/.config/heroic/local_library` (44 MB heroes, 37 MB arte custom).
- Biblioteca real: 133 sideload (67 Steam URI, 45 executáveis, 21 emulados) + 57 Epic (legendary).
  Nenhum GOG/Amazon/Zoom. Instalados: 19 Linux, 2 Windows, 1 Epic.
- **Tempo de inicialização ainda não medido** (o app estava aberto). A Fase 0 mede.

## 3. Decisão de stack

### UI: nativa em GPU com **GPUI** (framework do Zed) + `gpui-component`

Por quê:
- Renderização em GPU, 1 processo, sem DOM nem JS. Isso atende memória e inicialização.
- API de estilo parecida com Tailwind (`flex`, `rounded`, `shadow`, `bg`, `absolute`, `overflow`):
  o CSS atual se traduz quase linha a linha.
- Tem lista virtualizada, imagens assíncronas com cache, animações, sombras com blur,
  gradiente linear e imagem em escala de cinza.
- Já provado em produção (Zed a 120 fps).

Riscos e mitigação:
- API pré-1.0 e pouca documentação. Fixar a revisão do git e isolar o GPUI atrás de
  componentes próprios (`ui/`).
- Plano B: **Slint** (1.x estável, DSL declarativa, memória muito baixa).
- Plano C (só se a paridade visual for inegociável e o nativo não chegar): **Tauri 2**
  reaproveitando o React/CSS. Não é a recomendação: no Linux usa WebKitGTK, que economiza RAM
  mas costuma renderizar `backdrop-filter` e rolagem pior que o Chromium. Isso vai contra o
  objetivo de desempenho.

### Resto da stack

| Área | Escolha |
|---|---|
| Persistência | SQLite (`rusqlite`, bundled) + arquivos de imagem em disco |
| HTTP | `reqwest` em runtime tokio dedicado (IGDB, Steam, Lutris, HLTB, DDG) |
| Imagens | `image` + `zune-jpeg` + `fast_image_resize` (substitui `sharp`/libvips) |
| Steam local | parser VDF (`keyvalues-parser`) para `appmanifest_*.acf` e `libraryfolders.vdf` |
| Playnite | port do leitor LiteDB v4 escrito à mão (`playnite/litedb.ts`, ~350 linhas) |
| Atalho global | portal `GlobalShortcuts` via `ashpd` + caminho Hyprland (`hyprctl`) + `grim` |
| Lixeira | crate `trash` |
| i18n | `fluent` ou tabela própria; importar chaves `collection.*` existentes |

## 4. Arquitetura (workspace Cargo)

```
crates/
  core/       tipos de domínio: Game, GameMeta, Status, Session, Settings, Art (sem UI)
  store/      SQLite + migrações; diretórios de cache de imagem
  import/     importador do Heroic/Heronite (somente leitura) e leitor Playnite LiteDB
  sources/    Steam local/web, Epic (legendary), emuladores, IGDB, Lutris, HLTB, busca de imagem
  launch/     Steam URI, nativo, umu/Proton, emulador, legendary; rastreio de processo e playtime
  images/     decodificar → redimensionar → cinza/blur → cache em disco → LRU de texturas
app/          UI GPUI: theme.rs (tokens), ui/ (componentes), screens/collection/*
```

Regras:
- A UI nunca faz I/O. Tudo passa por serviços em `core`/`store`, que emitem eventos.
- A biblioteca inteira cabe em memória (190 jogos, centenas de KB). Carrega do SQLite em <20 ms.
- Chaves de jogo continuam `runner:appName`, para reimportar do Heroic sem conflito.

## 5. Coexistência com o Heroic durante a transição

- **Importação somente leitura** (nunca escrever nos arquivos do Heroic). Fontes:
  - `local_library/*.json`
  - `sideload_apps/library.json`
  - `store_cache/legendary_library.json`
  - `store/timestamp.json` (playtime)
  - `store/config.json` (favoritos, ocultos, recentes)
  - `GamesConfig/*.json` (argumentos e prefixos)
  - arte em `local_library/{art,heroes,metadata-art}`
- **Botão "Reimportar do Heroic"** enquanto os dois apps convivem.
- **Jogos Epic:**
  - Lançados com o binário `legendary` existente (`LEGENDARY_CONFIG_PATH` apontando para
    `~/.config/heroic/legendaryConfig/legendary`).
  - Instalar e desinstalar Epic fica no Heroic (botão "Instalar no Heroic" via `heroic://`)
    até uma fase posterior.
- **GOG, Amazon e Zoom ficam fora do escopo** (biblioteca atual não tem nenhum).

## 6. Estratégia de paridade visual

1. **Tokens extraídos do CSS** para `theme.rs`:
   - cores do tema midnightMirage
   - cores de status
   - raios, sombras e tamanhos (grade 3×125 px, coluna de 27rem, breakpoints 1100 e 780 px)
   - fontes Rubik e Cabin embutidas
   - ícones Material como SVG
2. **Screenshots de referência:** capturar o Heronite em 1920×1080, 1280×800 e 1000×700, com o
   mesmo jogo focado. Comparar com o app Rust por diff perceptual (`dssim`) a cada tela.
3. **Mapa de efeitos CSS → nativo:**

| Efeito no Heronite | Implementação em Rust | Fidelidade |
|---|---|---|
| Stage com `backdrop-filter: blur(6px) saturate(.94)` | Hero pré-borrado na CPU em ½ ou ¼ da resolução, em cache por jogo e por raio | Idêntica e mais barata |
| Painéis de vidro (blur 18/22 px) | Mesma textura do stage mais borrada, recortada no retângulo do painel em coordenadas da janela | Quase idêntica (o fundo é estático) |
| Scrim de 3 gradientes multi-stop | Textura de gradiente gerada uma vez, ou várias camadas de 2 stops | Idêntica |
| `grayscale()` em capas não instaladas + hover de 200 ms | Variante cinza gerada na decodificação; crossfade de opacidade cor/cinza | Idêntica (também resolve o cinza parcial do progresso de instalação) |
| Blur do cabeçalho de grupo fixo e do overlay da galeria (conteúdo dinâmico atrás) | Gradiente escuro sem blur real | **Diferença pequena e aceita** |
| `mask-image` no fade das tags | Clipe + gradiente na cor do painel | Quase idêntica |
| `text-shadow` | Verificar suporte no spike; senão confiar no scrim | A validar |
| Fade-in de 500 ms das imagens, barra de hover, pulso "jogando", spin | Animações do GPUI com easings `cubic-bezier` | Idêntica |
| Container queries | Breakpoints por largura do painel no layout | Idêntica |
| Navegação de gamepad | `gilrs` + navegação espacial própria na grade (melhor que hoje) | Superior |

## 7. Pipeline de imagens (onde a memória realmente se decide)

- Capas decodificadas no **tamanho de exibição** (250×375 @2x ≈ 375 KB RGBA) e não no original.
  Cache em disco de miniaturas prontas, para não redecodificar JPEG grande a cada boot.
- **LRU de texturas na GPU** só para as capas visíveis ± margem (~40 capas ≈ 15 MB).
  Hoje o React nunca desmonta um card já renderizado.
- Hero borrado guardado em baixa resolução (960×540 ≈ 2 MB contra 8 MB em 1080p cheio).
- Screenshots: miniaturas de 320×180 q60, no máximo 2 jobs, LRU de 500 MB em disco
  (mesmo comportamento atual).

## 8. Fases

Estimativas em semanas de trabalho focado.

| Fase | Escopo | Estimativa |
|---|---|---|
| **0. Spike e medição** | Medir o startup do Heronite. Protótipo GPUI com dados reais: stage borrado, lista agrupada, grade e painel de foco estático. Medir PSS, tempo até o primeiro frame e fps de rolagem. **Gate go/no-go** (abaixo). | 1–2 |
| **1. Dados** | `core`, `store` (SQLite), importador do Heroic, testes com cópia do `~/.config/heroic` real | 1–2 |
| **2. UI da Collection (paridade)** | Header (busca, filtros de instalação e facetas, ordenação, agrupar por status), grupos fixos colapsáveis, cards, painel de foco completo (overview com HTML da Steam sanitizado, relacionados, detalhes com chips-filtro, conquistas em cache, HLTB em cache), menu de contexto, diálogo de status, aparência nas configurações | 3–4 |
| **3. Jogar** | Steam URI, nativo, emuladores (catálogo de 17), `.exe` via umu-run/Proton, Epic via legendary; rastreio de processo, sessões, playtime, "forçar parar de jogar", tail do log da Steam | 2 |
| **— Marco 1: Collection usável no dia a dia —** | | **~7–10** |
| 4. Metadados e arte | IGDB, Steam, Lutris, merge por prioridade de campo, wizard em lote, diálogo de arte (upload, busca na web por HTTP direto, escolher de screenshot), heroes da Steam, SteamGridDB | 2–3 |
| 5. Screenshots | Faixa, galeria paginada, visualizador com zoom e pan, lixeira, captura por atalho (portal, Hyprland, grim) com notificação | 2 |
| 6. Playnite e emulação avançada | Importar, mesclar e exportar do Playnite; varredura de ROMs; disc-id; save states; Ludusavi; backup agendado | 3–4 |
| 7. Independência total (opcional) | Instalar Epic (legendary), gerenciar prefixos Wine, MangoHud, GameMode e Gamescope | aberto |

### Gate da Fase 0 (critérios para seguir com GPUI)

- PSS ≤ 150 MB com a Collection aberta e 190 jogos.
- Primeiro frame ≤ 400 ms; capas visíveis ≤ 800 ms com o cache quente.
- Rolagem sem quedas a 60 Hz, com folga visível a 120 Hz.
- Diff visual do stage, grade e painel de foco aceito lado a lado.
- Se falhar: Slint; se a paridade continuar falhando, Tauri.

## 9. Estimativa de ganho (migração completa)

| Métrica | Heronite hoje | Rust nativo (estimado) | Ganho |
|---|---|---|---|
| RAM ociosa (PSS) | **565 MB medido**, 9 processos | 90–160 MB, 1 processo | **−70 a −85% (≈4–6×)** |
| RAM rolando e com a galeria aberta | sobe (cards nunca desmontados, imagens em tamanho cheio) | limitada pelo LRU, ~150–250 MB | teto previsível |
| Inicialização até a Collection interativa | estimado 2–5 s (medir na Fase 0) | primeiro frame 0,15–0,4 s; capas em ~0,5–0,8 s | **≈4–10×** |
| Tamanho instalado | 409 MB | 25–50 MB (com fontes e ícones) | **≈10×** |
| CPU ocioso | timers do Chromium e polling do backend do Heroic | ~0% (redesenha só quando algo muda) | pequeno em valor absoluto, mas real |
| Fluidez de rolagem e animação | Chromium já compõe na GPU, roda bem | estável, sem GC e sem relayout de DOM; 120 Hz viável | **moderado** |

Ressalvas:
- **Parte grande dos 565 MB é o Heroic em si** (backend de todas as lojas, tela Library,
  stores), não só o Electron. Um app só-Collection já cairia bastante em qualquer stack.
  O Rust nativo leva isso ao mínimo.
- **O ganho de "desempenho" percebido é o menor dos três.** Chromium é muito bom em grade de
  imagens. O salto grande está em memória, inicialização e tamanho.
- **Memória depende mais do pipeline de imagens do que da linguagem.** A seção 7 é obrigatória
  para chegar na faixa estimada.

## 10. Riscos principais

1. **Escopo:** o overlay tem 27,5 mil linhas e ainda depende do Heroic para Wine/Proton, Epic,
   playtime, HLTB e conquistas. Mitigação: cortes de fase rígidos e importação somente leitura.
2. **GPUI pré-1.0:** mudanças de API. Mitigação: fixar a revisão e usar a camada `ui/` própria.
3. **Wayland e atalhos globais:** o portal tem comportamento irregular no Hyprland. Mitigação:
   portar a lógica atual (bind temporário via `hyprctl` + `grim`).
4. **Busca de imagem na web:** hoje usa Chromium oculto com DevTools. Em Rust vira HTTP direto
   (token `vqd` do DDG), que é mais frágil. Mitigação: fallback Bing/Google e erro claro.
5. **HLTB e APIs externas mudam com frequência:** isolar cada uma em `sources/` com testes de
   contrato.
