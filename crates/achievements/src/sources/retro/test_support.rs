//! A throwaway local server standing in for retroachievements.org.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use heronite_data::paths::DataPaths;
use heronite_metadata::Http;

use crate::accounts::Accounts;
use crate::sources::Ctx;

pub struct Server {
    /// Ends with `/API/`, like the real base.
    pub base: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    /// The head (request line and headers) of every request so far.
    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

/// Answers `(path fragment, status, body)`; anything else is a 404.
pub fn serve(routes: Vec<(&'static str, u16, String)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/API/", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (routes, seen) = (routes.clone(), seen.clone());
            std::thread::spawn(move || answer(stream, &routes, &seen));
        }
    });
    Server { base, log }
}

fn answer(mut stream: std::net::TcpStream, routes: &[(&'static str, u16, String)], seen: &Mutex<Vec<String>>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let (status, body) = routes
        .iter()
        .find(|(fragment, _, _)| head.lines().next().is_some_and(|l| l.contains(fragment)))
        .map_or((404, "{}".to_owned()), |(_, status, body)| (*status, body.clone()));
    seen.lock().unwrap().push(head);
    let reply = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes());
}

/// The data folder and HTTP client a source needs.
pub struct Env {
    paths: DataPaths,
    http: Http,
}

impl Env {
    pub fn new(dir: &Path) -> Self {
        Self { paths: DataPaths::at(dir), http: Http::default() }
    }

    pub fn ctx<'a>(&'a self, accounts: &'a Accounts) -> Ctx<'a> {
        Ctx { paths: &self.paths, http: &self.http, accounts, lang: "en" }
    }
}
