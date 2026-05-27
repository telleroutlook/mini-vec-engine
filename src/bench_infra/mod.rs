//! Benchmark infrastructure — copied from hft-latency-lab (frozen).
//!
//! Provides cycle-accurate timing, latency distribution reporting, and environment
//! noise detection. Shared methodology between HFT and DB kernel projects.

pub mod bench_env;
pub mod histogram;
pub mod latency_buf;
pub mod timer;
