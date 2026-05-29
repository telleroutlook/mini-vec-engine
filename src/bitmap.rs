/// Fixed-width multi-word bitmap for selection vectors and validity masks.
///
/// Directly inspired by `Bitmap<const W: usize>` from the Golomb-Vanguard OGR engine.
/// In a vectorized database, this serves as the selection vector output from predicate
/// evaluation — the "which rows survive the filter?" structure.

#[derive(Clone, Debug)]
pub struct Bitmap<const W: usize> {
    words: [u64; W],
}

impl<const W: usize> Bitmap<W> {
    pub const BITS: usize = W * 64;

    pub fn zeroed() -> Self {
        Self { words: [0u64; W] }
    }

    pub fn all_ones() -> Self {
        Self {
            words: [u64::MAX; W],
        }
    }

    #[inline]
    pub fn set(&mut self, bit: usize) {
        debug_assert!(bit < Self::BITS);
        self.words[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline]
    pub fn clear(&mut self, bit: usize) {
        debug_assert!(bit < Self::BITS);
        self.words[bit / 64] &= !(1u64 << (bit % 64));
    }

    #[inline]
    pub fn get(&self, bit: usize) -> bool {
        debug_assert!(bit < Self::BITS);
        (self.words[bit / 64] >> (bit % 64)) & 1 == 1
    }

    pub fn popcount(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Iterate over all set bit positions (ascending order).
    pub fn iter_set_bits(&self) -> impl Iterator<Item = usize> + '_ {
        self.words
            .iter()
            .enumerate()
            .filter(|(_, &word)| word != 0)
            .flat_map(|(word_idx, &word)| {
                let base = word_idx * 64;
                std::iter::successors(Some(word), move |&w| {
                    let next = w & (w - 1); // clear lowest set bit
                    if next == 0 {
                        None
                    } else {
                        Some(next)
                    }
                })
                .map(move |w| base + (w.trailing_zeros() as usize))
            })
    }

    /// Bitwise AND — intersection of two selection bitmaps.
    pub fn and(&self, other: &Self) -> Self {
        let mut result = Self::zeroed();
        for i in 0..W {
            result.words[i] = self.words[i] & other.words[i];
        }
        result
    }

    /// Bitwise OR — union of two selection bitmaps.
    pub fn or(&self, other: &Self) -> Self {
        let mut result = Self::zeroed();
        for i in 0..W {
            result.words[i] = self.words[i] | other.words[i];
        }
        result
    }

    /// Bitwise NOT — complement.
    pub fn not(&self) -> Self {
        let mut result = Self::zeroed();
        for i in 0..W {
            result.words[i] = !self.words[i];
        }
        result
    }

    /// Shift all bits left by `n` positions, returning a new bitmap.
    ///
    /// Branchless cross-word shift: decomposes `n` into (word_off, bit_off)
    /// and handles carry between adjacent words without conditional branches
    /// in the hot loop. Edge cases (n=0, n >= W*64, bit_off=0) are handled
    /// with early returns.
    pub fn shl(&self, n: usize) -> Self {
        if n == 0 {
            return self.clone();
        }
        if n >= W * 64 {
            return Self::zeroed();
        }

        let word_off = n / 64;
        let bit_off = n % 64;
        let mut result = Self::zeroed();

        if bit_off == 0 {
            // Word-aligned: pure copy, no carry needed.
            // Avoids the (64-0) shift overflow trap.
            let mut i = W;
            while i > word_off {
                i -= 1;
                result.words[i] = self.words[i - word_off];
            }
        } else {
            // Cross-word shift with carry.
            // bit_off is in 1..63, so inv = 64 - bit_off is in 1..63 — safe for shift.
            let inv = 64 - bit_off;
            let mut i = word_off;
            while i < W {
                let src = i - word_off;
                result.words[i] = self.words[src] << bit_off;
                if src > 0 {
                    result.words[i] |= self.words[src - 1] >> inv;
                }
                i += 1;
            }
        }
        result
    }

    /// Shift all bits right by `n` positions, returning a new bitmap.
    ///
    /// Branchless cross-word shift: mirrors shl, pulling carry from the
    /// next higher word into the current word's high bits.
    pub fn shr(&self, n: usize) -> Self {
        if n == 0 {
            return self.clone();
        }
        if n >= W * 64 {
            return Self::zeroed();
        }

        let word_off = n / 64;
        let bit_off = n % 64;
        let mut result = Self::zeroed();

        if bit_off == 0 {
            let mut i = 0;
            while i + word_off < W {
                result.words[i] = self.words[i + word_off];
                i += 1;
            }
        } else {
            let inv = 64 - bit_off;
            let mut i = 0;
            while i + word_off < W {
                result.words[i] = self.words[i + word_off] >> bit_off;
                if i + word_off + 1 < W {
                    result.words[i] |= self.words[i + word_off + 1] << inv;
                }
                i += 1;
            }
        }
        result
    }

    pub fn as_words(&self) -> &[u64; W] {
        &self.words
    }

    pub fn as_raw_words_mut(&mut self) -> &mut [u64; W] {
        &mut self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_clear() {
        let mut bm: Bitmap<4> = Bitmap::zeroed();
        assert!(!bm.get(0));
        assert!(!bm.get(200));

        bm.set(0);
        bm.set(200);
        assert!(bm.get(0));
        assert!(bm.get(200));
        assert_eq!(bm.popcount(), 2);

        bm.clear(0);
        assert!(!bm.get(0));
        assert_eq!(bm.popcount(), 1);
    }

    #[test]
    fn iter_set_bits() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(0);
        bm.set(63);
        bm.set(64);
        bm.set(127);
        let bits: Vec<_> = bm.iter_set_bits().collect();
        assert_eq!(bits, vec![0, 63, 64, 127]);
    }

    #[test]
    fn and_or_not() {
        let mut a: Bitmap<1> = Bitmap::zeroed();
        let mut b: Bitmap<1> = Bitmap::zeroed();
        a.set(0);
        a.set(1);
        b.set(1);
        b.set(2);

        let anded = a.and(&b);
        assert!(anded.get(1));
        assert!(!anded.get(0));
        assert!(!anded.get(2));

        let ored = a.or(&b);
        assert!(ored.get(0));
        assert!(ored.get(1));
        assert!(ored.get(2));

        let noted = a.not();
        assert!(!noted.get(0));
        assert!(!noted.get(1));
        assert!(noted.get(2));
    }

    // -----------------------------------------------------------------------
    // Shift tests
    // -----------------------------------------------------------------------

    #[test]
    fn shl_identity() {
        let mut bm: Bitmap<4> = Bitmap::zeroed();
        bm.set(5);
        bm.set(100);
        let shifted = bm.shl(0);
        assert!(shifted.get(5));
        assert!(shifted.get(100));
    }

    #[test]
    fn shl_small_within_word() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(0);
        let shifted = bm.shl(3);
        assert!(shifted.get(3));
        assert!(!shifted.get(0));
    }

    #[test]
    fn shl_cross_word() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(62);
        // bit 62 + 3 = bit 65 (word 1)
        let shifted = bm.shl(3);
        assert!(!shifted.get(62));
        assert!(shifted.get(65));
    }

    #[test]
    fn shl_word_aligned() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(0);
        bm.set(5);
        let shifted = bm.shl(64);
        assert!(!shifted.get(0));
        assert!(!shifted.get(5));
        assert!(shifted.get(64));
        assert!(shifted.get(69));
    }

    #[test]
    fn shl_beyond_size() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(0);
        assert!(bm.shl(128).popcount() == 0);
    }

    #[test]
    fn shl_large_gap() {
        // n = 2*64 + 2 = 130, testing word_off > 0 with bit_off > 0
        let mut bm: Bitmap<5> = Bitmap::zeroed();
        bm.set(0);
        bm.set(10);
        bm.set(50);
        let shifted = bm.shl(130);
        assert!(shifted.get(130));
        assert!(shifted.get(140));
        assert!(shifted.get(180));
        assert!(!shifted.get(0));
    }

    #[test]
    fn shr_identity() {
        let mut bm: Bitmap<4> = Bitmap::zeroed();
        bm.set(5);
        bm.set(100);
        let shifted = bm.shr(0);
        assert!(shifted.get(5));
        assert!(shifted.get(100));
    }

    #[test]
    fn shr_basic() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(5);
        bm.set(70);
        let shifted = bm.shr(3);
        assert!(shifted.get(2));
        assert!(shifted.get(67));
        assert!(!shifted.get(5));
    }

    #[test]
    fn shr_cross_word() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(65); // word 1, bit 1
        let shifted = bm.shr(3);
        assert!(shifted.get(62));
    }

    #[test]
    fn shr_word_aligned() {
        let mut bm: Bitmap<3> = Bitmap::zeroed();
        bm.set(64);
        bm.set(130);
        let shifted = bm.shr(64);
        assert!(shifted.get(0));
        assert!(shifted.get(66));
    }

    #[test]
    fn shr_beyond_size() {
        let mut bm: Bitmap<2> = Bitmap::zeroed();
        bm.set(0);
        assert!(bm.shr(128).popcount() == 0);
    }

    #[test]
    fn shl_shr_roundtrip() {
        let mut bm: Bitmap<4> = Bitmap::zeroed();
        bm.set(10);
        bm.set(50);
        bm.set(100);
        bm.set(150);
        for n in [1, 5, 32, 63, 64, 65, 100] {
            let roundtrip = bm.shl(n).shr(n);
            assert_eq!(
                roundtrip.popcount(),
                bm.popcount(),
                "Roundtrip popcount mismatch at n={}",
                n
            );
            // All original bits that don't overflow should survive the roundtrip.
            for &bit in &[10, 50, 100, 150] {
                if bit + n < 256 {
                    assert!(
                        roundtrip.get(bit),
                        "Bit {} lost in roundtrip at n={}",
                        bit,
                        n
                    );
                }
            }
        }
    }
}
