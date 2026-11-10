# PCGamingWiki

A metadata source for Heronite that reads what PCGamingWiki knows of a PC
game through the wiki's Cargo API (`api.php?action=cargoquery`, see
<https://www.pcgamingwiki.com/wiki/PCGamingWiki:API>).

- Finds a game: by a Steam app id the wiki lists for it (a sure match, when
  the game has a Steam id), else by its page name.
- Fills:
  - **features**: "Full controller support", "Controller support" or
    "Partial controller support", "Ultra-widescreen", "HDR", "Ray tracing"
    (only what the wiki marks as supported);
  - **series**: the first series the wiki names.
  The engine the wiki names is left out: Heronite has no field for it.
- It is an extras source: it never decides which game a game is. Its match
  is used when it is sure (the Steam id) or the same game the details
  sources found.
- Asked about PC games only (`to = "pc"`): never about an emulated game.
- Contacts: `www.pcgamingwiki.com`, at most 2 requests a second.
- Needs: nothing.

Try it:

```sh
cargo run -p heronite-metadata-ext --example run -- extensions/pcgamingwiki find "Portal 2" --steam 620
```

PCGamingWiki's content is under CC BY-NC-SA 3.0.
