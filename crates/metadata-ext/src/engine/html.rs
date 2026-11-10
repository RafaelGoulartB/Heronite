//! `ctx.html.parse(text)`: a read-only document (`dom_query`, html5ever and
//! the CSS selectors of Servo) the script asks with CSS selectors. The
//! documents stay on the host side; a script holds handles.
//!
//! A node (the document too) has `text(selector?)`, `attr(selector, name)`
//! or `attr(name)`, `all(selector)` and `html()`.

use dom_query::{Document, NodeId, Selection};
use rquickjs::function::Opt;
use rquickjs::{Array, Ctx, Function, Object, Value};

use super::convert::text_within;
use super::host::{type_error, Host};
use std::rc::Rc;

/// A document larger than this is refused (what the call downloaded is
/// capped anyway; this bounds a string the script built).
const MAX_HTML: usize = 4 * 1024 * 1024;
/// Documents one call may parse, and their bytes together: they stay in the
/// host's memory, outside the engine's limit, until the call ends.
const MAX_DOCS: usize = 8;
const MAX_PARSED: usize = 8 * 1024 * 1024;
/// Tags (`<`) of the documents of one call together: what bounds their
/// nodes, a few hundred bytes each (`"<br>".repeat(1e6)` is a million).
const MAX_TAGS: usize = 200_000;
/// Nodes `all` returns at most.
const MAX_ALL: usize = 500;
/// Handles one call may hold.
const MAX_NODES: usize = 20_000;

/// The documents of the current call and the nodes handed to the script.
#[derive(Default)]
pub(super) struct Docs {
    docs: Vec<Document>,
    nodes: Vec<(usize, NodeId)>,
    /// Bytes and tags parsed in the call.
    bytes: usize,
    tags: usize,
}

impl Docs {
    fn node(&self, handle: usize) -> Option<Selection<'_>> {
        let (doc, id) = *self.nodes.get(handle)?;
        self.docs.get(doc)?.tree.get(&id).map(Selection::from)
    }

    fn add(&mut self, doc: usize, id: NodeId) -> Option<usize> {
        (self.nodes.len() < MAX_NODES).then(|| {
            self.nodes.push((doc, id));
            self.nodes.len() - 1
        })
    }
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn parse<'js>(ctx: &Ctx<'js>, host: &Rc<Host>, text: &rquickjs::String<'js>) -> rquickjs::Result<Value<'js>> {
    let text = text_within(text, MAX_HTML)?.ok_or_else(|| type_error(ctx, "the document is too large"))?;
    let handle = {
        let mut docs = host.docs.borrow_mut();
        if docs.docs.len() >= MAX_DOCS {
            return Err(type_error(ctx, &format!("more than {MAX_DOCS} documents in one call")));
        }
        let tags = text.bytes().filter(|b| *b == b'<').count();
        if docs.bytes + text.len() > MAX_PARSED || docs.tags + tags > MAX_TAGS {
            return Err(type_error(ctx, "the documents of one call are too large"));
        }
        docs.bytes += text.len();
        docs.tags += tags;
        let doc = Document::from(text.as_str());
        let root = doc.root().id;
        docs.docs.push(doc);
        let index = docs.docs.len() - 1;
        docs.add(index, root)
    };
    match handle {
        Some(h) => node_object(ctx, host, h),
        None => Err(type_error(ctx, "too many nodes in one call")),
    }
}

/// The first match of `selector` under `node`, or `node` itself without one.
fn pick<'a>(ctx: &Ctx<'_>, node: Selection<'a>, selector: Option<&str>) -> rquickjs::Result<Option<Selection<'a>>> {
    match selector {
        None => Ok(Some(node)),
        Some(sel) => {
            let found = node.try_select(sel).ok_or_else(|| type_error(ctx, &format!("invalid or unmatched selector \"{sel}\"")));
            // `try_select` gives `None` for a selector that does not parse
            // and for one that matches nothing: tell them apart.
            match found {
                Ok(s) => Ok(Some(s.first())),
                Err(e) if dom_query::Matcher::new(sel).is_err() => Err(e),
                Err(_) => Ok(None),
            }
        }
    }
}

fn node_object<'js>(ctx: &Ctx<'js>, host: &Rc<Host>, handle: usize) -> rquickjs::Result<Value<'js>> {
    let obj = Object::new(ctx.clone())?;

    let h = host.clone();
    let text = Function::new(ctx.clone(), move |ctx: Ctx<'js>, selector: Opt<String>| -> rquickjs::Result<Option<String>> {
        let docs = h.docs.borrow();
        let Some(node) = docs.node(handle) else { return Ok(None) };
        Ok(pick(&ctx, node, selector.0.as_deref())?.map(|s| squash(&s.text())))
    })?;
    obj.set("text", text)?;

    let h = host.clone();
    let attr = Function::new(ctx.clone(), move |ctx: Ctx<'js>, a: String, b: Opt<String>| -> rquickjs::Result<Option<String>> {
        let docs = h.docs.borrow();
        let Some(node) = docs.node(handle) else { return Ok(None) };
        let (selector, name) = match b.0 {
            Some(name) => (Some(a.as_str()), name),
            None => (None, a.clone()),
        };
        Ok(pick(&ctx, node, selector)?.and_then(|s| s.attr(&name)).map(|v| v.to_string()))
    })?;
    obj.set("attr", attr)?;

    let h = host.clone();
    let html = Function::new(ctx.clone(), move || -> Option<String> {
        let docs = h.docs.borrow();
        docs.node(handle).map(|s| s.html().to_string())
    })?;
    obj.set("html", html)?;

    let h = host.clone();
    let all = Function::new(ctx.clone(), move |ctx: Ctx<'js>, selector: String| -> rquickjs::Result<Array<'js>> {
        let handles: Vec<usize> = {
            let mut docs = h.docs.borrow_mut();
            let Some((doc, _)) = docs.nodes.get(handle).copied() else { return Array::new(ctx) };
            let ids: Vec<NodeId> = match docs.node(handle) {
                Some(node) => match node.try_select(&selector) {
                    Some(found) => found.nodes().iter().take(MAX_ALL).map(|n| n.id).collect(),
                    None if dom_query::Matcher::new(&selector).is_err() => {
                        return Err(type_error(&ctx, &format!("invalid selector \"{selector}\"")))
                    }
                    None => Vec::new(),
                },
                None => Vec::new(),
            };
            ids.into_iter().map_while(|id| docs.add(doc, id)).collect()
        };
        let out = Array::new(ctx.clone())?;
        for (i, handle) in handles.into_iter().enumerate() {
            out.set(i, node_object(&ctx, &h, handle)?)?;
        }
        Ok(out)
    })?;
    obj.set("all", all)?;
    Ok(obj.into_value())
}
