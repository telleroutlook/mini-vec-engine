//! TSC-based precision timing for latency measurement.
//! Shared infrastructure: HFT parser latency ↔ DB kernel micro-benchmarks use the same timer.

use core::arch::x86_64::{__rdtscp, _mm_lfence};

/// Read TSC with full serialization (rdtscp is inherently serializing).
/// lfence before/after prevents out-of-order execution from polluting the measurement window.
#[inline(always)]
pub fn rdtsc_serialized() -> u64 {
    unsafe {
        let mut aux = 0u32;
        _mm_lfence();
        let t = __rdtscp(&mut aux);
        _mm_lfence();
        t
    }
}

/// Calibrate TSC frequency: convert cycles to nanoseconds.
/// Runs two 1-second passes and verifies consistency. Invariant TSC CPUs
/// (Zen 3, Haswell+) should agree within 0.5%.
// TODO: programmatic cpuid check for constant_tsc / nonstop_tsc flags.
// Two-pass consistency is a valid indirect proxy, but an explicit flag check
// would let us fail fast on non-invariant-TSC CPUs instead of just warning.
pub fn calibrate_ghz() -> f64 {
    let g1 = calibrate_ghz_pass(std::time::Duration::from_secs(1));
    let g2 = calibrate_ghz_pass(std::time::Duration::from_secs(1));
    let delta = (g1 - g2).abs() / g1;
    if delta > 0.005 {
        eprintln!(
            "WARNING: TSC calibration inconsistent: pass1={g1:.3} pass2={g2:.3} delta={delta:.4}. \
             Invariant TSC expected — check cpuid flags."
        );
    }
    (g1 + g2) / 2.0
}

fn calibrate_ghz_pass(dur: std::time::Duration) -> f64 {
    use std::time::Instant;
    let start_tsc = rdtsc_serialized();
    let start = Instant::now();
    std::thread::sleep(dur);
    let cycles = rdtsc_serialized() - start_tsc;
    let secs = start.elapsed().as_secs_f64();
    (cycles as f64) / secs / 1e9
}

#[inline(always)]
pub fn cycles_to_ns(cycles: u64, ghz: f64) -> f64 {
    cycles as f64 / ghz
}

/// RAII guard that measures the elapsed cycles between construction and drop.
/// Use in hot paths: `let _m = ScopeTimer::new(&ghz, &mut buf);`
#[allow(dead_code)]
pub struct ScopeTimer<'a> {
    start: u64,
    ghz: f64, // kept for future cycle-to-ns conversion in drop
    buf: &'a mut super::latency_buf::LatencyBuffer,
}

impl<'a> ScopeTimer<'a> {
    #[inline(always)]
    pub fn new(ghz: f64, buf: &'a mut super::latency_buf::LatencyBuffer) -> Self {
        Self {
            start: rdtsc_serialized(),
            ghz,
            buf,
        }
    }
}

impl Drop for ScopeTimer<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        let elapsed = rdtsc_serialized() - self.start;
        self.buf.record(elapsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_is_reasonable() {
        let ghz = calibrate_ghz();
        assert!(
            ghz > 1.0 && ghz < 10.0,
            "calibrated ghz = {ghz}, expected ~3.9 (allow wide range)"
        );
    }

    #[test]
    fn rdtsc_is_monotonic() {
        let a = rdtsc_serialized();
        let b = rdtsc_serialized();
        assert!(b >= a, "TSC must be monotonic: {a} -> {b}");
    }
}
