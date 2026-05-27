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

    pub fn as_words(&self) -> &[u64; W] {
        &self.words
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
}
