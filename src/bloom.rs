use std::collections::{HashSet, HashMap};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

#[derive(Debug, Default)]
pub struct Bloom(u128);

pub trait IntoMask {
    fn into_mask(&self) -> u128;
}

impl IntoMask for Bloom {
    fn into_mask(&self) -> u128 {
        self.0
    }
}

impl Bloom {
    fn hash<T: Hash + Copy>(e: T) -> u64 {
        let mut hasher = DefaultHasher::new();
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