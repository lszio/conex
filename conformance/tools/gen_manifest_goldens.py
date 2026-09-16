#!/usr/bin/env python3
"""Regenerate the P1 canonical chunked-content golden vectors.

The vectors in ``conformance/vectors/p1/chunking.json`` are produced by an
implementation that shares no code with the Rust (`prost`) or TypeScript
(`ts-proto`) sides: manifest bytes come from ``protoc --encode`` (C++ protobuf)
and CIDs from the Python standard library. Both language implementations must
reproduce these bytes and roots exactly.

Usage:
    python3 conformance/tools/gen_manifest_goldens.py            # rewrite the vectors
    python3 conformance/tools/gen_manifest_goldens.py --check    # fail on drift

Requires ``protoc`` (repo toolchain, see scripts/bootstrap-toolchain.sh).
"""

from __future__ import annotations

import base64
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VECTORS = ROOT / "conformance/vectors/p1/chunking.json"
CHUNKING_PROTO = "conex/v1/chunking.proto"
MANIFEST_TYPE = "conex.v1.ChunkManifest"

FORMAT_VERSION = 1
MANIFEST_FANOUT = 1024
DEFAULT_CHUNK_SIZE = 262_144
HEX_BYTE_BUDGET = 4096

# (name, payload spec, chunk size)
CASES = [
    ("empty", {"kind": "literal", "value": ""}, DEFAULT_CHUNK_SIZE),
    ("hello_conex_newline", {"kind": "literal", "value": "hello conex\n"}, DEFAULT_CHUNK_SIZE),
    ("exactly_one_chunk_262kib", {"kind": "fill", "byte": 171, "sizeBytes": 262_144}, DEFAULT_CHUNK_SIZE),
    ("one_byte_over_one_chunk", {"kind": "fill", "byte": 171, "sizeBytes": 262_145}, DEFAULT_CHUNK_SIZE),
    ("two_full_chunks", {"kind": "fill", "byte": 1, "sizeBytes": 524_288}, DEFAULT_CHUNK_SIZE),
    ("three_chunks_short_last", {"kind": "pattern", "sizeBytes": 524_301}, DEFAULT_CHUNK_SIZE),
    ("single_level_full_fanout", {"kind": "pattern", "sizeBytes": 1024 * 16}, 16),
    ("layered_above_fanout", {"kind": "pattern", "sizeBytes": 1025 * 16}, 16),
]


def payload_bytes(spec: dict) -> bytes:
    kind = spec["kind"]
    if kind == "literal":
        return spec["value"].encode("utf-8")
    if kind == "fill":
        return bytes([spec["byte"]]) * spec["sizeBytes"]
    if kind == "pattern":
        return bytes(i % 256 for i in range(spec["sizeBytes"]))
    raise SystemExit(f"unknown payload kind: {kind}")


def cid_for_raw(payload: bytes) -> str:
    digest = hashlib.sha256(payload).digest()
    multihash = b"\x12\x20" + digest
    return "b" + base64.b32encode(b"\x01\x55" + multihash).decode().lower().rstrip("=")


def leaf_length(content_length: int, chunk_size: int, index: int) -> int:
    return min(chunk_size, content_length - index * chunk_size)


def leaf_manifest_text(chunk_size: int, content_length: int, leaves: list[str], base: int) -> str:
    entries = "".join(
        '  leaves { chunk_cid: "%s" chunk_length: "%d" }\n'
        % (cid, leaf_length(content_length, chunk_size, base + offset))
        for offset, cid in enumerate(leaves)
    )
    return (
        f"format_version: {FORMAT_VERSION}\n"
        f"chunk_size: {chunk_size}\n"
        f'content_length: "{content_length}"\n'
        f"root {{\n{entries}}}\n"
    )


def parent_manifest_text(chunk_size: int, content_length: int, children: list[str]) -> str:
    entries = "".join('  child_manifest_cids: "%s"\n' % cid for cid in children)
    return (
        f"format_version: {FORMAT_VERSION}\n"
        f"chunk_size: {chunk_size}\n"
        f'content_length: "{content_length}"\n'
        f"root {{\n{entries}}}\n"
    )


def protoc_encode(text_format: str) -> bytes:
    protoc = shutil.which("protoc")
    if protoc is None:
        raise SystemExit("protoc not found; run scripts/bootstrap-toolchain.sh and source .toolchain/env.sh")
    result = subprocess.run(
        [protoc, f"--encode={MANIFEST_TYPE}", f"-I{ROOT / 'schema'}", CHUNKING_PROTO],
        input=text_format.encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise SystemExit(f"protoc --encode failed: {result.stderr.decode('utf-8')}")
    return result.stdout


def manifest_tree(chunk_size: int, content_length: int, leaf_cids: list[str]) -> dict:
    """Mirror of the canonical tree rule; bytes come from protoc."""
    manifests: list[dict] = []
    level: list[str] = []
    for group_index in range(0, len(leaf_cids), MANIFEST_FANOUT):
        group = leaf_cids[group_index : group_index + MANIFEST_FANOUT]
        data = protoc_encode(leaf_manifest_text(chunk_size, content_length, group, group_index))
        cid = cid_for_raw(data)
        level.append(cid)
        manifests.append({"cid": cid, "bytes": data, "level": "leaf"})
    while len(level) > 1:
        next_level: list[str] = []
        for offset in range(0, len(level), MANIFEST_FANOUT):
            group = level[offset : offset + MANIFEST_FANOUT]
            data = protoc_encode(parent_manifest_text(chunk_size, content_length, group))
            cid = cid_for_raw(data)
            next_level.append(cid)
            manifests.append({"cid": cid, "bytes": data, "level": "parent"})
        level = next_level
    return {"root_cid": level[0], "manifests": manifests}


def build() -> dict:
    cases = []
    for name, spec, chunk_size in CASES:
        payload = payload_bytes(spec)
        total = len(payload)
        leaves = [cid_for_raw(payload[i : i + chunk_size]) for i in range(0, total, chunk_size)]
        case: dict = {
            "name": name,
            "payload": spec,
            "chunkSize": chunk_size,
            "sizeBytes": total,
        }
        if total <= chunk_size:
            case["expectedRootKind"] = "raw"
            case["rootCid"] = cid_for_raw(payload)
        else:
            tree = manifest_tree(chunk_size, total, leaves)
            root = tree["manifests"][-1]
            case["expectedRootKind"] = "manifest"
            case["rootCid"] = tree["root_cid"]
            case["rootManifestBytesLength"] = len(root["bytes"])
            if len(root["bytes"]) <= HEX_BYTE_BUDGET:
                case["rootManifestBytesHex"] = root["bytes"].hex()
            children = [node["cid"] for node in tree["manifests"] if node["level"] == "leaf"]
            case["leafManifestCids"] = children
            if len(leaves) <= 4:
                case["leafCids"] = leaves
        cases.append(case)

    return {
        "$schema": "conex P1 canonical chunked-content addressing (ChunkManifest)",
        "contract": "docs/contracts/p1-chunking.md",
        "structure": "schema/conex/v1/chunking.proto (ChunkManifest)",
        "generator": "conformance/tools/gen_manifest_goldens.py (protoc --encode + python stdlib)",
        "formatVersion": FORMAT_VERSION,
        "manifestFanout": MANIFEST_FANOUT,
        "defaultChunkSize": DEFAULT_CHUNK_SIZE,
        "cases": cases,
        "rejects": [
            {
                "name": "non_raw_cid_rejected",
                "cid": "bafybeigdyrzt5sfp7udm7hu76uh7yganzbm4p7p7z3l3e3q",
                "chunkSize": 16,
                "contentLength": 48,
                "leafCount": 3,
                "reason": "non-raw codec",
            },
            {
                "name": "non_sha256_multihash_rejected",
                "cid": "QmYwAPJzv5CZsnAzt8auVZRnAn7p9HRw1L1TqQ6T6t9K9Q",
                "chunkSize": 16,
                "contentLength": 48,
                "leafCount": 3,
                "reason": "non-sha2-256 multihash",
            },
            {
                "name": "malformed_cid_rejected",
                "cid": "not-a-cid",
                "chunkSize": 16,
                "contentLength": 48,
                "leafCount": 3,
                "reason": "unparsable text form",
            },
            {
                "name": "single_block_content_has_no_manifest",
                "chunkSize": DEFAULT_CHUNK_SIZE,
                "contentLength": DEFAULT_CHUNK_SIZE,
                "leafCount": 1,
                "reason": "content_length <= chunk_size is addressed by the raw CID",
            },
            {
                "name": "wrong_leaf_count_rejected",
                "chunkSize": 16,
                "contentLength": 48,
                "leafCount": 2,
                "reason": "leaf count must be ceil(content_length / chunk_size)",
            },
            {
                "name": "zero_chunk_size_rejected",
                "chunkSize": 0,
                "contentLength": 48,
                "leafCount": 1,
                "reason": "chunk_size must be > 0",
            },
        ],
    }


def main() -> int:
    check = "--check" in sys.argv[1:]
    document = build()
    rendered = json.dumps(document, indent=2, ensure_ascii=False) + "\n"
    if check:
        current = VECTORS.read_text(encoding="utf-8")
        if current != rendered:
            print(f"{VECTORS.relative_to(ROOT)} is out of date; run gen_manifest_goldens.py", file=sys.stderr)
            return 1
        print("manifest goldens: up to date")
        return 0
    VECTORS.write_text(rendered, encoding="utf-8")
    print(f"wrote {VECTORS.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
