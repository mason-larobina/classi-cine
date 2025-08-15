use ahash::AHasher;
use std::hash::{Hash, Hasher};

/// A simple Bloom filter implementation using a 128-bit mask
/// Provides fast approximate set membership testing
#[derive(Debug, Default, Clone)]
pub struct Bloom(u128);

/// Trait for types that can be converted into a Bloom filter mask
pub trait ToMask {
    /// Convert this type into a 128-bit Bloom filter mask
    fn to_mask(&self) -> u128;
}

impl ToMask for Bloom {
    fn to_mask(&self) -> u128 {
        self.0
    }
}

impl Bloom {
    fn hash<T: Hash + Copy>(e: T) -> u64 {
        // Twice the speed of DefaultHasher.
        let mut hasher = AHasher::default();
        e.hash(&mut hasher);
        hasher.finish()
    }

    pub fn mask<T: Hash + Copy>(e: T) -> u128 {
        1u128 << (Self::hash(e) % 128)
    }

    pub fn set<T: Hash + Copy>(&mut self, e: T) {
        self.0 |= Self::mask(e);
    }

    pub fn contains<M: ToMask>(&self, e: &M) -> bool {
        let mask = e.to_mask();
        (self.0 & mask) == mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash() {
        let hash1 = Bloom::hash(42);
        let hash2 = Bloom::hash(42);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_mask() {
        let mask = Bloom::mask(42);
        assert!(mask.is_power_of_two());
    }

    #[test]
    fn test_set() {
        let mut bloom = Bloom::default();
        bloom.set(42);
        assert_ne!(bloom.0, 0);
    }

    #[test]
    fn test_contains() {
        let mut bloom1 = Bloom::default();
        let mut bloom2 = Bloom::default();
        bloom1.set(42);
        bloom2.set(42);
        assert!(bloom1.contains(&bloom2));
    }

    #[test]
    fn test_to_mask() {
        let mut bloom = Bloom::default();
        bloom.set(42);
        assert_eq!(bloom.to_mask(), bloom.0);
    }

    #[test]
    fn test_contains_subset() {
        let mut bloom1 = Bloom::default();
        let mut bloom2 = Bloom::default();

        bloom1.set(1);
        bloom1.set(2);
        bloom1.set(3);

        bloom2.set(1);
        bloom2.set(2);

        assert!(bloom1.contains(&bloom2));
        assert!(!bloom2.contains(&bloom1));
    }
}
