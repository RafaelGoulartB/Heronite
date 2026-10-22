//! gpu-screen-recorder's `-ipc` socket: one JSON request per line, each
//! answered by a line with the same `id` (`gpu-screen-recorder(1)`, IPC).
//!
//! ```text
//! → {"id":1,"name":"save-replay","data":{"seconds":30}}
//! ← {"id":1,"result":"ok","data":"/home/u/Videos/Replay_2026-08-05_14-04-22.mp4"}
//! ```

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// A save answers once the file is written: a few seconds at most.
const SAVE_TIMEOUT: Duration = Duration::from_secs(30);
/// Right after the start the socket may not exist yet.
const CONNECT_FOR: Duration = Duration::from_secs(5);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Saves the last `seconds` (the whole buffer when longer) and returns the
/// file. Blocking.
pub fn save_replay(socket: &Path, seconds: u32) -> Result<PathBuf, String> {
    let reply = request(socket, "save-replay", json!({ "seconds": seconds.max(1) }), SAVE_TIMEOUT)?;
    reply.as_str().filter(|p| !p.is_empty()).map(PathBuf::from).ok_or_else(|| "the recorder did not say where it saved".to_string())
}

/// Sends one request and waits for its answer; the answer's `data` on success.
fn request(socket: &Path, name: &str, data: Value, timeout: Duration) -> Result<Value, String> {
    let mut stream = connect(socket)?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut body = json!({ "id": id, "name": name });
    if !data.is_null() {
        body["data"] = data;
    }
    stream.write_all(format!("{body}\n").as_bytes()).map_err(|e| format!("sending {name}: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).map_err(|e| format!("waiting for {name}: {e}"))?;
        if n == 0 {
            return Err(format!("the recorder closed the connection during {name}"));
        }
        if let Some(answer) = parse_reply(&line, id) {
            return answer;
        }
    }
}

fn connect(socket: &Path) -> Result<UnixStream, String> {
    let until = Instant::now() + CONNECT_FOR;
    loop {
        match UnixStream::connect(socket) {
            Ok(s) => return Ok(s),
            Err(e)
                if Instant::now() < until && matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) =>
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("the recorder is not answering ({}): {e}", socket.display())),
        }
    }
}

/// The answer to request `id` in `line`; `None` when the line is about
/// another request.
fn parse_reply(line: &str, id: u64) -> Option<Result<Value, String>> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let reply_id = v.get("id").and_then(Value::as_u64).unwrap_or(0);
    // 0: the recorder could not read our id.
    if reply_id != id && reply_id != 0 {
        return None;
    }
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    Some(match v.get("result").and_then(Value::as_str) {
        Some("ok") => Ok(data),
        _ => Err(data.as_str().map(str::to_string).unwrap_or_else(|| line.trim().to_string())),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn replies_are_matched_by_id() {
        assert_eq!(parse_reply(r#"{"id":3,"result":"ok","data":"/v/a.mp4"}"#, 3), Some(Ok(json!("/v/a.mp4"))));
        assert_eq!(parse_reply(r#"{"id":4,"result":"ok"}"#, 3), None);
        assert_eq!(parse_reply(r#"{"id":0,"result":"error","data":"bad request"}"#, 3), Some(Err("bad request".into())));
        assert_eq!(parse_reply("not json", 3), None);
    }

    #[test]
    fn a_save_returns_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("gsr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).unwrap();
            let req: Value = serde_json::from_slice(&buf[..n]).unwrap();
            assert_eq!(req["name"], "save-replay");
            assert_eq!(req["data"]["seconds"], 30);
            let id = req["id"].as_u64().unwrap();
            s.write_all(format!("{{\"id\":{id},\"result\":\"ok\",\"data\":\"/v/Replay.mp4\"}}\n").as_bytes()).unwrap();
        });
        assert_eq!(save_replay(&socket, 30).unwrap(), PathBuf::from("/v/Replay.mp4"));
        server.join().unwrap();
    }

    #[test]
    fn a_failed_save_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("gsr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).unwrap();
            let id = serde_json::from_slice::<Value>(&buf[..n]).unwrap()["id"].as_u64().unwrap();
            s.write_all(format!("{{\"id\":{id},\"result\":\"error\",\"data\":\"Failed to save replay\"}}\n").as_bytes()).unwrap();
        });
        assert_eq!(save_replay(&socket, 30), Err("Failed to save replay".into()));
        server.join().unwrap();
    }
}
