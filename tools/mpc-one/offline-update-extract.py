#!/usr/bin/env python3
"""OFFLINE, workstation only: unpack the partition payloads of an Akai update image.

It reads both container formats offline-update-image.py knows: FDT/FIT
images (each node that carries inline "data" or FIT external data) and
"AZ01" images (each PART record). For each payload it checks the image's
own hashes over the stored bytes, decompresses the payload as the image
says and writes it to a new folder under ~/mpfree-work. A hash that doesn't
match the stored bytes is checked again over the uncompressed data, since
a format may hash either. A payload whose hash matches neither is not
written. Signatures are reported but not verified: the signing key isn't
ours.

It only reads the image, never contacts the MPC, and never writes anywhere
but a new output folder. The output is Akai's proprietary software: keep
it private and commit only findings.

Usage: python3 tools/mpc-one/offline-update-extract.py IMAGE [--node PATH ...]
Output folder: $MPFREE_WORK_DIR (default ~/mpfree-work)/akai-unpacked-<UTC>-<random>/
"""
import argparse
import bz2
import hashlib
import importlib.util
import lzma
import os
import shutil
import subprocess
import sys
import tempfile
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
CHUNK = 1 << 20
OUT_LIMIT = 16 << 20


def load_lister():
    spec = importlib.util.spec_from_file_location("offline_update_image",
                                                  os.path.join(HERE, "offline-update-image.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


IMG = load_lister()


class ExtractError(Exception):
    pass


def region_reader(fh, offset, length):
    """Yield the bytes of a file region in chunks."""
    fh.seek(offset)
    left = length
    while left:
        block = fh.read(min(CHUNK, left))
        if not block:
            raise ExtractError(f"file ends inside a payload at offset {offset + length - left}")
        left -= len(block)
        yield block


class Payload:
    """One payload to extract, from either container format."""

    def __init__(self, path, offset, length, where, compression, hashes, notes):
        self.path, self.offset, self.length, self.where = path, offset, length, where
        self.compression = compression  # None: guess from the payload's first bytes
        self.hashes = hashes  # [(label, algo, expected bytes)]
        self.notes = notes


SUPPORTED = ("crc32", "sha1", "sha256", "sha384", "sha512", "md5")


def digests_of(blocks, algos):
    state = {a: (0 if a == "crc32" else hashlib.new(a)) for a in set(algos)}
    for block in blocks:
        for algo in state:
            if algo == "crc32":
                state[algo] = zlib.crc32(block, state[algo])
            else:
                state[algo].update(block)
    return {a: (v.to_bytes(4, "big") if a == "crc32" else v.digest()) for a, v in state.items()}


def file_blocks(path):
    with open(path, "rb") as fh:
        yield from iter(lambda: fh.read(CHUNK), b"")


def fdt_payloads(fdt):
    nodes = [n for n in payload_nodes(fdt.root) if fdt.payload(n)]
    found = []
    for node in nodes:
        offset, length, where = fdt.payload(node)
        hashes, notes = [], []
        for child in node.children:
            if child.name.startswith("hash"):
                algo_prop, value_prop = child.prop("algo"), child.prop("value")
                if algo_prop and value_prop:
                    algo = fdt.read(*algo_prop).rstrip(b"\x00").decode("ascii", "replace")
                    hashes.append((child.name, algo, fdt.read(*value_prop)))
            elif child.name.startswith("signature"):
                notes.append(f"  {child.name}: present, not verified (Akai's key)")
        prop = node.prop("compression")
        compression = fdt.read(*prop).rstrip(b"\x00").decode("ascii", "replace") if prop else None
        found.append(Payload(node.path, offset, length, where, compression, hashes, notes))
    return found


def az01_payloads(az):
    return [Payload(p.path, p.offset, p.length, "AZ01 PART record", p.compression or None,
                    [(f"hash-{i + 1}", algo, value) for i, (algo, value) in enumerate(p.hashes)], [])
            for p in az.parts]


def check_stored(fh, payload):
    """Check hashes over the stored bytes. Return (lines, pending, any_checked)."""
    lines, pending = [], []
    known = [(label, algo, value) for label, algo, value in payload.hashes if algo in SUPPORTED]
    for label, algo, _ in payload.hashes:
        if algo not in SUPPORTED:
            lines.append(f"  {label}: algorithm {algo} not supported, not checked")
    if known:
        got = digests_of(region_reader(fh, payload.offset, payload.length), [a for _, a, _ in known])
        for label, algo, expected in known:
            if got[algo] == expected:
                lines.append(f"  {label}: {algo} matches (image says {expected.hex()}, data gives {got[algo].hex()})")
            else:
                lines.append(f"  {label}: {algo} differs over the stored bytes "
                             f"({got[algo].hex()}); checking the uncompressed data")
                pending.append((label, algo, expected, got[algo]))
    lines.extend(payload.notes)
    if not known:
        lines.append("  no hash in the image for this payload")
    return lines, pending


def check_output(path, pending):
    """Check the hashes that didn't match the stored bytes against the written file."""
    got = digests_of(file_blocks(path), [algo for _, algo, _, _ in pending])
    lines, ok = [], True
    for label, algo, expected, stored in pending:
        if got[algo] == expected:
            lines.append(f"  {label}: {algo} matches the uncompressed data ({expected.hex()})")
        else:
            ok = False
            lines.append(f"  {label}: {algo} MISMATCH (image says {expected.hex()}, stored bytes give "
                         f"{stored.hex()}, uncompressed data gives {got[algo].hex()})")
    return lines, ok


def compression_of(fh, payload):
    if payload.compression:
        return payload.compression
    fh.seek(payload.offset)
    head = fh.read(min(payload.length, 16))
    return {"gzip": "gzip", "xz": "xz", "zstd": "zstd", "bzip2": "bzip2",
            "lz4 frame": "lz4"}.get(IMG.guess(head), "none")


def decompress_to(fh, offset, length, method, dst):
    chunks = region_reader(fh, offset, length)
    if method == "none":
        for block in chunks:
            dst.write(block)
        return
    if method == "gzip":
        # Output is drained in bounded pieces: a filesystem image is mostly zeros,
        # so one compressed chunk can expand to hundreds of megabytes.
        decomp = zlib.decompressobj(16 + zlib.MAX_WBITS)
        for block in chunks:
            dst.write(decomp.decompress(block, OUT_LIMIT))
            while decomp.unconsumed_tail:
                dst.write(decomp.decompress(decomp.unconsumed_tail, OUT_LIMIT))
        dst.write(decomp.flush())
        if not decomp.eof:
            raise ExtractError("gzip stream is truncated")
        if decomp.unused_data:
            raise ExtractError("unexpected data after the gzip stream")
        return
    if method in ("lzma", "xz", "bzip2"):
        decomp = bz2.BZ2Decompressor() if method == "bzip2" else lzma.LZMADecompressor()
        for block in chunks:
            if decomp.eof:
                raise ExtractError(f"unexpected data after the {method} stream")
            dst.write(decomp.decompress(block, max_length=OUT_LIMIT))
            while not decomp.eof and not decomp.needs_input:
                dst.write(decomp.decompress(b"", max_length=OUT_LIMIT))
        if not decomp.eof:
            raise ExtractError(f"{method} stream is truncated")
        if decomp.unused_data:
            raise ExtractError(f"unexpected data after the {method} stream")
        return
    if method in ("zstd", "lz4"):
        tool = shutil.which(method)
        if not tool:
            raise ExtractError(f"payload is {method}-compressed; install the '{method}' command-line tool")
        # The payload goes in on stdin and out on stdout; the tool is given no file names.
        proc = subprocess.Popen([tool, "-d", "-c", "-q"], stdin=subprocess.PIPE, stdout=dst,
                                stderr=subprocess.PIPE)
        try:
            for block in chunks:
                proc.stdin.write(block)
        finally:
            proc.stdin.close()
        err = proc.stderr.read().decode("utf-8", "replace").strip()
        if proc.wait() != 0:
            raise ExtractError(f"{method} failed: {err}")
        return
    raise ExtractError(f"unknown compression '{method}'")


def output_root():
    root = os.path.realpath(os.path.expanduser(os.environ.get("MPFREE_WORK_DIR", "~/mpfree-work")))
    if root == REPO_ROOT or root.startswith(REPO_ROOT + os.sep):
        probe = os.path.join(root, "probe")
        ignored = subprocess.run(["git", "-C", REPO_ROOT, "check-ignore", "-q", "--", probe],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
        if not ignored:
            raise ExtractError(f"MPFREE_WORK_DIR is inside the git clone and not git-ignored: {root}")
    return root


def payload_nodes(node):
    for child in node.children:
        yield child
        yield from payload_nodes(child)


def payloads_in(fh, size):
    fh.seek(0)
    if fh.read(4) == IMG.AZ01_MAGIC:
        az = IMG.Az01(fh, size)
        found = az01_payloads(az)
        if az.unknown or az.trailer:
            pos = (az.unknown or az.trailer)[0]
            print(f"note: {size - pos} bytes at offset {pos} are not a PART record "
                  f"(offline-update-image.py shows them)")
        return found
    return fdt_payloads(IMG.Fdt(fh, size))


def extract(image, wanted):
    out_root = output_root()
    size = os.path.getsize(image)
    status = 0
    with open(image, "rb") as fh:
        payloads = payloads_in(fh, size)
        if wanted:
            missing = sorted(set(wanted) - {p.path for p in payloads})
            if missing:
                raise ExtractError(f"no payload at {', '.join(missing)}; payloads: "
                                   f"{', '.join(p.path for p in payloads) or 'none'}")
            payloads = [p for p in payloads if p.path in wanted]
        if not payloads:
            raise ExtractError("the image has no payloads")
        os.makedirs(out_root, mode=0o700, exist_ok=True)
        stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
        out_dir = tempfile.mkdtemp(prefix=f"akai-unpacked-{stamp}-", dir=out_root)
        print(f"image: {os.path.abspath(image)}")
        print(f"output folder: {out_dir}")
        for payload in payloads:
            if payload.offset + payload.length > size:
                print(f"{payload.path}: {payload.where} payload runs past the end of the file; skipped")
                status = 1
                continue
            print(f"{payload.path}: {payload.length} bytes stored ({payload.where})")
            lines, pending = check_stored(fh, payload)
            print("\n".join(lines))
            method = compression_of(fh, payload)
            name = payload.path.replace("/", "_") + ".img"
            final = os.path.join(out_dir, name)
            partial = final + ".partial"
            try:
                with open(partial, "xb") as dst:
                    decompress_to(fh, payload.offset, payload.length, method, dst)
                    dst.flush()
                    os.fsync(dst.fileno())
                if pending:
                    lines, ok = check_output(partial, pending)
                    print("\n".join(lines))
                    if not ok:
                        os.unlink(partial)
                        print(f"{payload.path}: NOT written, because a hash doesn't match")
                        status = 1
                        continue
            except (ExtractError, OSError, EOFError, lzma.LZMAError, zlib.error) as exc:
                if os.path.exists(partial):
                    os.unlink(partial)
                print(f"{payload.path}: NOT written: {exc}")
                status = 1
                continue
            os.replace(partial, final)
            os.chmod(final, 0o400)
            with open(final, "rb") as check:
                kind, head = IMG.guess_region(check, 0, os.path.getsize(final))
            print(f"{payload.path}: compression {method} -> {name}, {os.path.getsize(final)} bytes, {kind}, "
                  f"sha256 {IMG.file_sha256(final)}")
    print("Akai's files are proprietary: keep this folder private and commit only findings.")
    return status


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.strip().split("\n\n")[0])
    parser.add_argument("image")
    parser.add_argument("--node", action="append", default=[],
                        help="payload node path to extract (repeatable); default: all")
    args = parser.parse_args(argv)
    try:
        return extract(args.image, args.node)
    except (ExtractError, IMG.ImageError, OSError) as exc:
        print(f"ERROR: {exc}")
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
