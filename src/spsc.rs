//! Single-Producer Single-Consumer ring buffer (Disruptor-inspired).
//! Ported from hft-latency-lab — zero-allocation, lock-free for the SPSC case.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SpscRing<T, const CAP: usize> {
    buf: UnsafeCell<[MaybeUninit<T>; CAP]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

unsafe impl<T: Send, const CAP: usize> Send for SpscRing<T, CAP> {}
unsafe impl<T: Send, const CAP: usize> Sync for SpscRing<T, CAP> {}

impl<T, const CAP: usize> SpscRing<T, CAP> {
    pub fn new() -> Self {
        assert!(CAP.is_power_of_two(), "CAP must be power of 2");
        Self {
            buf: UnsafeCell::new(unsafe { MaybeUninit::uninit().assume_init() }),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Producer: push an item. Returns false if full.
    #[inline(always)]
    pub fn push(&self, item: T) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail - head >= CAP {
            return false;
        }
        unsafe {
            let buf = &mut *self.buf.get();
            buf[tail & (CAP - 1)] = MaybeUninit::new(item);
        }
        self.tail.store(tail + 1, Ordering::Release);
        true
    }

    /// Consumer: pop an item. Returns None if empty.
    #[inline(always)]
    pub fn pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head >= tail {
            return None;
        }
        let item = unsafe {
            let buf = &*self.buf.get();
            buf[head & (CAP - 1)].assume_init_read()
        };
        self.head.store(head + 1, Ordering::Release);
        Some(item)
    }

    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Relaxed);
        tail - head
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        CAP
    }
}

impl<T, const CAP: usize> Default for SpscRing<T, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn spsc_basic() {
        let ring: SpscRing<u64, 4> = SpscRing::new();
        assert!(ring.push(42));
        assert!(ring.push(43));
        assert_eq!(ring.pop(), Some(42));
        assert_eq!(ring.pop(), Some(43));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn spsc_full() {
        let ring: SpscRing<u64, 2> = SpscRing::new();
        assert!(ring.push(1));
        assert!(ring.push(2));
        assert!(!ring.push(3));
        assert_eq!(ring.pop(), Some(1));
        assert!(ring.push(3));
    }

    #[test]
    fn spsc_cross_thread() {
        let ring = Arc::new(SpscRing::<u64, 4096>::new());
        let n: u64 = 10_000;

        let producer_ring = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            for i in 0..n {
                let mut spins = 0;
                while !producer_ring.push(i) {
                    spins += 1;
                    if spins > 1_000_000 {
                        thread::yield_now();
                        spins = 0;
                    }
                }
            }
        });

        let consumer_ring = Arc::clone(&ring);
        let consumer = thread::spawn(move || {
            let mut received = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let mut spins = 0;
                loop {
                    if let Some(val) = consumer_ring.pop() {
                        received.push(val);
                        break;
                    }
                    spins += 1;
                    if spins > 1_000_000 {
                        thread::yield_now();
                        spins = 0;
                    }
                }
            }
            received
        });

        producer.join().unwrap();
        let received = consumer.join().unwrap();

        assert_eq!(received.len(), n as usize);
        for (i, &val) in received.iter().enumerate() {
            assert_eq!(val, i as u64);
        }
    }
}
