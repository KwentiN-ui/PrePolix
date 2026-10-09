//! A hash map for integer keys.

use std::collections::HashMap;

/// Hash map with a fast multiplicative hash for the integer keys of large meshes; the
/// standard hasher is built to resist collision attacks and is several times slower here.
pub(crate) type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;

#[derive(Default)]
pub(crate) struct FastHasher(u64);

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for &chunk in chunks {
            self.write_u64(u64::from_le_bytes(chunk));
        }
        for &b in rest {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}
