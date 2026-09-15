//! CIDv1 content addressing (design S5.3). P0 fixes raw + SHA-256.
use cid::Cid;
use multihash::Multihash;
use sha2::{Digest, Sha256};

use crate::wire::ProtocolError;

/// multicodec: raw
pub const RAW_CODEC: u64 = 0x55;
/// multihash: sha2-256
pub const SHA2_256: u64 = 0x12;

/// CIDv1 / raw / SHA-256, lowercase base32 text.
pub fn cid_for_raw(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hash = Multihash::<64>::wrap(SHA2_256, &digest).expect("sha2-256 digest fits in 64 bytes");
    Cid::new_v1(RAW_CODEC, hash).to_string()
}

/// Parse and normalize a CID text form. Rejects unsupported codec/multihash and
/// trailing bytes; accepted text bases are normalized to binary for comparison.
pub fn parse_cid(text: &str) -> Result<Cid, ProtocolError> {
    let cid = Cid::try_from(text)
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid CID: {e}")))?;
    if cid.codec() != RAW_CODEC {
        return Err(ProtocolError::bad_request(
            None,
            format!("unsupported content codec {}", cid.codec()),
        ));
    }
    if cid.hash().code() != SHA2_256 {
        return Err(ProtocolError::bad_request(
            None,
            format!("unsupported multihash code {}", cid.hash().code()),
        ));
    }
    if cid.hash().size() != 32 {
        return Err(ProtocolError::bad_request(
            None,
            format!(
                "sha2-256 digest must be 32 bytes, got {}",
                cid.hash().size()
            ),
        ));
    }
    Ok(cid)
}
