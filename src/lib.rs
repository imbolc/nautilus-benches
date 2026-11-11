use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Instant,
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

pub const NAUTILUS_REVISION: &str = "86e640d623747e5f07b7b7df90e4585193341055";

pub struct CountingAllocator;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static REALLOCS: AtomicU64 = AtomicU64::new(0);
static REQUESTED: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

fn added(bytes: u64) {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}

// Counters never allocate and System receives each pointer with its original layout.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(layout.size() as u64, Relaxed);
            added(layout.size() as u64);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(layout.size() as u64, Relaxed);
            added(layout.size() as u64);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as u64, Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            REALLOCS.fetch_add(1, Relaxed);
            REQUESTED.fetch_add(size as u64, Relaxed);
            if size >= layout.size() {
                added((size - layout.size()) as u64);
            } else {
                LIVE.fetch_sub((layout.size() - size) as u64, Relaxed);
            }
        }
        next
    }
}

#[derive(Clone, Copy, Serialize)]
pub struct Heap {
    pub allocation_calls: u64,
    pub reallocation_calls: u64,
    pub requested_bytes: u64,
    pub live_bytes: u64,
    pub peak_live_bytes: u64,
}

pub fn heap() -> Heap {
    Heap {
        allocation_calls: ALLOCS.load(Relaxed),
        reallocation_calls: REALLOCS.load(Relaxed),
        requested_bytes: REQUESTED.load(Relaxed),
        live_bytes: LIVE.load(Relaxed),
        peak_live_bytes: PEAK.load(Relaxed),
    }
}

#[derive(Serialize)]
pub struct Rss {
    pub current_kib: u64,
    pub process_peak_kib: u64,
}

pub fn rss() -> Result<Rss> {
    let status = fs::read_to_string("/proc/self/status")?;
    let value = |name: &str| -> Result<u64> {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|line| line.split_whitespace().next())
            .context("missing process RSS")?
            .parse()
            .map_err(Into::into)
    };
    Ok(Rss {
        current_kib: value("VmRSS:")?,
        process_peak_kib: value("VmHWM:")?,
    })
}

fn cpu() -> Result<f64> {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // clock_gettime initializes the supplied timespec on success.
    ensure!(
        unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, time.as_mut_ptr()) } == 0,
        "process CPU clock: {}",
        std::io::Error::last_os_error()
    );
    let time = unsafe { time.assume_init() };
    Ok(time.tv_sec as f64 + time.tv_nsec as f64 / 1e9)
}

pub struct Measurement {
    start: Heap,
    start_rss: Rss,
    start_cpu: f64,
    start_wall: Instant,
}

#[derive(Serialize)]
pub struct Metrics {
    pub allocation_calls: u64,
    pub reallocation_calls: u64,
    pub requested_bytes: u64,
    pub start_live_bytes: u64,
    pub end_live_bytes: u64,
    pub peak_live_bytes: u64,
    pub start_rss: Rss,
    pub end_rss: Rss,
    pub cpu_seconds: f64,
    pub elapsed_seconds: f64,
}

impl Measurement {
    pub fn begin() -> Result<Self> {
        let start_rss = rss()?;
        let start_cpu = cpu()?;
        PEAK.store(LIVE.load(Relaxed), Relaxed);
        Ok(Self {
            start: heap(),
            start_rss,
            start_cpu,
            start_wall: Instant::now(),
        })
    }

    pub fn finish(self) -> Result<Metrics> {
        let end = heap();
        let elapsed_seconds = self.start_wall.elapsed().as_secs_f64();
        let cpu_seconds = cpu()? - self.start_cpu;
        Ok(Metrics {
            allocation_calls: end.allocation_calls - self.start.allocation_calls,
            reallocation_calls: end.reallocation_calls - self.start.reallocation_calls,
            requested_bytes: end.requested_bytes - self.start.requested_bytes,
            start_live_bytes: self.start.live_bytes,
            end_live_bytes: end.live_bytes,
            peak_live_bytes: end.peak_live_bytes,
            start_rss: self.start_rss,
            end_rss: rss()?,
            cpu_seconds,
            elapsed_seconds,
        })
    }
}

pub fn check_allocator() -> Result<()> {
    use std::alloc::{alloc, alloc_zeroed, dealloc, realloc};
    let small = Layout::from_size_align(8, 8)?;
    let other = Layout::from_size_align(16, 8)?;
    let large = Layout::from_size_align(32, 8)?;
    let start = heap();
    // All pointers are checked before use, and layouts match their final allocations.
    unsafe {
        let first = alloc(small);
        let second = alloc_zeroed(other);
        ensure!(
            !first.is_null() && !second.is_null(),
            "allocator check allocation failed"
        );
        let first = realloc(first, small, large.size());
        ensure!(!first.is_null(), "allocator check reallocation failed");
        std::hint::black_box((first, second));
        dealloc(first, large);
        dealloc(second, other);
    }
    let end = heap();
    ensure!(
        end.allocation_calls - start.allocation_calls == 2,
        "allocation counter"
    );
    ensure!(
        end.reallocation_calls - start.reallocation_calls == 1,
        "reallocation counter"
    );
    ensure!(
        end.requested_bytes - start.requested_bytes == 56,
        "requested-byte counter"
    );
    ensure!(end.live_bytes == start.live_bytes, "live-byte counter");
    Ok(())
}
