//! Which processes listen on which local TCP ports: the table of listening
//! sockets (`/proc/net/tcp` on Linux, `GetExtendedTcpTable` on Windows).
//!
//! Only sockets a connection to `127.0.0.1` can reach count: the ones on the
//! loopback address or on every address. A port on a LAN address alone is
//! not what a local client talks to.

/// Who listens on a local port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortUse {
    /// Nothing listens on it.
    Free,
    /// This process listens on it; `name` is its program (`qbittorrent.exe`),
    /// when it can be read.
    Owned { pid: u32, name: Option<String> },
    /// Something listens on it, but the process is not visible to this user
    /// (another user's, or outside this sandbox).
    Unknown,
}

/// The local ports `pids` listen on, lowest first.
pub fn ports_of(pids: &[u32]) -> Vec<u16> {
    let mut ports = imp::ports_of(pids);
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Who listens on `port`. When several processes do (one on `127.0.0.1`,
/// another on every address), one of `prefer` is named if it is among them.
pub fn port_use(port: u16, prefer: &[u32]) -> PortUse {
    imp::port_use(port, prefer)
}

/// Whether a local client reaches an IPv6 listener on `addr`: `::`, `::1`
/// or an IPv4-mapped loopback address. (A dual-stack `::` socket also takes
/// IPv4 connections, so a program listening "everywhere" often has only
/// this one.)
#[cfg_attr(not(windows), allow(dead_code))]
fn v6_reaches_loopback(addr: [u8; 16]) -> bool {
    let ip = std::net::Ipv6Addr::from(addr);
    ip.is_unspecified() || ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashSet;
    use std::path::Path;

    use super::PortUse;

    /// `(port, socket inode)` of the listening sockets in a `/proc/net/tcp`
    /// or `tcp6` table that local clients reach.
    pub(super) fn parse_table(text: &str) -> Vec<(u16, u64)> {
        // sl  local_address rem_address   st tx_queue:rx_queue tr:tm->when retrnsmt uid timeout inode
        text.lines()
            .skip(1)
            .filter_map(|line| {
                let cols: Vec<&str> = line.split_whitespace().collect();
                let (local, state, inode) = (cols.get(1)?, cols.get(3)?, cols.get(9)?);
                // 0A: TCP_LISTEN.
                if *state != "0A" {
                    return None;
                }
                let (addr, port) = local.split_once(':')?;
                if !reaches_loopback(addr) {
                    return None;
                }
                Some((u16::from_str_radix(port, 16).ok()?, inode.parse().ok()?))
            })
            .collect()
    }

    /// The kernel prints addresses as words in host order: 127.0.0.1 is
    /// `0100007F`, `::1` ends in `01000000`, `::ffff:127.0.0.1` in `0100007F`.
    fn reaches_loopback(addr: &str) -> bool {
        match addr.len() {
            8 => addr == "00000000" || addr.ends_with("7F"),
            32 => {
                let (head, tail) = addr.split_at(24);
                (head.bytes().all(|b| b == b'0') && (tail == "00000000" || tail == "01000000"))
                    || (head == "0000000000000000FFFF0000" && tail.ends_with("7F"))
            }
            _ => false,
        }
    }

    fn listening(proc: &Path) -> Vec<(u16, u64)> {
        ["net/tcp", "net/tcp6"].iter().filter_map(|t| std::fs::read_to_string(proc.join(t)).ok()).flat_map(|t| parse_table(&t)).collect()
    }

    /// The socket inodes `pid` has open (`/proc/<pid>/fd/* -> socket:[n]`).
    fn sockets_of(proc: &Path, pid: u32) -> HashSet<u64> {
        let Ok(fds) = std::fs::read_dir(proc.join(pid.to_string()).join("fd")) else { return HashSet::new() };
        fds.flatten()
            .filter_map(|fd| {
                let link = std::fs::read_link(fd.path()).ok()?;
                link.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
            })
            .collect()
    }

    pub(super) fn ports_of_in(proc: &Path, pids: &[u32]) -> Vec<u16> {
        let owned: HashSet<u64> = pids.iter().flat_map(|pid| sockets_of(proc, *pid)).collect();
        listening(proc).into_iter().filter(|(_, inode)| owned.contains(inode)).map(|(port, _)| port).collect()
    }

    pub(super) fn port_use_in(proc: &Path, port: u16, prefer: &[u32]) -> PortUse {
        let inodes: HashSet<u64> = listening(proc).into_iter().filter(|(p, _)| *p == port).map(|(_, i)| i).collect();
        if inodes.is_empty() {
            return PortUse::Free;
        }
        let owned = |pid: u32| {
            let name = std::fs::read_to_string(proc.join(pid.to_string()).join("comm"))
                .ok()
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty());
            PortUse::Owned { pid, name }
        };
        if let Some(pid) = prefer.iter().find(|pid| !sockets_of(proc, **pid).is_disjoint(&inodes)) {
            return owned(*pid);
        }
        // Every process's sockets: only reached when something does listen.
        let Ok(entries) = std::fs::read_dir(proc) else { return PortUse::Unknown };
        for entry in entries.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if !sockets_of(proc, pid).is_disjoint(&inodes) {
                return owned(pid);
            }
        }
        PortUse::Unknown
    }

    pub(super) fn ports_of(pids: &[u32]) -> Vec<u16> {
        ports_of_in(Path::new("/proc"), pids)
    }

    pub(super) fn port_use(port: u16, prefer: &[u32]) -> PortUse {
        port_use_in(Path::new("/proc"), port, prefer)
    }
}

#[cfg(windows)]
mod imp {
    use std::mem::{offset_of, size_of};

    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };

    use super::PortUse;

    /// WinSock's address families.
    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;
    const NO_ERROR: u32 = 0;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    /// The listening sockets' table of `family`, as Windows wrote it: a u32
    /// count, then the rows. u64s: the buffer only needs their alignment.
    fn table(family: u32) -> Option<Vec<u64>> {
        let mut buf: Vec<u64> = vec![0; 2048];
        loop {
            let mut size = (buf.len() * 8) as u32;
            // SAFETY: `buf` holds `size` bytes; the call writes at most that many.
            let r = unsafe { GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, family, TCP_TABLE_OWNER_PID_LISTENER, 0) };
            match r {
                NO_ERROR => return Some(buf),
                // The table grew between the two calls: ask again with its size.
                ERROR_INSUFFICIENT_BUFFER if (size as usize) > buf.len() * 8 => buf.resize(size as usize / 8 + 1, 0),
                _ => return None,
            }
        }
    }

    /// The rows of `buf` (a table from [`table`] whose rows start at byte
    /// `offset`), as many as its count says and the buffer holds.
    fn rows<T: Copy>(buf: &[u64], offset: usize) -> Vec<T> {
        let bytes = buf.len() * 8;
        let base = buf.as_ptr().cast::<u8>();
        // SAFETY: the buffer starts with the u32 count, and is aligned for it.
        let count = unsafe { base.cast::<u32>().read() } as usize;
        let count = count.min(bytes.saturating_sub(offset) / size_of::<T>());
        // SAFETY: `count` rows of `T` (plain u32s and bytes, aligned at 4 like
        // `offset`) lie inside `buf`, which outlives the copy; built from the
        // buffer's pointer, not from the one-row array the struct declares.
        unsafe { std::slice::from_raw_parts(base.add(offset).cast::<T>(), count).to_vec() }
    }

    /// The port of a row: the low 16 bits, in network order.
    fn port(raw: u32) -> u16 {
        u16::from_be(raw as u16)
    }

    /// `(port, pid)` of the listening sockets local clients reach, IPv4 and IPv6.
    fn listening() -> Vec<(u16, u32)> {
        let mut out = Vec::new();
        if let Some(buf) = table(AF_INET) {
            for r in rows::<MIB_TCPROW_OWNER_PID>(&buf, offset_of!(MIB_TCPTABLE_OWNER_PID, table)) {
                // dwLocalAddr is in network order: its bytes are the address.
                let [a, ..] = r.dwLocalAddr.to_ne_bytes();
                if r.dwLocalAddr == 0 || a == 127 {
                    out.push((port(r.dwLocalPort), r.dwOwningPid));
                }
            }
        }
        if let Some(buf) = table(AF_INET6) {
            for r in rows::<MIB_TCP6ROW_OWNER_PID>(&buf, offset_of!(MIB_TCP6TABLE_OWNER_PID, table)) {
                if super::v6_reaches_loopback(r.ucLocalAddr) {
                    out.push((port(r.dwLocalPort), r.dwOwningPid));
                }
            }
        }
        out
    }

    pub(super) fn ports_of(pids: &[u32]) -> Vec<u16> {
        listening().into_iter().filter(|(_, pid)| pids.contains(pid)).map(|(port, _)| port).collect()
    }

    pub(super) fn port_use(port: u16, prefer: &[u32]) -> PortUse {
        let owners: Vec<u32> = listening().into_iter().filter(|(p, _)| *p == port).map(|(_, pid)| pid).collect();
        let Some(pid) = owners.iter().find(|pid| prefer.contains(pid)).or(owners.first()).copied() else { return PortUse::Free };
        // 0 and 4 are Windows itself (http.sys URL reservations): no program to close.
        let name = (pid > 4).then(|| crate::process::snapshot().into_iter().find(|p| p.pid == pid).map(|p| p.name)).flatten();
        PortUse::Owned { pid, name }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::PortUse;

    pub(super) fn ports_of(_: &[u32]) -> Vec<u16> {
        Vec::new()
    }

    pub(super) fn port_use(_: u16, _: &[u32]) -> PortUse {
        PortUse::Unknown
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1 0000000000000000 100 0 0 10 0
   1: 00000000:9A11 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 222 1 0000000000000000 100 0 0 10 0
   2: 0101A8C0:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 333 1 0000000000000000 100 0 0 10 0
   3: 0100007F:1F90 0100007F:D2F0 01 00000000:00000000 00:00000000 00000000  1000        0 444 1 0000000000000000 100 0 0 10 0
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1388 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 555 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:1389 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 666 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF00000100007F:138A 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 777 1 0000000000000000 100 0 0 10 0
   3: 802A0120000000000000000001000000:138B 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 888 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn only_listening_sockets_local_clients_reach_count() {
        // 192.168.1.1 (LAN only) and an established connection do not.
        assert_eq!(imp::parse_table(TCP), [(8080, 111), (39441, 222)]);
        // ::1, ::, ::ffff:127.0.0.1; not a global IPv6 address.
        assert_eq!(imp::parse_table(TCP6), [(5000, 555), (5001, 666), (5002, 777)]);
    }

    fn fake_proc() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path();
        std::fs::create_dir_all(proc.join("net")).unwrap();
        std::fs::write(proc.join("net/tcp"), TCP).unwrap();
        let process = |pid: &str, comm: &str, sockets: &[u64]| {
            let fd = proc.join(pid).join("fd");
            std::fs::create_dir_all(&fd).unwrap();
            std::fs::write(proc.join(pid).join("comm"), format!("{comm}\n")).unwrap();
            for (i, inode) in sockets.iter().enumerate() {
                std::os::unix::fs::symlink(format!("socket:[{inode}]"), fd.join(i.to_string())).unwrap();
            }
            std::os::unix::fs::symlink("/dev/null", fd.join("99")).unwrap();
        };
        process("10", "qbittorrent", &[111]);
        process("20", "steamwebhelper", &[222, 999]);
        dir
    }

    #[test]
    fn the_ports_of_a_process_come_from_its_sockets() {
        let proc = fake_proc();
        assert_eq!(imp::ports_of_in(proc.path(), &[20]), [39441]);
        assert_eq!(imp::ports_of_in(proc.path(), &[10, 20]), [8080, 39441]);
        assert!(imp::ports_of_in(proc.path(), &[30]).is_empty());
    }

    #[test]
    fn a_port_names_the_process_listening_on_it() {
        let proc = fake_proc();
        assert_eq!(imp::port_use_in(proc.path(), 8080, &[]), PortUse::Owned { pid: 10, name: Some("qbittorrent".into()) });
        assert_eq!(imp::port_use_in(proc.path(), 1234, &[]), PortUse::Free);
        std::fs::remove_dir_all(proc.path().join("10")).unwrap();
        assert_eq!(imp::port_use_in(proc.path(), 8080, &[]), PortUse::Unknown);
    }

    #[test]
    fn a_port_two_processes_listen_on_names_the_preferred_one() {
        let proc = fake_proc();
        // Steam's helper on 127.0.0.1:8080 too (the same port, another socket).
        let tcp = std::fs::read_to_string(proc.path().join("net/tcp")).unwrap()
            + "   4: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 999 1 0000000000000000 100 0 0 10 0\n";
        std::fs::write(proc.path().join("net/tcp"), tcp).unwrap();
        assert_eq!(imp::port_use_in(proc.path(), 8080, &[20]), PortUse::Owned { pid: 20, name: Some("steamwebhelper".into()) });
    }

    #[test]
    fn ipv6_listeners_local_clients_reach() {
        let mut v4_mapped = [0u8; 16];
        v4_mapped[10..].copy_from_slice(&[0xff, 0xff, 127, 0, 0, 1]);
        assert!(v6_reaches_loopback([0; 16]));
        assert!(v6_reaches_loopback(std::net::Ipv6Addr::LOCALHOST.octets()));
        assert!(v6_reaches_loopback(v4_mapped));
        assert!(!v6_reaches_loopback("2a01::1".parse::<std::net::Ipv6Addr>().unwrap().octets()));
    }
}
