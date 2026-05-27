//! Hot-path flat array buffer for latency samples.
//! Rule: NEVER touch HdrHistogram in the hot path. Record cycles here, aggregate after.

pub struct LatencyBuffer {
    samples: Vec<u64>,
    idx: usize,
}

impl LatencyBuffer {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            samples: vec![0u64; n],
            idx: 0,
        }
    }

    /// Hot path: one write, no branch, no allocation.
    #[inline(always)]
    pub fn record(&mut self, cycles: u64) {
        if self.idx < self.samples.len() {
            unsafe {
                *self.samples.get_unchecked_mut(self.idx) = cycles;
            }
        } else {
            self.samples.push(cycles);
        }
        self.idx += 1;
    }

    pub fn finish(&self) -> &[u64] {
        &self.samples[..self.idx]
    }

    pub fn reset(&mut self) {
        self.idx = 0;
    }

    pub fn len(&self) -> usize {
        self.idx
    }

    pub fn is_empty(&self) -> bool {
        self.idx == 0
    }

    pub fn capacity(&self) -> usize {
        self.samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_record_and_finish() {
        let mut buf = LatencyBuffer::with_capacity(4);
        buf.record(10);
        buf.record(20);
        buf.record(30);
        assert_eq!(buf.finish(), &[10, 20, 30]);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn overflow_grows_silently() {
        let mut buf = LatencyBuffer::with_capacity(2);
        buf.record(1);
        buf.record(2);
        buf.record(3);
        buf.record(4);
        assert_eq!(buf.finish(), &[1, 2, 3, 4]);
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn reset_clears() {
        let mut buf = LatencyBuffer::with_capacity(4);
        buf.record(10);
        buf.reset();
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        buf.record(20);
        assert_eq!(buf.finish(), &[20]);
    }

    #[test]
    fn stress_large_volume() {
        let n = 1_000_000;
        let mut buf = LatencyBuffer::with_capacity(n);
        for i in 0u64..n as u64 {
            buf.record(i);
        }
        assert_eq!(buf.len(), n);
        assert_eq!(buf.finish()[0], 0);
        assert_eq!(buf.finish()[n - 1], (n - 1) as u64);
    }
}
