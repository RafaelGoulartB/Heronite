Answers of the services the example extensions (`extensions/metadata/`) ask, served
by the tests' fixture server.

- `wikipedia_*.json`: recorded from Wikipedia (en, pt) once.
- `vndb_*.json`: recorded from VNDB's API (`POST /kana/vn`) once and
  trimmed by hand: the title searches with every field the extension
  reads, `vndb_search_fate.json` with a hit's fields only. `vndb_id_v9.json`
  is a real entry whose cover VNDB's users rate explicit; its text and tags
  were rewritten for the tests, and no explicit text or picture is kept.
  `tests/vndb.rs` writes the cases the recordings lack (partial dates,
  formatting codes, kinds of tags and screenshots) itself.
- `dlsite_product_*.json`: recorded from DLsite (`GET /maniax/api/=/product.json`,
  `locale=en_US`) once, for two products rated for all ages, and trimmed to
  the keys the extension reads (no prices, sales or files). `tests/dlsite.rs`
  writes the adult-only product itself, with made-up picture addresses: no
  adult picture or its address is kept.
