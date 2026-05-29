//! RDTSC-based query profiling infrastructure.
//!
//! Ported from hft-latency-lab's timer module and adapted for per-query
//! stage profiling. Provides `OperationTimer` for inline timing and
//! `QueryProfile` for structured timing breakdowns.
//!
//! Gate behind `#[cfg(feature = "profile")]` so all profiling overhead
//! compiles away in normal builds.

use core::arch::x86_64::{_rdtsc, _mm_lfence, __rdtscp};

/// Raw timestamp counter read — no serialization guarantees.
///
/// Use when you need the fastest possible read and can tolerate
/// out-of-order execution around the measurement point.
#[inline(always)]
pub fn rdtsc() -> u64 {
    unsafe { _rdtsc() }
}

/// Serialized TSC read: lfence + rdtscp + lfence.
///
/// Prevents out-of-order execution from polluting the measurement window.
/// Use for all accurate interval measurements.
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

/// Convert cycles to nanoseconds given CPU frequency in GHz.
#[inline(always)]
pub fn cycles_to_ns(cycles: u64, ghz: f64) -> u64 {
    (cycles as f64 / ghz) as u64
}

/// Inline operation timer — records start TSC on creation, reports elapsed
/// cycles on demand.
///
/// ```
/// use mini_vec_engine::timing::OperationTimer;
/// let timer = OperationTimer::new();
/// // ... do work ...
/// let elapsed_cycles = timer.elapsed();
/// ```
pub struct OperationTimer {
    start: u64,
}

impl OperationTimer {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            start: rdtsc_serialized(),
        }
    }

    #[inline(always)]
    pub fn elapsed(&self) -> u64 {
        rdtsc_serialized() - self.start
    }
}

/// Per-query stage cycle counts for profiling breakdown.
///
/// Tracks time spent in each major query execution stage so you can
/// see where cycles go without pulling out a full profiler.
pub struct QueryProfile {
    pub predicate_cycles: u64,
    pub aggregate_cycles: u64,
    pub partition_cycles: u64,
    pub total_cycles: u64,
    pub operation_name: String,
}

impl QueryProfile {
    pub fn new(operation_name: &str) -> Self {
        Self {
            predicate_cycles: 0,
            aggregate_cycles: 0,
            partition_cycles: 0,
            total_cycles: 0,
            operation_name: operation_name.to_string(),
        }
    }

    /// Human-readable timing breakdown with cycle counts and nanosecond
    /// conversions. `ghz` should come from `bench_infra::timer::calibrate_ghz()`.
    pub fn summary(&self, ghz: f64) -> String {
        let pred_ns = cycles_to_ns(self.predicate_cycles, ghz);
        let agg_ns = cycles_to_ns(self.aggregate_cycles, ghz);
        let part_ns = cycles_to_ns(self.partition_cycles, ghz);
        let total_ns = cycles_to_ns(self.total_cycles, ghz);

        let pct = |stage: u64| -> String {
            if self.total_cycles == 0 {
                return "  0.0%".to_string();
            }
            format!("{:5.1}%", stage as f64 / self.total_cycles as f64 * 100.0)
        };

        format!(
            "[{}] query profile (ghz={:.3}):\n\
             \x20 predicate : {:>12} cycles | {:>10} ns | {}\n\
             \x20 aggregate : {:>12} cycles | {:>10} ns | {}\n\
             \x20 partition : {:>12} cycles | {:>10} ns | {}\n\
             \x20 total     : {:>12} cycles | {:>10} ns |",
            self.operation_name,
            ghz,
            self.predicate_cycles, pred_ns, pct(self.predicate_cycles),
            self.aggregate_cycles, agg_ns, pct(self.aggregate_cycles),
            self.partition_cycles, part_ns, pct(self.partition_cycles),
            self.total_cycles, total_ns,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdtsc_is_monotonic() {
        let a = rdtsc_serialized();
        let b = rdtsc_serialized();
        assert!(b >= a, "TSC must be monotonic: {a} -> {b}");
    }

    #[test]
    fn raw_rdtsc_is_monotonic() {
        let a = rdtsc();
        let b = rdtsc();
        assert!(b >= a, "raw TSC must be monotonic: {a} -> {b}");
    }

    #[test]
    fn operation_timer_measures_positive_elapsed() {
        let timer = OperationTimer::new();
        // Burn a few cycles so we get a nonzero measurement.
        let mut dummy = 0u64;
        for i in 0..100 {
            dummy = dummy.wrapping_add(i);
        }
        std::hint::black_box(dummy);
        let elapsed = timer.elapsed();
        assert!(elapsed > 0, "elapsed should be > 0, got {elapsed}");
    }

    #[test]
    fn cycles_to_ns_roundtrip() {
        let ghz = 3.9;
        let ns = cycles_to_ns(3900, ghz);
        assert_eq!(ns, 1000, "3900 cycles at 3.9 GHz should be 1000 ns, got {ns}");
    }

    #[test]
    fn query_profile_summary_format() {
        let mut p = QueryProfile::new("test_op");
        p.predicate_cycles = 1000;
        p.aggregate_cycles = 2000;
        p.partition_cycles = 500;
        p.total_cycles = 3500;

        let s = p.summary(3.9);
        assert!(s.contains("test_op"), "summary should contain operation name");
        assert!(s.contains("predicate"), "summary should contain predicate");
        assert!(s.contains("aggregate"), "summary should contain aggregate");
        assert!(s.contains("partition"), "summary should contain partition");
    }

    #[test]
    fn query_profile_zero_total_no_panic() {
        let p = QueryProfile::new("empty");
        let s = p.summary(3.9);
        assert!(s.contains("empty"));
    }
}
