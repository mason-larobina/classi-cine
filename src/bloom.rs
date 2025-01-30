use std::hash::{Hash, Hasher};
use ahash::AHasher;

/// A simple Bloom filter implementation using a 128-bit mask
/// Provides fast approximate set membership testing
#[derive(Debug, Default)]
pub struct Bloom(u128);

/// Trait for types that can be converted into a Bloom filter mask
pub trait IntoMask {
    /// Convert this type into a 128-bit Bloom filter mask
    fn into_mask(&self) -> u128;
}

impl IntoMask for Bloom {
    fn into_mask(&self) -> u128 {
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
    
    pub fn contains<M: IntoMask>(&self, e: &M) -> bool {
        let mask = e.into_mask();
        (self.0 & mask) == mask
    }
}
