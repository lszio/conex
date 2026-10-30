//! Crockford base32 ULID minting（契约 §2：ID 为仓库惯例 ULID，节点分配）。
//! 进程内单调计数器保证同毫秒唯一；随机位来自进程随机哈希种子。
//! 这不是全局唯一性强保证，P2 节点身份约束了单写者。

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Default)]
pub(crate) struct IdGen {
    counter: AtomicU64,
    seed: u64,
}

impl IdGen {
    pub(crate) fn new() -> Self {
        // RandomState 每进程随机；以此为伪随机种子足够避免跨重启可预测。
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        Self {
            counter: AtomicU64::new(0),
            seed: hasher.finish(),
        }
    }

    pub(crate) fn mint(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(self.seed ^ n.rotate_left(17));
        let mut random = hasher.finish() as u128;
        // 80-bit randomness: fold another round for the upper bits.
        let mut hasher2 = RandomState::new().build_hasher();
        hasher2.write_u64(self.seed.wrapping_mul(0x9E3779B97F4A7C15) ^ n);
        random |= (hasher2.finish() as u128) << 64;
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() & ((1u128 << 48) - 1))
            .unwrap_or(0);
        encode((ms << 80) | (random & ((1u128 << 80) - 1)))
    }
}

/// 26-char Crockford ULID; the first char encodes 3 bits so it is <= '7'.
pub(crate) fn encode(value: u128) -> String {
    let mut out = [b'0'; 26];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = 125 - 5 * i;
        let index = ((value >> shift) & 0x1f) as usize;
        *slot = ALPHABET[index];
    }
    String::from_utf8(out.to_vec()).expect("crockford alphabet is ASCII")
}
