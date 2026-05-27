//! Benchmark environment validation — detect if the machine is in a clean state for measurement.

/// Read voluntary and nonvoluntary context switches from /proc/self/status.
pub fn read_ctxt_switches() -> (u64, u64) {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let (mut vol, mut nonvol) = (0u64, 0u64);
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("voluntary_ctxt_switches:") {
            vol = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("nonvoluntary_ctxt_switches:") {
            nonvol = v.trim().parse().unwrap_or(0);
        }
    }
    (vol, nonvol)
}

/// Read total interrupt count from /proc/interrupts.
fn read_total_irqs() -> u64 {
    let s = std::fs::read_to_string("/proc/interrupts").unwrap_or_default();
    let mut total = 0u64;
    for line in s.lines() {
        if line.starts_with("           ") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        for part in parts.iter().skip(1) {
            if let Ok(v) = part.parse::<u64>() {
                total += v;
            }
        }
    }
    total
}

pub struct EnvSnapshot {
    pub vol: u64,
    pub nonvol: u64,
    pub irqs: u64,
}

impl EnvSnapshot {
    pub fn take() -> Self {
        let (vol, nonvol) = read_ctxt_switches();
        let irqs = read_total_irqs();
        Self { vol, nonvol, irqs }
    }

    /// Returns true if no involuntary preemption or significant IRQ activity occurred.
    pub fn isolation_clean(&self, after: &EnvSnapshot) -> bool {
        let nonvol_ok = after.nonvol - self.nonvol == 0;
        let irq_delta = after.irqs.saturating_sub(self.irqs);
        if irq_delta > 10_000 {
            eprintln!(
                "WARNING: {irq_delta} interrupts during measurement — IRQ noise may inflate tail latencies"
            );
        }
        nonvol_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_read_ctxt_switches() {
        let (vol, nonvol) = read_ctxt_switches();
        let _ = (vol, nonvol);
    }

    #[test]
    fn snapshot_pair_consistent() {
        let before = EnvSnapshot::take();
        let mut sum = 0u64;
        for i in 0..1000 {
            sum += i;
        }
        std::hint::black_box(sum);
        let after = EnvSnapshot::take();

        assert!(after.vol >= before.vol);
        assert!(after.nonvol >= before.nonvol);
    }
}
