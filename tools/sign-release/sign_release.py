#!/usr/bin/env python3
"""Sign / verify FreeViewer update feeds (Ed25519).

The private key never lives in the repo. Default location:
    /root/fvwork/keys/update-ed25519.key   (hex seed, chmod 600)

Signed message (must match src/update.rs `signed_message`):
    "freeviewer-update-v1\\n<platform>\\n<version>\\n<sha256 hex>\\n<size>"
platform = "win" (feed fields version/url/sha256/size/sig) or "mac" (mac_version/mac_url/mac_sha256/mac_size/mac_sig;
the Mac signature binds mac_version, fallback version for legacy feeds).

Usage:
    sign_release.py keygen [--key PATH]            create key (refuses to overwrite)
    sign_release.py pubkey [--key PATH]            print public key (hex, 64 chars)
    sign_release.py sign --feed version.json [--key PATH] [--win FILE] [--mac FILE [--mac-version V]]
                                                   add "sig" / "mac_sig" to the feed (in place)
    sign_release.py sign-fields --platform win --version V --sha256 H --size N [--key PATH]
                                                   print one signature (used by the relay publish step)
    sign_release.py verify --feed version.json --pub HEX
"""
import argparse
import hashlib
import json
import os
import stat
import sys

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from cryptography.hazmat.primitives import serialization

DEFAULT_KEY = "/root/fvwork/keys/update-ed25519.key"
PREFIX = "freeviewer-update-v1"
FIELDS = {
    "win": ("version", "sha256", "size", "sig"),
    "mac": ("mac_version", "mac_sha256", "mac_size", "mac_sig"),
}


def feed_version(feed, plat):
    """Version a platform signature binds. Mac: mac_version (r28), legacy feeds fall back to version
    (same rule as update::feed_version in the client)."""
    if plat == "mac" and feed.get("mac_version"):
        return feed["mac_version"]
    return feed["version"]


def message(platform, version, sha256, size):
    return f"{PREFIX}\n{platform}\n{version}\n{sha256.lower()}\n{int(size)}".encode()


def load_key(path):
    mode = os.stat(path).st_mode
    if mode & (stat.S_IRWXG | stat.S_IRWXO):
        sys.exit(f"refusing key with loose permissions: {path} (chmod 600)")
    seed = bytes.fromhex(open(path).read().strip())
    return Ed25519PrivateKey.from_private_bytes(seed)


def pub_hex(priv):
    raw = priv.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )
    return raw.hex()


def file_info(path):
    data = open(path, "rb").read()
    return hashlib.sha256(data).hexdigest(), len(data)


def cmd_keygen(a):
    if os.path.exists(a.key):
        sys.exit(f"key exists, not overwriting: {a.key}")
    os.makedirs(os.path.dirname(a.key), mode=0o700, exist_ok=True)
    seed = os.urandom(32)
    fd = os.open(a.key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(seed.hex() + "\n")
    print("public key:", pub_hex(Ed25519PrivateKey.from_private_bytes(seed)))


def cmd_pubkey(a):
    print(pub_hex(load_key(a.key)))


def cmd_sign(a):
    priv = load_key(a.key)
    feed = json.load(open(a.feed))
    if getattr(a, "mac_version", None):
        feed["mac_version"] = a.mac_version
    for plat, path in (("win", a.win), ("mac", a.mac)):
        vf, hf, sf, gf = FIELDS[plat]
        if path:
            h, n = file_info(path)
            if feed.get(hf) and feed[hf].lower() != h:
                sys.exit(f"{plat}: sha256 in feed does not match file")
            if feed.get(sf) and int(feed[sf]) != n:
                sys.exit(f"{plat}: size in feed does not match file")
            feed[hf], feed[sf] = h, n
        if not feed.get(hf):
            continue  # platform not published
        size = feed.get(sf)
        if size is None:
            sys.exit(f"{plat}: feed has a hash but no size; pass the file")
        feed[gf] = priv.sign(message(plat, feed_version(feed, plat), feed[hf], size)).hex()
        print(f"signed {plat}: {feed[gf][:16]}...")
    json.dump(feed, open(a.feed, "w"), indent=2)
    open(a.feed, "a").write("\n")


def cmd_sign_fields(a):
    priv = load_key(a.key)
    print(priv.sign(message(a.platform, a.version, a.sha256, a.size)).hex())


def cmd_verify(a):
    pub = Ed25519PublicKey.from_public_bytes(bytes.fromhex(a.pub))
    feed = json.load(open(a.feed))
    ok = True
    for plat, (vf, hf, sf, gf) in FIELDS.items():
        if not feed.get(hf):
            continue
        sig = feed.get(gf)
        if not sig:
            print(f"{plat}: NO SIGNATURE")
            ok = False
            continue
        try:
            pub.verify(bytes.fromhex(sig), message(plat, feed_version(feed, plat), feed[hf], feed.get(sf, 0)))
            print(f"{plat}: ok")
        except (InvalidSignature, ValueError):
            print(f"{plat}: INVALID")
            ok = False
    sys.exit(0 if ok else 1)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("keygen", "pubkey", "sign", "sign-fields"):
        s = sub.add_parser(name)
        s.add_argument("--key", default=DEFAULT_KEY)
        if name == "sign":
            s.add_argument("--feed", required=True)
            s.add_argument("--win")
            s.add_argument("--mac")
            s.add_argument("--mac-version", dest="mac_version", help="set feed mac_version (the version of the Mac package; signed instead of 'version')")
        if name == "sign-fields":
            s.add_argument("--platform", choices=["win", "mac"], required=True)
            s.add_argument("--version", required=True)
            s.add_argument("--sha256", required=True)
            s.add_argument("--size", required=True)
    v = sub.add_parser("verify")
    v.add_argument("--feed", required=True)
    v.add_argument("--pub", required=True)
    a = p.parse_args()
    {
        "keygen": cmd_keygen,
        "pubkey": cmd_pubkey,
        "sign": cmd_sign,
        "sign-fields": cmd_sign_fields,
        "verify": cmd_verify,
    }[a.cmd](a)


if __name__ == "__main__":
    main()
