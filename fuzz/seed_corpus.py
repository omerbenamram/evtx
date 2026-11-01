#!/usr/bin/env python3
"""Seeds each fuzz target's corpus, fuzz/corpus/<target>/, from samples/.

- evtx_file: each log, cut to its file header and first two chunks.
- evtx_chunk: the first chunks of each log, those that have the chunk magic.
- wevt_template: the WEVT_TEMPLATE (CRIM) blobs in samples/dlls, which git-lfs keeps.
- mta: the .MTA files.
"""

import hashlib
import pathlib
import struct

FILE_HEADER = 4096
CHUNK = 65536
CHUNKS_PER_LOG = 8

root = pathlib.Path(__file__).resolve().parent.parent
samples = root / "samples"
corpus = root / "fuzz" / "corpus"
seeds = {}


def seed(target, data):
    folder = corpus / target
    folder.mkdir(parents=True, exist_ok=True)
    (folder / hashlib.sha1(data).hexdigest()).write_bytes(data)
    seeds[target] = seeds.get(target, 0) + 1


for log in sorted(samples.rglob("*.evtx")):
    data = log.read_bytes()
    if not data.startswith(b"ElfFile\0"):
        continue
    seed("evtx_file", data[: FILE_HEADER + 2 * CHUNK])
    ends = range(FILE_HEADER + CHUNK, len(data) + 1, CHUNK)
    chunks = [data[end - CHUNK : end] for end in ends]
    chunks = [chunk for chunk in chunks if chunk.startswith(b"ElfChnk\0")]
    for chunk in chunks[:CHUNKS_PER_LOG]:
        seed("evtx_chunk", chunk)

for pe in sorted((samples / "dlls").iterdir()):
    data = pe.read_bytes()
    start = data.find(b"CRIM")
    while start != -1:
        if start + 8 <= len(data):
            (size,) = struct.unpack_from("<I", data, start + 4)
            if 16 <= size <= len(data) - start:
                seed("wevt_template", data[start : start + size])
        start = data.find(b"CRIM", start + 1)

for mta in sorted(samples.rglob("*.MTA")):
    seed("mta", mta.read_bytes())

for target, count in sorted(seeds.items()):
    print(f"{target}: {count} seeds")
