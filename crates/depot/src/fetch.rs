//! Fetching chunks ahead of the writer, within a memory budget.
//!
//! The writer goes through the chunk parts of the unfinished files in order
//! (the "stream"). The fetcher keeps the workers busy with the chunks the
//! stream needs next, as long as the decoded chunks held and on their way fit
//! in the budget. A chunk stays until its last use in the stream. When the
//! budget is full, the chunk needed furthest ahead is dropped for one needed
//! sooner, and fetched again when its turn comes (as Belady's cache would).
//! The writer can always fetch the one chunk it waits for, over budget.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use heronite_data::proc::Cancel;

use crate::install::Source;
use crate::plan::{Chunk, Part, Plan};

/// Tries to fetch a chunk a few times, waiting longer each time, unless
/// cancelled.
pub(crate) fn with_retries(source: &dyn Source, chunk: &Chunk, cancel: &Cancel) -> Result<Vec<u8>> {
    const WAITS: [u64; 4] = [1, 2, 4, 8];
    let mut attempt = 0;
    loop {
        match source.fetch(chunk, cancel) {
            Ok(data) => return Ok(data),
            Err(e) if attempt < WAITS.len() && !cancel.is_requested() => {
                log::warn!("chunk {}: {e:#} (trying again)", chunk.id);
                let until = std::time::Instant::now() + Duration::from_secs(WAITS[attempt]);
                while std::time::Instant::now() < until && !cancel.is_requested() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                attempt += 1;
            }
            Err(e) => return Err(e.context(format!("chunk {}", chunk.id))),
        }
    }
}

pub(crate) struct Fetcher<'p> {
    plan: &'p Plan,
    jobs: Sender<usize>,
    results: Receiver<(usize, Result<Vec<u8>>)>,
    /// Chunk index per stream position.
    stream: Vec<usize>,
    /// Stream positions of each chunk's uses, and how many are behind.
    uses: Vec<Vec<usize>>,
    used: Vec<usize>,
    cache: Vec<Option<Vec<u8>>>,
    in_flight: Vec<bool>,
    in_flight_count: usize,
    max_in_flight: usize,
    /// Bytes held and on their way.
    memory: u64,
    max_memory: u64,
    /// The writer's stream position, and how far ahead fetches went.
    pos: usize,
    ahead: usize,
    downloaded: u64,
}

impl<'p> Fetcher<'p> {
    pub fn new(
        plan: &'p Plan,
        files: &[usize],
        jobs: Sender<usize>,
        results: Receiver<(usize, Result<Vec<u8>>)>,
        max_memory: u64,
        max_in_flight: usize,
    ) -> Self {
        let mut stream = Vec::new();
        let mut uses = vec![Vec::new(); plan.chunks.len()];
        for &i in files {
            for part in &plan.files[i].parts {
                if let Part::Chunk { chunk, .. } = part {
                    uses[*chunk].push(stream.len());
                    stream.push(*chunk);
                }
            }
        }
        let n = plan.chunks.len();
        Self {
            plan,
            jobs,
            results,
            stream,
            uses,
            used: vec![0; n],
            cache: vec![None; n],
            in_flight: vec![false; n],
            in_flight_count: 0,
            max_in_flight: max_in_flight.max(1),
            memory: 0,
            max_memory,
            pos: 0,
            ahead: 0,
            downloaded: 0,
        }
    }

    /// Bytes to download for the stream: each chunk once.
    pub fn download_total(&self) -> u64 {
        self.uses.iter().enumerate().filter(|(_, u)| !u.is_empty()).map(|(c, _)| self.plan.chunks[c].download_size).sum()
    }

    pub fn downloaded(&self) -> u64 {
        self.downloaded
    }

    fn next_use(&self, c: usize) -> Option<usize> {
        self.uses[c].get(self.used[c]).copied()
    }

    fn dispatch(&mut self, c: usize) {
        self.in_flight[c] = true;
        self.in_flight_count += 1;
        self.memory += self.plan.chunks[c].size;
        // The workers outlive the fetcher; a closed queue only means the run ends.
        let _ = self.jobs.send(c);
    }

    fn receive(&mut self, (c, got): (usize, Result<Vec<u8>>)) -> Result<()> {
        let chunk = &self.plan.chunks[c];
        self.in_flight[c] = false;
        self.in_flight_count -= 1;
        self.memory -= chunk.size;
        let data = got?;
        if data.len() as u64 != chunk.size {
            bail!("chunk {} has {} bytes instead of {}", chunk.id, data.len(), chunk.size);
        }
        self.downloaded += chunk.download_size;
        if self.next_use(c).is_some() {
            self.memory += data.len() as u64;
            self.cache[c] = Some(data);
        }
        Ok(())
    }

    /// Takes in the chunks that arrived, without waiting.
    pub fn receive_ready(&mut self) -> Result<()> {
        while let Ok(msg) = self.results.try_recv() {
            self.receive(msg)?;
        }
        Ok(())
    }

    /// Sends the workers what the stream needs next, within the budget.
    pub fn prefetch(&mut self) {
        self.ahead = self.ahead.max(self.pos);
        while self.ahead < self.stream.len() && self.in_flight_count < self.max_in_flight {
            let c = self.stream[self.ahead];
            if self.cache[c].is_some() || self.in_flight[c] {
                self.ahead += 1;
                continue;
            }
            let size = self.plan.chunks[c].size;
            while self.memory > 0 && self.memory + size > self.max_memory {
                if !self.drop_furthest(self.ahead) {
                    return;
                }
            }
            self.dispatch(c);
            self.ahead += 1;
        }
    }

    /// Drops the held chunk whose next use is furthest, if that is after
    /// stream position `than`. Gives whether one was dropped.
    fn drop_furthest(&mut self, than: usize) -> bool {
        let furthest = (0..self.cache.len()).filter(|&c| self.cache[c].is_some()).filter_map(|c| Some((self.next_use(c)?, c))).max();
        match furthest {
            Some((at, c)) if at > than => {
                if let Some(data) = self.cache[c].take() {
                    self.memory -= data.len() as u64;
                }
                true
            }
            _ => false,
        }
    }

    /// Waits until chunk `c` is held, fetching it now if nothing else will;
    /// `false` when cancelled. `tick` gets the bytes downloaded while waiting.
    pub fn wait_for(&mut self, c: usize, cancel: &Cancel, tick: &mut dyn FnMut(u64)) -> Result<bool> {
        loop {
            if self.cache[c].is_some() {
                return Ok(true);
            }
            if cancel.is_requested() {
                return Ok(false);
            }
            if !self.in_flight[c] {
                self.dispatch(c);
            }
            self.prefetch();
            match self.results.recv_timeout(Duration::from_millis(100)) {
                Ok(msg) => {
                    if let Err(e) = self.receive(msg) {
                        if cancel.is_requested() {
                            return Ok(false);
                        }
                        return Err(e);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(anyhow!("the download workers stopped")),
            }
            tick(self.downloaded);
        }
    }

    /// The held chunk `c`; [`Self::wait_for`] said it is there.
    pub fn data(&self, c: usize) -> &[u8] {
        self.cache[c].as_deref().expect("a chunk waited for")
    }

    /// The writer used chunk `c` at its stream position.
    pub fn consumed(&mut self, c: usize) {
        self.used[c] += 1;
        self.pos += 1;
        if self.next_use(c).is_none() {
            if let Some(data) = self.cache[c].take() {
                self.memory -= data.len() as u64;
            }
        }
        // Over budget after fetching the writer's own chunk: make room.
        while self.memory > self.max_memory && self.drop_furthest(self.pos) {}
    }
}
