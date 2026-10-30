//! 投影编解码：生成记录 <-> JSON BLOB（pbjson 规则，uint64 为规范十进制
//! 字符串）。编码即配额检查点：单事件 <=8192 bytes，超限整体拒绝。

use serde::{Serialize, de::DeserializeOwned};

use crate::model::StoreError;

pub(crate) fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(record)
        .map_err(|error| StoreError::Db(format!("projection encode: {error}")))
}

pub(crate) fn encode_bounded<T: Serialize>(
    record: &T,
    limit: usize,
) -> Result<Vec<u8>, StoreError> {
    let bytes = encode(record)?;
    if bytes.len() > limit {
        return Err(StoreError::PayloadTooLarge { limit });
    }
    Ok(bytes)
}

pub(crate) fn decode<T: DeserializeOwned>(blob: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(blob)
        .map_err(|error| StoreError::Db(format!("projection decode: {error}")))
}

/// SHA-256 摘要（幂等参数规范化摘要、decision 摘要）。
pub(crate) fn digest(parts: &[&[u8]]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hex(&hasher.finalize())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 按 UTF-8 边界拆分为 <=max_bytes 的块（契约 §4/§5：文本按边界拆成
/// <=4096-byte delta；不按字符数计算）。
pub(crate) fn split_utf8(text: &str, max_bytes: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            // 4-byte char cannot exceed a 4096-byte limit; guarded by caller.
            end = (start + max_bytes).min(text.len());
        }
        chunks.push(&text[start..end]);
        start = end;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_respects_utf8_boundaries() {
        // Each 'é' is 2 bytes; a 3-byte chunk boundary must move back.
        let text = "éééé"; // 8 bytes
        let chunks = split_utf8(text, 3);
        let joined: String = chunks.concat();
        assert_eq!(joined, text);
        assert!(chunks.iter().all(|c| c.len() <= 3 || c.len() == 2));
        for chunk in &chunks {
            let start = text.find(chunk).expect("chunk comes from this text");
            assert!(text.is_char_boundary(start + chunk.len()));
        }
    }

    #[test]
    fn digest_is_length_separated() {
        assert_ne!(digest(&[b"ab", b"c"]), digest(&[b"a", b"bc"]));
    }
}
