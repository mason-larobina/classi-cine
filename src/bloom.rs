use ahash::AHasher;
use std::hash::{Hash, Hasher};

/// Number of bits in the filter.
const BITS: u64 = 512;

/// A precomputed single-bit Bloom filter mask: which word and which bit in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomMask {
    word: u8,
    bit: u64,
}

/// A simple Bloom filter implementation using a 512-bit mask (single hash)
/// Provides fast approximate set membership testing
#[derive(Debug, Default, Clone)]
pub struct Bloom([u64; 8]);

impl Bloom {
    fn hash<T: Hash + Copy>(e: T) -> u64 {
        // Twice the speed of DefaultHasher.
        let mut hasher = AHasher::default();
        e.hash(&mut hasher);
        hasher.finish()
    }

    pub fn mask<T: Hash + Copy>(e: T) -> BloomMask {
        let i = (Self::hash(e) % BITS) as usize;
        BloomMask {
            word: (i / 64) as u8,
            bit: 1u64 << (i % 64),
        }
    }

    pub fn set<T: Hash + Copy>(&mut self, e: T) {
        self.set_mask(Self::mask(e));
    }

    pub fn set_mask(&mut self, m: BloomMask) {
        self.0[m.word as usize] |= m.bit;
    }

    pub fn contains_mask(&self, m: BloomMask) -> bool {
        self.0[m.word as usize] & m.bit != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash() {
        assert_eq!(Bloom::hash(42), Bloom::hash(42));
    }

    #[test]
    fn test_mask_range() {
        for e in 0..1000u32 {
            let m = Bloom::mask(e);
            assert!(m.word < 8);
            assert!(m.bit.is_power_of_two());
            assert_eq!(m, Bloom::mask(e));
        }
    }

    #[test]
    fn test_set_and_contains() {
        let mut bloom = Bloom::default();
        bloom.set(42);
        assert!(bloom.contains_mask(Bloom::mask(42)));
        assert!(bloom.0.iter().any(|w| *w != 0));
    }

    #[test]
    fn test_unset_not_contained() {
        let mut bloom = Bloom::default();
        bloom.set(1);
        // Find an element whose mask differs from that of 1.
        let other = (2..1000u32)
            .find(|e| Bloom::mask(*e) != Bloom::mask(1))
            .unwrap();
        assert!(!bloom.contains_mask(Bloom::mask(other)));
        assert!(!Bloom::default().contains_mask(Bloom::mask(1)));
    }

    #[test]
    fn test_set_mask_matches_set() {
        let mut a = Bloom::default();
        let mut b = Bloom::default();
        a.set(7);
        b.set_mask(Bloom::mask(7));
        assert_eq!(a.0, b.0);
    }
}
