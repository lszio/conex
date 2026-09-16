//! Helper: compute the canonical P1 chunking golden CIDs for the conformance
//! vector file. The script is not part of the runtime crate; it exists so the
//! author of `conformance/vectors/p1/chunking.json` can cross-check expected
//! values against an independent Rust computation.
//!
//! Run with:
//!   cargo run --quiet --bin p1-vectors

use cid::Cid;
use multihash::Multihash;
use sha2::{Digest, Sha256};

const RAW_CODEC: u64 = 0x55;
const SHA2_256: u64 = 0x12;
const CHUNK_SIZE: usize = 262_144; // 256 KiB
const MANIFEST_PREFIX: &[u8] = b"manifest/v1/";

fn cid_for_raw(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hash = Multihash::<64>::wrap(SHA2_256, &digest).expect("sha2-256 fits in 64 bytes");
    Cid::new_v1(RAW_CODEC, hash).to_string()
}

fn content_cid(bytes: &[u8]) -> String {
    if bytes.len() <= CHUNK_SIZE {
        return cid_for_raw(bytes);
    }
    let mut manifest = MANIFEST_PREFIX.to_vec();
    for chunk in bytes.chunks(CHUNK_SIZE) {
        manifest.extend_from_slice(cid_for_raw(chunk).as_bytes());
    }
    cid_for_raw(&manifest)
}

fn main() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("single_byte", b"x".to_vec()),
        ("hello", b"hello".to_vec()),
        ("hello_conex_newline", b"hello conex\n".to_vec()),
        ("exactly_256kib", vec![0xAB; CHUNK_SIZE]),
        ("exactly_256kib_plus_one", {
            let mut v = vec![0xAB; CHUNK_SIZE];
            v.push(0xCD);
            v
        }),
        ("two_chunks_2", vec![0x01; CHUNK_SIZE * 2]),
        ("five_chunks_random", {
            let mut v = Vec::with_capacity(CHUNK_SIZE * 5);
            for i in 0..(CHUNK_SIZE * 5) {
                v.push((i & 0xFF) as u8);
            }
            v
        }),
    ];

    for (name, bytes) in &cases {
        let cid = content_cid(bytes);
        let len = bytes.len();
        let kind = if len <= CHUNK_SIZE { "raw" } else { "manifest" };
        println!("{name}\t{len}\t{kind}\t{cid}");
    }
}