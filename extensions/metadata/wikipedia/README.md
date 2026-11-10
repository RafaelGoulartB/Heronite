# Wikipedia

A metadata source for Heronite that takes a game's description from its
Wikipedia article: the lead (the first paragraphs), in the interface's
language when Wikipedia has the article in it, else in English. The release
year comes from the article's short description ("2011 video game").

It is asked as a fallback: only when the other sources did not find the game
for sure, or when the user asks it by name.

- Finds a game: a title search of the English Wikipedia, keeping the pages
  described as a video game. A page of the same name is a sure match; one
  whose year is more than a year from the game's is a guess.
- Fills: description, release date (the year).
- Contacts: `*.wikipedia.org` (the English site, and the site of the
  interface's language), at most 5 requests a second.
- Needs: nothing.

Try it:

```sh
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/wikipedia find "Portal 2"
cargo run -p heronite-metadata-ext --example run -- extensions/metadata/wikipedia find "Portal 2" --language pt-BR
```

Text from Wikipedia is under CC BY-SA 4.0.
