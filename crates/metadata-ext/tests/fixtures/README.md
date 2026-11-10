Answers of the services the example extensions (`extensions/`) ask, served
by the tests' fixture server.

- `wikipedia_*.json`: recorded from Wikipedia (en, pt) once.
- `pcgamingwiki_*.json`: written by hand in the shape of PCGamingWiki's
  Cargo API answers (`{"cargoquery": [{"title": {...}}]}`, the aliases the
  script asks for); the wiki answered the recording machine with a
  Cloudflare challenge. `HERONITE_LIVE=1 cargo test -p heronite-metadata-ext
  --test live` checks the script against the real service.
