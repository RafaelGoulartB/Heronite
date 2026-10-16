//! Pauses the download queue when the connection drops and resumes it when it
//! comes back.
//!
//! Two signals: the kernel's routing table says whether any network is up (a
//! file read, no traffic), and the probes of
//! [`Net::is_online`](heronite_launch::net::Net::is_online) whether the
//! internet answers. A download that still moves is never paused, so a
//! network that blocks the probes but downloads fine is left alone.

use std::time::{Duration, Instant};

/// How often the queue is looked at while it runs or waits for the network.
pub const TICK: Duration = Duration::from_secs(5);
/// How long the download must stand still before the connection is checked.
const STALL: Duration = Duration::from_secs(20);
/// After a pause, the queue resumes once a network has been up this long even
/// when the probes keep failing (they may be blocked). A download that stalls
/// again is paused again.
const RETRY: Duration = Duration::from_secs(120);

/// What to do on this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Nothing,
    Pause,
    /// Probe the internet; pause when it does not answer.
    PauseIfOffline,
    Resume,
    /// Probe the internet; resume when it answers.
    ResumeIfOnline,
}

#[derive(Debug, Default)]
pub struct OfflineWatch {
    /// The queue was paused here (not by the user).
    paused: bool,
    /// Last time the running download moved (or was first seen).
    moved_at: Option<Instant>,
    /// While paused: a network has been up since.
    up_since: Option<Instant>,
    /// A probe runs in the background.
    probing: bool,
}

impl OfflineWatch {
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// The running download reported progress that changed.
    pub fn moved(&mut self, now: Instant) {
        self.moved_at = Some(now);
    }

    /// Nothing runs: the stall clock starts over with the next download.
    pub fn idle(&mut self) {
        self.moved_at = None;
    }

    /// The user paused, resumed or emptied the queue: it is theirs again.
    pub fn release(&mut self) {
        *self = Self { probing: self.probing, ..Self::default() };
    }

    pub fn did_pause(&mut self) {
        *self = Self { paused: true, probing: self.probing, ..Self::default() };
    }

    pub fn probe_started(&mut self) {
        self.probing = true;
    }

    pub fn probe_ended(&mut self) {
        self.probing = false;
    }

    /// Whether the running download has stood still long enough to suspect
    /// the connection.
    pub fn stalled(&self, now: Instant) -> bool {
        self.moved_at.is_some_and(|at| now.duration_since(at) >= STALL)
    }

    /// `running`: the queue is downloading. `network`: a default route exists.
    pub fn check(&mut self, running: bool, network: bool, now: Instant) -> Check {
        if self.paused {
            if !network {
                self.up_since = None;
                return Check::Nothing;
            }
            let up = *self.up_since.get_or_insert(now);
            if now.duration_since(up) >= RETRY {
                return Check::Resume;
            }
            return if self.probing { Check::Nothing } else { Check::ResumeIfOnline };
        }
        if !running {
            self.idle();
            return Check::Nothing;
        }
        self.moved_at.get_or_insert(now);
        if !self.stalled(now) {
            return Check::Nothing;
        }
        match (network, self.probing) {
            (false, _) => Check::Pause,
            (true, false) => Check::PauseIfOffline,
            (true, true) => Check::Nothing,
        }
    }
}

/// Whether any network is up: a default route through an interface that is
/// up, IPv4 or IPv6. A system without these files counts as online (nothing
/// is paused on a guess).
pub fn has_network() -> bool {
    let v4 = std::fs::read_to_string("/proc/net/route");
    let v6 = std::fs::read_to_string("/proc/net/ipv6_route");
    if v4.is_err() && v6.is_err() {
        return true;
    }
    v4.is_ok_and(|t| ipv4_default(&t)) || v6.is_ok_and(|t| ipv6_default(&t))
}

const RTF_UP: u32 = 0x1;
const RTF_REJECT: u32 = 0x200;

fn usable(iface: &str, flags: &str) -> bool {
    let flags = u32::from_str_radix(flags, 16).unwrap_or(0);
    iface != "lo" && flags & RTF_UP != 0 && flags & RTF_REJECT == 0
}

/// `/proc/net/route`: `Iface Destination Gateway Flags .. Mask ..` (hex).
fn ipv4_default(table: &str) -> bool {
    table.lines().skip(1).any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() >= 8 && f[1] == "00000000" && f[7] == "00000000" && usable(f[0], f[3])
    })
}

/// `/proc/net/ipv6_route`: `dest dest_len src src_len next_hop metric refcnt
/// use flags iface` (hex, no header).
fn ipv6_default(table: &str) -> bool {
    table.lines().any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        f.len() >= 10 && f[0].bytes().all(|b| b == b'0') && f[1] == "00" && usable(f[9], f[8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const V4_UP: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0
wlan0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0
";
    const V4_LAN_ONLY: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";
    const V6_UP: &str = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wlan0
fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 wlan0
";
    // What a machine without IPv6 connectivity has: the unreachable default on lo.
    const V6_NONE: &str = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo
00000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000002 00000000 80200001 lo
";

    #[test]
    fn default_routes() {
        assert!(ipv4_default(V4_UP));
        assert!(!ipv4_default(V4_LAN_ONLY));
        assert!(!ipv4_default(""));
        assert!(ipv6_default(V6_UP));
        assert!(!ipv6_default(V6_NONE));
        // a default route that is not up does not count
        assert!(!ipv4_default(&V4_UP.replace("\t0003\t", "\t0002\t")));
    }

    fn at(base: Instant, s: u64) -> Instant {
        base + Duration::from_secs(s)
    }

    #[test]
    fn a_moving_download_is_left_alone() {
        let t = Instant::now();
        let mut w = OfflineWatch::default();
        assert_eq!(w.check(true, false, t), Check::Nothing, "the stall clock starts now");
        w.moved(at(t, 15));
        assert_eq!(w.check(true, false, at(t, 30)), Check::Nothing);
        assert_eq!(w.check(true, true, at(t, 34)), Check::Nothing);
        assert_eq!(w.check(true, true, at(t, 35)), Check::PauseIfOffline);
    }

    #[test]
    fn a_stalled_download_pauses() {
        let t = Instant::now();
        let mut w = OfflineWatch::default();
        w.check(true, true, t);
        assert_eq!(w.check(true, false, at(t, 20)), Check::Pause, "no network at all");
        assert_eq!(w.check(true, true, at(t, 20)), Check::PauseIfOffline, "network up: ask the internet");
        w.probe_started();
        assert_eq!(w.check(true, true, at(t, 25)), Check::Nothing, "one probe at a time");
        w.probe_ended();
        // nothing runs: the clock starts over with the next download
        assert_eq!(w.check(false, false, at(t, 30)), Check::Nothing);
        assert_eq!(w.check(true, false, at(t, 31)), Check::Nothing);
    }

    #[test]
    fn resumes_when_the_network_is_back() {
        let t = Instant::now();
        let mut w = OfflineWatch::default();
        w.did_pause();
        assert!(w.is_paused());
        assert_eq!(w.check(false, false, at(t, 5)), Check::Nothing);
        assert_eq!(w.check(false, true, at(t, 10)), Check::ResumeIfOnline);
        // the probes keep failing but the network stays up: try anyway
        assert_eq!(w.check(false, true, at(t, 129)), Check::ResumeIfOnline);
        assert_eq!(w.check(false, true, at(t, 130)), Check::Resume);
        // a network that went away restarts that wait
        assert_eq!(w.check(false, false, at(t, 131)), Check::Nothing);
        assert_eq!(w.check(false, true, at(t, 132)), Check::ResumeIfOnline);
        w.release();
        assert!(!w.is_paused());
    }
}
