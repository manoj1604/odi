//! An ODI file shown as a read-only folder: a read fetches the chunks it
//! needs, checked as they arrive (Reader::read_chunk), so a program starts
//! without the rest of the file (SPEC §7 step 3). Its writable folders (§5.8)
//! are links to folders under scratch. Linux (FUSE) for now: elsewhere odi run
//! gets the whole file first.
//!
//! Reads that must fetch run on a pool of threads, so one slow fetch doesn't
//! hold up the others; opening a file fetches all its chunks at once (one
//! batch, with a Prefetch). With `record`, the order chunks are first read in
//! is kept: odi profile turns it into a start list.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Chunks in the order they were first read: (entry index, chunk index).
#[derive(Default)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Reads {
    seen: std::collections::HashSet<(usize, usize)>,
    order: Vec<(usize, usize)>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Reads {
    pub fn note(&mut self, entry: usize, chunk: usize) {
        if self.seen.insert((entry, chunk)) {
            self.order.push((entry, chunk));
        }
    }
}

/// A mounted file; dropping it unmounts it.
pub struct Mounted {
    pub dir: PathBuf,
    /// Whether dir was made for it (and goes when it's unmounted).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    own: bool,
    reads: Option<Arc<Mutex<Reads>>>,
    #[cfg(target_os = "linux")]
    session: Option<fuser::BackgroundSession>,
}

impl Mounted {
    /// What was read, in order, as (path, chunk index), when recording.
    pub fn reads(&self, rd: &odi_format::Reader) -> Vec<(String, usize)> {
        let Some(r) = &self.reads else { return Vec::new() };
        let r = r.lock().expect("lock");
        r.order.iter().map(|&(e, c)| (rd.entries[e].path.clone(), c)).collect()
    }
}

#[cfg(not(target_os = "linux"))]
pub fn mount_lazy(
    _rd: Arc<odi_format::Reader>,
    _scratch: &std::path::Path,
    _prefetch: Option<Arc<dyn crate::hub::Prefetch>>,
    _record: bool,
    _warm: Vec<(usize, usize)>,
) -> anyhow::Result<Mounted> {
    anyhow::bail!("reading on demand is on Linux for now")
}

/// As mount_lazy, at dir (an empty folder, kept when unmounted): for `cd`
/// into a file (odi open).
#[cfg(target_os = "linux")]
pub use linux::{mount_at, mount_lazy};

#[cfg(target_os = "linux")]
impl Drop for Mounted {
    fn drop(&mut self) {
        drop(self.session.take());
        if !self.own {
            return;
        }
        for _ in 0..20 {
            if std::fs::remove_dir(&self.dir).is_ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        for prog in ["fusermount3", "fusermount"] {
            if std::process::Command::new(prog).arg("-uz").arg(&self.dir).status().is_ok_and(|s| s.success()) {
                break;
            }
        }
        let _ = std::fs::remove_dir(&self.dir);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::{BTreeMap, HashMap};
    use std::ffi::OsStr;
    use std::path::Path;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, UNIX_EPOCH};

    use anyhow::{bail, Result};
    use fuser::consts::FOPEN_KEEP_CACHE;
    use fuser::{
        FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
        Request,
    };
    use odi_format::{Kind, Reader};

    use super::{Mounted, Reads};
    use crate::hub::Prefetch;

    const TTL: Duration = Duration::from_secs(3600);
    const CHUNK_CACHE_BYTES: usize = 256 << 20;
    /// Reads fetching at once.
    const WORKERS: usize = 16;

    enum What {
        Dir(BTreeMap<String, u64>),
        File(usize),
        Link(Vec<u8>),
    }

    struct Node {
        parent: u64,
        mode: u32,
        what: What,
    }

    /// Recent chunks, uncompressed: a read is usually a piece of a chunk, and
    /// the next read wants the next piece.
    #[derive(Default)]
    struct Chunks {
        map: HashMap<String, Arc<Vec<u8>>>,
        bytes: usize,
    }

    type Cache = Arc<Mutex<Chunks>>;

    fn chunk(rd: &Reader, cache: &Cache, e: usize, c: usize) -> odi_format::Result<Arc<Vec<u8>>> {
        let ch = &rd.entries[e].chunks[c];
        if let Some(b) = cache.lock().expect("lock").map.get(&ch.hash) {
            return Ok(b.clone());
        }
        let b = Arc::new(rd.read_chunk(ch)?);
        let mut g = cache.lock().expect("lock");
        if g.bytes + b.len() > CHUNK_CACHE_BYTES {
            *g = Chunks::default();
        }
        g.bytes += b.len();
        g.map.insert(ch.hash.clone(), b.clone());
        Ok(b)
    }

    /// The chunks of entry e that [off, off + size) touches, and where each starts.
    fn touched(rd: &Reader, e: usize, off: u64, size: u64) -> Vec<(usize, u64)> {
        let mut out = Vec::new();
        let mut at = 0u64;
        for (c, ch) in rd.entries[e].chunks.iter().enumerate() {
            if at >= off + size {
                break;
            }
            if off < at + ch.size {
                out.push((c, at));
            }
            at += ch.size;
        }
        out
    }

    fn read_range(rd: &Reader, cache: &Cache, e: usize, off: u64, size: u64) -> odi_format::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        for (c, at) in touched(rd, e, off, size) {
            let b = chunk(rd, cache, e, c)?;
            let start = (off + out.len() as u64 - at) as usize;
            let n = (b.len() - start).min(size as usize - out.len());
            out.extend_from_slice(&b[start..start + n]);
        }
        Ok(out)
    }

    type Job = Box<dyn FnOnce() + Send>;

    /// A few threads taking jobs from one queue.
    struct Pool(mpsc::Sender<Job>);

    impl Pool {
        fn new(n: usize) -> Pool {
            let (tx, rx) = mpsc::channel::<Job>();
            let rx = Arc::new(Mutex::new(rx));
            for _ in 0..n {
                let rx = rx.clone();
                std::thread::spawn(move || loop {
                    let job = rx.lock().expect("lock").recv();
                    match job {
                        Ok(job) => job(),
                        Err(_) => return,
                    }
                });
            }
            Pool(tx)
        }

        fn run(&self, job: impl FnOnce() + Send + 'static) {
            let _ = self.0.send(Box::new(job));
        }
    }

    struct OdiFs {
        rd: Arc<Reader>,
        nodes: Vec<Node>, // (inode n is nodes[n - 1]; the root is 1)
        cache: Cache,
        pool: Pool,
        prefetch: Option<Arc<dyn Prefetch>>,
        reads: Option<Arc<Mutex<Reads>>>,
    }

    impl OdiFs {
        fn node(&self, ino: u64) -> Option<&Node> {
            self.nodes.get((ino as usize).checked_sub(1)?)
        }

        fn attr(&self, ino: u64) -> Option<FileAttr> {
            let n = self.node(ino)?;
            let (kind, size) = match &n.what {
                What::Dir(_) => (FileType::Directory, 0),
                What::File(i) => (FileType::RegularFile, self.rd.entries[*i].size),
                What::Link(t) => (FileType::Symlink, t.len() as u64),
            };
            Some(FileAttr {
                ino,
                size,
                blocks: size.div_ceil(512),
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
                crtime: UNIX_EPOCH,
                kind,
                perm: n.mode as u16,
                nlink: if kind == FileType::Directory { 2 } else { 1 },
                // SAFETY: getuid and getgid can't fail and touch no memory.
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0,
                blksize: 4096,
                flags: 0,
            })
        }

        fn cached(&self, e: usize, c: usize) -> bool {
            self.cache.lock().expect("lock").map.contains_key(&self.rd.entries[e].chunks[c].hash)
        }
    }

    /// Builds the tree: every entry, with writable folders (and what's
    /// under them in the file) replaced by links to scratch folders.
    fn tree(rd: &Reader, scratch: &Path) -> Vec<Node> {
        let mut nodes = vec![Node { parent: 1, mode: 0o755, what: What::Dir(BTreeMap::new()) }];
        let mut dirs: HashMap<String, u64> = HashMap::from([(String::new(), 1)]);
        fn add(nodes: &mut Vec<Node>, parent: u64, name: &str, node: Node) -> u64 {
            nodes.push(node);
            let ino = nodes.len() as u64;
            if let What::Dir(children) = &mut nodes[parent as usize - 1].what {
                children.insert(name.to_string(), ino);
            }
            ino
        }
        fn dir_of(nodes: &mut Vec<Node>, dirs: &mut HashMap<String, u64>, p: &str) -> u64 {
            if let Some(&ino) = dirs.get(p) {
                return ino;
            }
            let (up, name) = p.rsplit_once('/').unwrap_or(("", p));
            let parent = dir_of(nodes, dirs, up);
            let ino = add(nodes, parent, name, Node { parent, mode: 0o755, what: What::Dir(BTreeMap::new()) });
            dirs.insert(p.to_string(), ino);
            ino
        }
        let split = |p: &str| -> (String, String) {
            let (up, name) = p.rsplit_once('/').unwrap_or(("", p));
            (up.to_string(), name.to_string())
        };
        let writable = &rd.manifest.writable;
        let in_writable = |p: &str| writable.iter().any(|w| p == w || p.starts_with(&format!("{w}/")));
        for (i, w) in writable.iter().enumerate() {
            let dir = scratch.join(i.to_string());
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("odi: {w}: {e}");
                continue;
            }
            let (up, name) = split(w);
            let parent = dir_of(&mut nodes, &mut dirs, &up);
            let target = dir.as_os_str().as_encoded_bytes().to_vec();
            add(&mut nodes, parent, &name, Node { parent, mode: 0o777, what: What::Link(target) });
        }
        for (i, e) in rd.entries.iter().enumerate() {
            if in_writable(&e.path) {
                continue;
            }
            let (up, name) = split(&e.path);
            let parent = dir_of(&mut nodes, &mut dirs, &up);
            match e.kind {
                Kind::Dir => {
                    if let Some(&ino) = dirs.get(&e.path) {
                        nodes[ino as usize - 1].mode = e.mode;
                    } else {
                        let ino = add(
                            &mut nodes,
                            parent,
                            &name,
                            Node { parent, mode: e.mode, what: What::Dir(BTreeMap::new()) },
                        );
                        dirs.insert(e.path.clone(), ino);
                    }
                }
                Kind::Symlink => {
                    add(
                        &mut nodes,
                        parent,
                        &name,
                        Node { parent, mode: 0o777, what: What::Link(e.target.as_bytes().to_vec()) },
                    );
                }
                Kind::File => {
                    add(&mut nodes, parent, &name, Node { parent, mode: e.mode, what: What::File(i) });
                }
            }
        }
        nodes
    }

    impl Filesystem for OdiFs {
        fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let (is_dir, found) = match (self.node(parent), name.to_str()) {
                (Some(Node { what: What::Dir(c), .. }), n) => (true, n.and_then(|n| c.get(n).copied())),
                _ => (false, None),
            };
            match found.and_then(|ino| self.attr(ino)) {
                Some(a) => reply.entry(&TTL, &a, 0),
                // (Nothing by that name, and there never will be: the kernel may
                // remember that, as an entry with inode 0. A program looking for
                // its modules asks for thousands of names that aren't there.)
                None if is_dir => {
                    let mut none = self.attr(1).expect("the root");
                    none.ino = 0;
                    reply.entry(&TTL, &none, 0)
                }
                None => reply.error(libc::ENOENT),
            }
        }

        fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
            match self.attr(ino) {
                Some(a) => reply.attr(&TTL, &a),
                None => reply.error(libc::ENOENT),
            }
        }

        fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
            match self.node(ino) {
                Some(Node { what: What::Link(t), .. }) => reply.data(t),
                _ => reply.error(libc::EINVAL),
            }
        }

        fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
            if flags & (libc::O_WRONLY | libc::O_RDWR) != 0 {
                return reply.error(libc::EROFS);
            }
            let Some(Node { what: What::File(e), .. }) = self.node(ino) else { return reply.error(libc::EISDIR) };
            let e = *e;
            // (What isn't here yet comes in one batch, while the program
            // gets on with its first read.)
            if let Some(pf) = &self.prefetch {
                let chunks = &self.rd.entries[e].chunks;
                if (0..chunks.len()).any(|c| !self.cached(e, c)) {
                    let ranges: Vec<(u64, u64)> = chunks.iter().map(|c| (c.offset, c.length)).collect();
                    let pf = pf.clone();
                    self.pool.run(move || pf.prefetch(&ranges));
                }
            }
            reply.opened(0, FOPEN_KEEP_CACHE);
        }

        fn read(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            offset: i64,
            size: u32,
            _flags: i32,
            _lock: Option<u64>,
            reply: ReplyData,
        ) {
            let Some(Node { what: What::File(e), .. }) = self.node(ino) else { return reply.error(libc::EISDIR) };
            let (e, off, size) = (*e, offset.max(0) as u64, size as u64);
            let touched = touched(&self.rd, e, off, size);
            if let Some(r) = &self.reads {
                let mut r = r.lock().expect("lock");
                for &(c, _) in &touched {
                    r.note(e, c);
                }
            }
            let (rd, cache) = (self.rd.clone(), self.cache.clone());
            let answer = move || match read_range(&rd, &cache, e, off, size) {
                Ok(b) => reply.data(&b),
                Err(err) => {
                    eprintln!("odi: {}: {err}", rd.entries[e].path);
                    reply.error(libc::EIO)
                }
            };
            // (What's here is answered at once; what must be fetched waits
            // on a worker, not on this loop.)
            if touched.iter().all(|&(c, _)| self.cached(e, c)) {
                answer();
            } else {
                self.pool.run(answer);
            }
        }

        fn readdir(&mut self, _req: &Request<'_>, ino: u64, _fh: u64, offset: i64, mut reply: ReplyDirectory) {
            let Some(Node { what: What::Dir(children), parent, .. }) = self.node(ino) else {
                return reply.error(libc::ENOTDIR);
            };
            let mut all: Vec<(u64, FileType, &str)> =
                vec![(ino, FileType::Directory, "."), (*parent, FileType::Directory, "..")];
            for (name, &child) in children {
                let kind = match self.node(child).map(|n| &n.what) {
                    Some(What::Dir(_)) => FileType::Directory,
                    Some(What::Link(_)) => FileType::Symlink,
                    _ => FileType::RegularFile,
                };
                all.push((child, kind, name));
            }
            for (n, (child, kind, name)) in all.into_iter().enumerate().skip(offset.max(0) as usize) {
                if reply.add(child, n as i64 + 1, kind, name) {
                    break;
                }
            }
            reply.ok();
        }
    }

    pub fn mount_lazy(
        rd: Arc<Reader>,
        scratch: &Path,
        prefetch: Option<Arc<dyn Prefetch>>,
        record: bool,
        warm: Vec<(usize, usize)>,
    ) -> Result<Mounted> {
        mount(rd, scratch, prefetch, record, warm, None)
    }

    pub fn mount_at(
        rd: Arc<Reader>,
        scratch: &Path,
        prefetch: Option<Arc<dyn Prefetch>>,
        dir: &Path,
    ) -> Result<Mounted> {
        mount(rd, scratch, prefetch, false, Vec::new(), Some(dir))
    }

    fn mount(
        rd: Arc<Reader>,
        scratch: &Path,
        prefetch: Option<Arc<dyn Prefetch>>,
        record: bool,
        warm: Vec<(usize, usize)>,
        at: Option<&Path>,
    ) -> Result<Mounted> {
        if !Path::new("/dev/fuse").exists() {
            bail!("no /dev/fuse here");
        }
        let on_path = |p: &str| {
            std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(p).is_file()))
        };
        if !on_path("fusermount3") && !on_path("fusermount") {
            bail!("no fusermount here (the fuse3 package)");
        }
        let (dir, own) = match at {
            Some(d) => (d.to_path_buf(), false),
            None => (tempfile::Builder::new().prefix("odi-mount-").tempdir()?.keep(), true),
        };
        let reads = record.then(|| Arc::new(Mutex::new(Reads::default())));
        let (cache, pool) = (Cache::default(), Pool::new(WORKERS));
        // (Chunks the program will read first, (entry, chunk), decompressed
        // ahead, so its reads are answered at once.)
        for (e, c) in warm.into_iter().filter(|&(e, c)| rd.entries.get(e).is_some_and(|x| c < x.chunks.len())) {
            let (rd, cache) = (rd.clone(), cache.clone());
            pool.run(move || {
                let _ = chunk(&rd, &cache, e, c);
            });
        }
        let fs = OdiFs { nodes: tree(&rd, scratch), rd, cache, pool, prefetch, reads: reads.clone() };
        let opts = [MountOption::RO, MountOption::FSName("odi".into()), MountOption::Subtype("odi".into())];
        match fuser::spawn_mount2(fs, &dir, &opts) {
            Ok(s) => Ok(Mounted { dir, own, reads, session: Some(s) }),
            Err(e) => {
                if own {
                    let _ = std::fs::remove_dir(&dir);
                }
                Err(e.into())
            }
        }
    }
}
