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
- [ ] Tema (nord-dark, fontes Rubik/Cabin, ícones)
- [ ] Stage + scrim, header, lista agrupada com cabeçalhos fixos, cards
- [ ] Painel de foco (overview, relacionados, detalhes, HLTB do cache, screenshots)
- [ ] Menu de contexto, diálogos de status e aparência
- [ ] Comparação com `docs/reference`

## Log

- 2026-10-07: workspace criado; GPUI validado; linha de base medida; camada de dados compatível com
  testes de ida e volta nos arquivos reais.
