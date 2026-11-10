//! `ctx.http.get` / `ctx.http.post`: a request to a host of the manifest,
//! with the app's user agent, at the manifest's rate, within the call's
//! budgets. Redirects are followed here, each one checked like the first
//! address. Resolves to `{ status, url, headers, text, json() }`.

use std::time::{Duration, Instant};

use heronite_metadata::http::{BodyTooLarge, APP_UA};
use rquickjs::{Coerced, Ctx, Exception, Function, Object, Value};
use url::Url;

use super::host::{type_error, Host};
use super::{CallError, Limit};
use crate::net::{self, Denied};

/// Redirects one request may follow (each counts as a request too).
const MAX_REDIRECTS: u32 = 5;

/// Headers a script may not set: the host sets them or they could change
/// where the request goes.
const FORBIDDEN_HEADERS: [&str; 7] = ["host", "user-agent", "content-length", "connection", "transfer-encoding", "te", "upgrade"];

struct Outgoing {
    headers: Vec<(String, String)>,
    body: Option<String>,
    content_type: Option<&'static str>,
}

fn options<'js>(ctx: &Ctx<'js>, url: &mut Url, opts: Option<&Object<'js>>) -> rquickjs::Result<Outgoing> {
    let mut out = Outgoing { headers: Vec::new(), body: None, content_type: None };
    let Some(opts) = opts else { return Ok(out) };
    if let Some(headers) = opts.get::<_, Option<Object>>("headers")? {
        for key in headers.keys::<String>() {
            let key = key?;
            let lower = key.to_ascii_lowercase();
            if FORBIDDEN_HEADERS.contains(&lower.as_str()) || lower.starts_with("proxy-") {
                return Err(type_error(ctx, &format!("the header {key} is set by the app")));
            }
            let value: Coerced<String> = headers.get(&key)?;
            out.headers.push((key, value.0));
        }
    }
    if let Some(query) = opts.get::<_, Option<Object>>("query")? {
        let mut pairs = url.query_pairs_mut();
        for key in query.keys::<String>() {
            let key = key?;
            let value: Coerced<String> = query.get(&key)?;
            pairs.append_pair(&key, &value.0);
        }
    }
    if url.query() == Some("") {
        url.set_query(None);
    }
    if let Some(json) = opts.get::<_, Option<Value>>("json")?.filter(|v| !v.is_undefined()) {
        out.body = Some(ctx.json_stringify(json)?.map(|s| s.to_string()).transpose()?.unwrap_or_else(|| "null".into()));
        out.content_type = Some("application/json");
    } else if let Some(form) = opts.get::<_, Option<Object>>("form")? {
        let mut enc = url::form_urlencoded::Serializer::new(String::new());
        for key in form.keys::<String>() {
            let key = key?;
            let value: Coerced<String> = form.get(&key)?;
            enc.append_pair(&key, &value.0);
        }
        out.body = Some(enc.finish());
        out.content_type = Some("application/x-www-form-urlencoded");
    } else if let Some(body) = opts.get::<_, Option<Coerced<String>>>("body")? {
        out.body = Some(body.0);
    }
    Ok(out)
}

/// Ends the call with `e` and throws it to the script.
fn abort(ctx: &Ctx<'_>, host: &Host, e: CallError) -> rquickjs::Error {
    let message = e.to_string();
    host.abort(e);
    Exception::throw_message(ctx, &message)
}

fn denied(ctx: &Ctx<'_>, host: &Host, url: &str, why: Denied) -> rquickjs::Error {
    abort(ctx, host, CallError::Denied(format!("{url}: {why}")))
}

pub(super) fn send<'js>(ctx: &Ctx<'js>, host: &Host, post: bool, address: &str, opts: Option<Object<'js>>) -> rquickjs::Result<Value<'js>> {
    let rules = host.env.hosts.clone();
    let mut url = net::check(address, &rules).map_err(|why| denied(ctx, host, address, why))?;
    let outgoing = options(ctx, &mut url, opts.as_ref())?;
    // Options may add a query; the address is still the one checked.
    let mut post = post;
    let mut body = outgoing.body;
    let mut redirects = 0;
    loop {
        let response = once(ctx, host, post, &url, &outgoing.headers, body.as_deref(), outgoing.content_type)?;
        let location = response.headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.clone());
        match location {
            Some(to) if (300..400).contains(&response.status) && response.status != 304 => {
                redirects += 1;
                if redirects > MAX_REDIRECTS {
                    return Err(abort(ctx, host, CallError::Denied(format!("{url}: more than {MAX_REDIRECTS} redirects"))));
                }
                let next = url.join(&to).map_err(|_| denied(ctx, host, &to, Denied::NotAnAddress))?;
                url = net::check(next.as_str(), &rules).map_err(|why| denied(ctx, host, next.as_str(), why))?;
                if !matches!(response.status, 307 | 308) {
                    post = false;
                    body = None;
                }
            }
            _ => return response_object(ctx, &url, response),
        }
    }
}

/// One request, without following a redirect.
fn once(
    ctx: &Ctx<'_>,
    host: &Host,
    post: bool,
    url: &Url,
    headers: &[(String, String)],
    body: Option<&str>,
    content_type: Option<&str>,
) -> rquickjs::Result<heronite_metadata::http::Response> {
    let limits = &host.env.limits;
    let n = host.requests.get() + 1;
    host.requests.set(n);
    if n > limits.requests {
        return Err(abort(ctx, host, CallError::Limit(Limit::Requests, format!("more than {} requests in one call", limits.requests))));
    }
    let http = &host.env.http;
    let (target, routed) = http.route(url.as_str());
    // A test serves the origin itself; elsewhere the name must not lead to
    // this computer or the local network.
    if !routed {
        match net::public(url) {
            Err(why) => return Err(denied(ctx, host, url.as_str(), why)),
            Ok(false) => {
                host.set_unreachable(format!("{}: the name does not resolve", url.host_str().unwrap_or("")));
                return Err(Exception::throw_message(ctx, &format!("could not reach {url}")));
            }
            Ok(true) => {}
        }
    }
    let slot = host.env.rate.slot();
    if slot >= host.deadline() {
        return Err(abort(ctx, host, CallError::Limit(Limit::Time, "the rate leaves no time for this request".into())));
    }
    if let Some(wait) = slot.checked_duration_since(Instant::now()) {
        std::thread::sleep(wait);
    }
    let left = host.deadline().saturating_duration_since(Instant::now());
    if left < Duration::from_millis(50) {
        return Err(abort(ctx, host, CallError::Limit(Limit::Time, format!("ran longer than {:?}", limits.time))));
    }
    let bytes_left = limits.total_bytes.saturating_sub(host.bytes.get());
    let max = limits.response_bytes.min(bytes_left);
    let mut req = if post { http.post(target) } else { http.get(target) };
    req = req.no_redirects().timeout(limits.request_timeout.min(left)).max_bytes(max).header("user-agent", APP_UA);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(ct) = content_type.filter(|_| body.is_some()) {
        req = req.header("content-type", ct);
    }
    if let Some(b) = body {
        req = req.body(b);
    }
    match req.send() {
        Ok(r) => {
            host.bytes.set(host.bytes.get() + r.body.len() as u64);
            Ok(r)
        }
        Err(e) if e.downcast_ref::<BodyTooLarge>().is_some() => {
            let what = if max < limits.response_bytes { "all the answers of the call" } else { "one answer" };
            Err(abort(ctx, host, CallError::Limit(Limit::Bytes, format!("{url}: {what} over the size limit"))))
        }
        Err(e) => {
            let why = format!("{e:#}").replace(target_text(http, url).as_str(), url.as_str());
            host.set_unreachable(why);
            Err(Exception::throw_message(ctx, &format!("could not reach {url}")))
        }
    }
}

/// The address a request really went to (a test's server), to keep it out
/// of messages.
fn target_text(http: &heronite_metadata::Http, url: &Url) -> String {
    http.route(url.as_str()).0
}

fn response_object<'js>(ctx: &Ctx<'js>, url: &Url, r: heronite_metadata::http::Response) -> rquickjs::Result<Value<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("status", r.status as i32)?;
    obj.set("url", url.as_str())?;
    let headers = Object::new(ctx.clone())?;
    for (k, v) in &r.headers {
        headers.set(k.as_str(), v.as_str())?;
    }
    obj.set("headers", headers)?;
    let text = String::from_utf8_lossy(&r.body).into_owned();
    obj.set("text", text.as_str())?;
    obj.set("json", Function::new(ctx.clone(), move |ctx: Ctx<'js>| ctx.json_parse(text.clone()))?)?;
    let (promise, resolve, _) = ctx.promise()?;
    resolve.call::<_, ()>((obj,))?;
    Ok(promise.into_value())
}
