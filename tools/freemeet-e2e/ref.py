#!/usr/bin/env python3
"""Reference implementation of the FreeMeet E2E frame format v2.

Independent of the Rust and JS code on purpose: both of those are tested
against the vectors this script writes (tests/freemeet-e2e-vectors.json).
Spec: docs/FREEMEET-E2E-NONCE.md

  python3 tools/freemeet-e2e/ref.py            # (re)write the vectors
  python3 tools/freemeet-e2e/ref.py --check    # fail if the file is stale
"""
import json
import os
import sys

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

VERSION = 0xE3          # first byte of every v2 frame / per-NAL blob
LEGACY = 0xE2           # v1 marker (nonce-reuse bug), never accepted
KDF_SALT = b"freemeet-e2e-v2"
SPUR = {"ton": 1, "bild": 2}
AUDIO_HEAD = 14         # ver(1) flags(1) salt(8) counter(4)
NAL_HEAD = 15           # ver(1) salt(8) counter(4) nal index(2)
# Video blob byte 0 for slice NAL units (type 1/5) carries the slice's pps_id (r82): a receiving WebRTC stack
# parses first_mb_in_slice / slice_type / pic_parameter_set_id as Exp-Golomb from the first slice bytes and drops
# frames whose pps_id has no PPS. Byte 0 is therefore chosen so that those three fields read 0, 0, pps_id:
# bits '1' '1' ue(pps_id) then padding. pps_id 0 is the old 0xE3. pps_id > 6 does not fit one byte -> 0xE3.
VIDEO_VERSION = {0: 0xE3, 1: 0xD3, 2: 0xDB, 3: 0xC9, 4: 0xCB, 5: 0xCD, 6: 0xCF}
VIDEO_VERSIONS = set(VIDEO_VERSION.values())


def derive_key(root: bytes, spur: str, salt: bytes) -> bytes:
    info = spur.encode() + b"\x00" + salt
    return HKDF(algorithm=hashes.SHA256(), length=32, salt=KDF_SALT, info=info).derive(root)


def make_nonce(spur: str, nal: int, salt: bytes, counter: int) -> bytes:
    n = bytearray(12)
    n[0] = SPUR[spur]
    n[1:3] = nal.to_bytes(2, "big")
    n[3:8] = salt[:5]
    n[8:12] = counter.to_bytes(4, "big")
    return bytes(n)


def seal_audio(root, salt, counter, keyframe, plain, spur="ton"):
    head = bytes([VERSION, 1 if keyframe else 0]) + salt + counter.to_bytes(4, "big")
    key = derive_key(root, spur, salt)
    ct = AESGCM(key).encrypt(make_nonce(spur, 0, salt, counter), plain, head)
    return head + ct


def open_audio(root, frame, spur="ton"):
    if len(frame) <= AUDIO_HEAD + 16 or frame[0] != VERSION:
        return None
    head = frame[:AUDIO_HEAD]
    salt = frame[2:10]
    counter = int.from_bytes(frame[10:14], "big")
    key = derive_key(root, spur, salt)
    try:
        return AESGCM(key).decrypt(make_nonce(spur, 0, salt, counter), frame[AUDIO_HEAD:], head)
    except Exception:
        return None


# ---- H.264 (Annex B) ------------------------------------------------------

def next_start(d: bytes, at: int):
    i = at
    while i + 3 <= len(d):
        if d[i] == 0 and d[i + 1] == 0:
            if d[i + 2] == 1:
                return i, i + 3
            if i + 4 <= len(d) and d[i + 2] == 0 and d[i + 3] == 1:
                return i, i + 4
        i += 1
    return None


def nal_units(d: bytes):
    out, at, open_ = [], 0, None
    while True:
        t = next_start(d, at)
        if t is None:
            break
        if open_:
            out.append((open_[0], open_[1], t[0]))
        open_ = t
        at = t[1]
    if open_:
        out.append((open_[0], open_[1], len(d)))
    return out


def epb_in(d: bytes) -> bytes:
    out, zeros = bytearray(), 0
    for b in d:
        if zeros >= 2 and b <= 3:
            out.append(3)
            zeros = 0
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


def epb_out(d: bytes) -> bytes:
    out, zeros, i = bytearray(), 0, 0
    while i < len(d):
        b = d[i]
        if zeros >= 2 and b == 3 and i + 1 < len(d) and d[i + 1] <= 3:
            zeros = 0
            i += 1
            continue
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
        i += 1
    return bytes(out)


def _read_ue(data: bytes, bit: int):
    """Exp-Golomb ue(v) at bit offset. Returns (value, next bit) or None."""
    zeros = 0
    while True:
        if bit >= len(data) * 8 or zeros > 16:
            return None
        if (data[bit >> 3] >> (7 - (bit & 7))) & 1:
            break
        zeros += 1
        bit += 1
    bit += 1
    value = 1
    for _ in range(zeros):
        if bit >= len(data) * 8:
            return None
        value = (value << 1) | ((data[bit >> 3] >> (7 - (bit & 7))) & 1)
        bit += 1
    return value - 1, bit


def slice_pps_id(body: bytes):
    """pic_parameter_set_id of a slice NAL payload (after the NAL header byte), or None."""
    head = body[:8]
    first = _read_ue(head, 0)
    if first is None:
        return None
    second = _read_ue(head, first[1])
    if second is None:
        return None
    third = _read_ue(head, second[1])
    return None if third is None else third[0]


def video_version(nal_type: int, body: bytes) -> int:
    """Blob byte 0 for one encrypted NAL unit."""
    if nal_type in (1, 5):
        pps = slice_pps_id(body)
        if pps is not None and pps in VIDEO_VERSION:
            return VIDEO_VERSION[pps]
    return VERSION


def seal_h264(root, salt, counter, frame: bytes) -> bytes:
    units = nal_units(frame)
    if not units:
        return frame
    key = derive_key(root, "bild", salt)
    out = bytearray()
    for nr, (sc, db, end) in enumerate(units):
        hdr_byte = frame[db]
        out += frame[sc:db + 1]
        body = frame[db + 1:end]
        if (hdr_byte & 0x1F) in (7, 8, 9) or not body:
            out += body
            continue
        blob_head = bytes([video_version(hdr_byte & 0x1F, body)]) + salt + counter.to_bytes(4, "big") + nr.to_bytes(2, "big")
        aad = bytes([hdr_byte]) + blob_head
        ct = AESGCM(key).encrypt(make_nonce("bild", nr, salt, counter), body, aad)
        out += epb_in(blob_head + ct)
    return bytes(out)


def open_h264(root, frame: bytes):
    units = nal_units(frame)
    if not units:
        return None
    out = bytearray()
    for sc, db, end in units:
        hdr_byte = frame[db]
        out += frame[sc:db + 1]
        body = frame[db + 1:end]
        if (hdr_byte & 0x1F) in (7, 8, 9) or not body:
            out += body
            continue
        raw = epb_out(body)
        if len(raw) < NAL_HEAD + 16 or raw[0] not in VIDEO_VERSIONS:
            return None
        salt = raw[1:9]
        counter = int.from_bytes(raw[9:13], "big")
        nr = int.from_bytes(raw[13:15], "big")
        key = derive_key(root, "bild", salt)
        try:
            out += AESGCM(key).decrypt(make_nonce("bild", nr, salt, counter), raw[NAL_HEAD:],
                                       bytes([hdr_byte]) + raw[:NAL_HEAD])
        except Exception:
            return None
    return bytes(out)


def open_h264_checked(root, frame: bytes):
    """Open a video frame AND enforce the frame-level rules of r60.

    Returns (plain, (salt, counter) | None) or None. Rules on top of the tags:
    every encrypted NAL of one frame carries the SAME salt+counter and the NAL
    indices strictly increase (no splicing, repeating or reordering of NALs).
    A frame without any encrypted NAL has no counter (nothing to guard).
    """
    units = nal_units(frame)
    if not units:
        return None
    head, last_nr = None, -1
    for sc, db, end in units:
        hdr_byte = frame[db]
        body = frame[db + 1:end]
        if (hdr_byte & 0x1F) in (7, 8, 9) or not body:
            continue
        raw = epb_out(body)
        if len(raw) < NAL_HEAD + 16 or raw[0] not in VIDEO_VERSIONS:
            return None
        this = (raw[1:9], int.from_bytes(raw[9:13], "big"))
        nr = int.from_bytes(raw[13:15], "big")
        if head is not None and this != head:
            return None
        head = this
        if nr <= last_nr:
            return None
        last_nr = nr
    plain = open_h264(root, frame)
    if plain is None:
        return None
    return plain, head


class Guard:
    """Model of the replay guard: highest counter + a SET of seen counters.

    Deliberately not a bit field (the implementations use one), so an off-by-one
    in the bit arithmetic shows up as a vector mismatch. Per sender salt; only
    the last 16 salts are remembered (older streams are forgotten).
    """
    SALTS_MAX = 16
    WINDOW = 64

    def __init__(self):
        self.streams = []   # [(salt, highest, seen_set)] oldest first

    def check(self, salt: bytes, counter: int) -> bool:
        for i, (s, hi, seen) in enumerate(self.streams):
            if s != salt:
                continue
            if counter > hi:
                self.streams[i] = (s, counter, seen | {counter})
                return True
            if hi - counter > self.WINDOW or counter in seen:
                return False
            self.streams[i] = (s, hi, seen | {counter})
            return True
        if len(self.streams) >= self.SALTS_MAX:
            self.streams.pop(0)
        self.streams.append((salt, counter, {counter}))
        return True


def guard_cases():
    """(name, [(salt_hex, counter)]) sequences; expectations come from Guard."""
    a, b = "a1a2a3a4a5a6a7a8", "0102030405060708"
    cases = [
        ("in order", [(a, c) for c in (0, 1, 2, 3)]),
        ("duplicate of the highest", [(a, 5), (a, 5)]),
        ("duplicate of an older one", [(a, 5), (a, 6), (a, 7), (a, 6)]),
        ("reordered inside the window", [(a, 10), (a, 12), (a, 11), (a, 11)]),
        ("exactly 64 behind is still inside", [(a, 100), (a, 36), (a, 36)]),
        ("65 behind is too old", [(a, 100), (a, 35)]),
        ("jump of 63", [(a, 0), (a, 63), (a, 0), (a, 62)]),
        ("jump of exactly 64 keeps the old highest", [(a, 0), (a, 64), (a, 0)]),
        ("jump of 65 forgets it", [(a, 0), (a, 65), (a, 0), (a, 1)]),
        ("first frame seen is remembered", [(a, 500), (a, 500), (a, 499)]),
        ("two salts count independently", [(a, 1), (b, 1), (a, 1), (b, 1), (b, 2)]),
        ("counter wrap edge", [(a, 0xFFFFFFFE), (a, 0xFFFFFFFF), (a, 0xFFFFFFFF), (a, 0xFFFFFFFD)]),
    ]
    # 17 streams: the oldest one is evicted (documented limit), so its
    # replay is accepted again.
    salts = [bytes([i] * 8).hex() for i in range(1, 18)]
    cases.append(("17 salts evict the oldest (documented limit)",
                  [(s, 1) for s in salts] + [(salts[0], 1), (salts[16], 1)]))
    out = []
    for name, steps in cases:
        g = Guard()
        out.append({"name": name,
                    "steps": [{"salt": s, "counter": c, "accept": g.check(bytes.fromhex(s), c)}
                              for s, c in steps]})
    return out


def replay_video_cases(root):
    """Frames + expectations for the frame-level video guard (r60)."""
    a = bytes.fromhex("a1a2a3a4a5a6a7a8")
    b = bytes.fromhex("0102030405060708")
    plain = sample_frame()
    fa, fb, fc = (seal_h264(root, a, c, plain) for c in (10, 11, 12))
    fother = seal_h264(root, b, 77, plain)
    units = nal_units(fa)                      # sps, pps, idr (nr 2), non-idr (nr 3)
    cut = lambda f, i: f[nal_units(f)[i][0]:nal_units(f)[i][2]]
    head = fa[:nal_units(fa)[2][0]]            # SPS + PPS in the clear
    spliced = head + cut(fa, 2) + cut(fb, 3)   # NAL 2 of frame 10 + NAL 3 of frame 11
    swapped = head + cut(fa, 3) + cut(fa, 2)   # indices 3, 2: out of order
    repeated = head + cut(fa, 2) + cut(fa, 2)  # index 2 twice
    only_clear = fa[:nal_units(fa)[2][0]]      # SPS + PPS only
    assert len(units) == 4
    cases = [
        ("in order", [fa, fb, fc]),
        ("replay of an old frame", [fa, fb, fa, fb]),
        ("reordered inside the window", [fa, fc, fb, fb]),
        ("spliced from two frames", [fa, spliced]),
        ("spliced frame as first sight", [spliced]),
        ("NAL units swapped", [swapped]),
        ("NAL unit repeated", [repeated]),
        ("second stream counts on its own", [fa, fother, fother, fa]),
        ("frame without encrypted NAL has no counter", [only_clear, only_clear]),
    ]
    frames, out = [], []
    for name, seq in cases:
        g = Guard()
        steps = []
        for fr in seq:
            if fr not in frames:
                frames.append(fr)
            r = open_h264_checked(root, fr)
            ok = r is not None and (r[1] is None or g.check(*r[1]))
            steps.append({"frame": frames.index(fr), "accept": ok})
        out.append({"name": name, "steps": steps})
    return {"frames": [f.hex() for f in frames], "cases": out}


def sample_frame() -> bytes:
    f = bytearray()
    f += bytes([0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1F, 0x96, 0x54])      # SPS
    f += bytes([0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80])                   # PPS
    f += bytes([0, 0, 0, 1, 0x65])                                     # IDR
    f += bytes((i * 13) % 256 for i in range(400))
    f += bytes([0, 0, 1, 0x41])                                        # non-IDR, 3-byte start code
    f += bytes((i * 29) % 256 for i in range(250))
    return bytes(f)


def pps_frame(pps: int) -> bytes:
    """SPS + PPS + IDR + non-IDR whose slice headers name `pps` (first_mb 0, slice_type 7, pps_id)."""
    def header_bits(p, slice_type):
        bits = "1"                                   # first_mb_in_slice = 0
        bits += format(slice_type + 1, "b").zfill(2 * len(format(slice_type + 1, "b")) - 1)
        bits += format(p + 1, "b").zfill(2 * len(format(p + 1, "b")) - 1)
        bits += "1" * ((-len(bits)) % 8)
        return int(bits, 2).to_bytes(len(bits) // 8, "big")
    f = bytearray()
    f += bytes([0, 0, 0, 1, 0x67, 0x42, 0xC0, 0x0C, 0x43, 0x1A])
    f += bytes([0, 0, 0, 1, 0x68, 0x48, 0xE3, 0xC8])
    f += bytes([0, 0, 0, 1, 0x65]) + header_bits(pps, 7) + bytes((i * 7 + 3) % 256 for i in range(120))
    f += bytes([0, 0, 0, 1, 0x41]) + header_bits(pps, 5) + bytes((i * 11 + 5) % 256 for i in range(90))
    return bytes(f)


def build() -> dict:
    root = bytes(range(32))
    salt_a = bytes.fromhex("a1a2a3a4a5a6a7a8")
    salt_b = bytes.fromhex("0102030405060708")
    v = {
        "_doc": "FreeMeet E2E v2 vectors, written by tools/freemeet-e2e/ref.py (independent reference). "
                "Rust (src/meete2e.rs tests) and JS (web-patch/e2e.test.mjs) must reproduce every value.",
        "root_hex": root.hex(),
        "keys": [],
        "nonces": [],
        "audio": [],
        "h264": [],
        "h264_pps": [],
        "guard": [],
        "replay_video": {},
    }
    for spur in ("ton", "bild"):
        for salt in (salt_a, salt_b):
            v["keys"].append({"spur": spur, "salt": salt.hex(),
                              "key": derive_key(root, spur, salt).hex()})
    for spur, nal, salt, counter in [("ton", 0, salt_a, 0), ("bild", 0, salt_a, 0),
                                     ("bild", 3, salt_b, 0x01020304), ("ton", 0, salt_b, 0xFFFFFFFF)]:
        v["nonces"].append({"spur": spur, "nal": nal, "salt": salt.hex(), "counter": counter,
                            "nonce": make_nonce(spur, nal, salt, counter).hex()})
    for salt, counter, key, plain in [
        (salt_a, 0, False, b"opus frame zero"),
        (salt_a, 1, True, b"keyframe flag set"),
        (salt_b, 0x01020304, False, bytes(range(60))),
        (salt_b, 0xFFFFFFFF, False, b"last counter value"),
    ]:
        fr = seal_audio(root, salt, counter, key, plain)
        assert open_audio(root, fr) == plain
        v["audio"].append({"salt": salt.hex(), "counter": counter, "keyframe": key,
                           "plain": plain.hex(), "frame": fr.hex()})
    sample = sample_frame()
    for salt, counter in [(salt_a, 0), (salt_b, 77)]:
        fr = seal_h264(root, salt, counter, sample)
        assert open_h264(root, fr) == sample
        v["h264"].append({"salt": salt.hex(), "counter": counter, "plain": sample.hex(),
                          "frame": fr.hex()})
    # r82: slice NAL blobs name the slice's pps_id in byte 0 (pps 1, 3, 6 and 9 = out of range -> 0xE3)
    v["h264_pps"] = []
    for pps in (0, 1, 3, 6, 9):
        plain = pps_frame(pps)
        fr = seal_h264(root, salt_a, 5 + pps, plain)
        assert open_h264(root, fr) == plain
        v["h264_pps"].append({"pps": pps, "salt": salt_a.hex(), "counter": 5 + pps, "plain": plain.hex(),
                              "frame": fr.hex(), "blob_version": video_version(5, plain[23:])})
    v["guard"] = guard_cases()
    v["replay_video"] = replay_video_cases(root)
    return v


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    out = os.path.join(here, "..", "..", "tests", "freemeet-e2e-vectors.json")
    text = json.dumps(build(), indent=1) + "\n"
    if "--check" in sys.argv:
        with open(out, encoding="utf-8") as fh:
            if fh.read() != text:
                print("vectors are stale: run tools/freemeet-e2e/ref.py")
                sys.exit(1)
        print("vectors up to date")
        return
    with open(out, "w", encoding="utf-8") as fh:
        fh.write(text)
    print("wrote", os.path.normpath(out))


if __name__ == "__main__":
    main()
