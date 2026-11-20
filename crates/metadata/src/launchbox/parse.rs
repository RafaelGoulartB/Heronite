//! The archive's XML read as a stream of flat records: each child of the
//! root (`<Game>`, `<GameAlternateName>`, `<GameImage>`, `<File>`, ...) with
//! the text of its own children. Nothing is held beyond one record, so a
//! 500 MB `Metadata.xml` costs a few kilobytes of memory.

use std::io::BufRead;

use anyhow::{anyhow, Result};
use quick_xml::errors::IllFormedError;
use quick_xml::events::{BytesRef, Event};
use quick_xml::Reader;

/// One child of the root: its element name and its children's texts.
#[derive(Debug, Default)]
pub(super) struct Record {
    pub kind: String,
    fields: Vec<(String, String)>,
}

impl Record {
    /// The text of child `name` (`""` for an empty one), trimmed.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.trim())
    }

    /// The text of child `name` when it has some.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.get(name).filter(|v| !v.is_empty())
    }
}

/// The text a general reference stands for (`&amp;`, `&#8217;`).
fn reference(r: &BytesRef<'_>) -> Result<Option<String>> {
    if let Some(c) = r.resolve_char_ref()? {
        return Ok(Some(c.to_string()));
    }
    Ok(match r.decode()?.as_ref() {
        "amp" => Some("&".into()),
        "lt" => Some("<".into()),
        "gt" => Some(">".into()),
        "quot" => Some("\"".into()),
        "apos" => Some("'".into()),
        _ => None,
    })
}

/// Calls `each` with every record of the document, in order, and returns how
/// many it left out. Stops at the first error (`each`'s, or a document that
/// is not well formed or ends early: a truncated file never passes for a
/// whole one). A record whose text alone does not read (a reference left
/// open, a character reference to no character, bytes that are not text) is
/// left out: the reader carries on past it in step with the elements.
pub(super) fn each_record(input: impl BufRead, mut each: impl FnMut(&Record) -> Result<()>) -> Result<u64> {
    let mut reader = Reader::from_reader(input);
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut record = Record::default();
    let mut depth = 0usize;
    let mut field = String::new();
    let mut text = String::new();
    let mut ended = false;
    // The record being read has text that does not read.
    let mut bad = false;
    let mut skipped = 0;
    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            // The reader has consumed the reference and goes on in step.
            Err(quick_xml::Error::IllFormed(IllFormedError::UnclosedReference)) if depth >= 2 => {
                bad = true;
                buf.clear();
                continue;
            }
            Err(e) => return Err(anyhow!("XML at byte {}: {e}", reader.buffer_position())),
        };
        match event {
            Event::Start(e) => {
                depth += 1;
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                match depth {
                    2 => {
                        record.kind = name;
                        record.fields.clear();
                        bad = false;
                    }
                    3 => {
                        field = name;
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                match depth + 1 {
                    2 => {
                        record.kind = name;
                        record.fields.clear();
                        each(&record)?;
                    }
                    3 => record.fields.push((name, String::new())),
                    _ => {}
                }
            }
            Event::Text(e) if depth == 3 => match e.xml10_content() {
                Ok(t) => text.push_str(&t),
                Err(_) => bad = true,
            },
            Event::CData(e) if depth == 3 => match e.decode() {
                Ok(t) => text.push_str(&t),
                Err(_) => bad = true,
            },
            Event::GeneralRef(r) if depth == 3 => match reference(&r) {
                Ok(Some(s)) => text.push_str(&s),
                Ok(None) => {}
                Err(_) => bad = true,
            },
            Event::End(_) => {
                match depth {
                    3 => record.fields.push((std::mem::take(&mut field), std::mem::take(&mut text))),
                    2 if bad => {
                        log::warn!("LaunchBox archive: a {} record whose text does not read is left out", record.kind);
                        skipped += 1;
                    }
                    2 => each(&record)?,
                    1 => ended = true,
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !ended {
        return Err(anyhow!("the XML ends before its root element does"));
    }
    Ok(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(xml: &str) -> Result<Vec<(String, Vec<(String, String)>)>> {
        let mut out = Vec::new();
        each_record(xml.as_bytes(), |r| {
            out.push((r.kind.clone(), r.fields.clone()));
            Ok(())
        })?;
        Ok(out)
    }

    #[test]
    fn records_with_entities_and_empty_children() {
        let xml = "<?xml version=\"1.0\" standalone=\"yes\"?>\n<LaunchBox>\n  <Game>\n    <Name>Ren &amp; Stimpy&#8217;s</Name>\n    <Genres />\n    <Overview>Line one\r\nLine two</Overview>\n  </Game>\n  <GameImage><DatabaseID>5</DatabaseID></GameImage>\n</LaunchBox>\n";
        let got = records(xml).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "Game");
        assert_eq!(got[0].1[0], ("Name".to_string(), "Ren & Stimpy\u{2019}s".to_string()));
        assert_eq!(got[0].1[1], ("Genres".to_string(), String::new()));
        assert_eq!(got[0].1[2].1, "Line one\nLine two");
        assert_eq!(got[1], ("GameImage".to_string(), vec![("DatabaseID".to_string(), "5".to_string())]));
    }

    #[test]
    fn a_record_whose_text_does_not_read_is_left_out_alone() {
        let xml = "<LaunchBox>\n  <Game><Name>A</Name></Game>\n  <Game><Name>Ren & Stimpy</Name><DatabaseID>2</DatabaseID></Game>\n  <Game><Name>Bad &#0; char</Name></Game>\n  <Game><Name>C</Name></Game>\n</LaunchBox>\n";
        let mut names = Vec::new();
        let skipped = each_record(xml.as_bytes(), |r| {
            names.push(r.text("Name").unwrap_or_default().to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!((names, skipped), (vec!["A".to_string(), "C".to_string()], 2));
        // An element closed out of turn is no record's own problem: the build stops.
        assert!(records("<LaunchBox><Game><Name>A</Game></Name></LaunchBox>").is_err());
    }

    #[test]
    fn a_truncated_document_is_an_error() {
        let xml = "<LaunchBox>\n  <Game>\n    <Name>Super Metroid</Name>\n  </Game>\n  <Game>\n    <Na";
        assert!(records(xml).is_err());
        assert!(records("<LaunchBox><Game><Name>A</Name></Game>").is_err(), "no closing root");
    }
}
