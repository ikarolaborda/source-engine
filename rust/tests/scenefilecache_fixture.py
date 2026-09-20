#!/usr/bin/env python3
"""Inputs for the scenefilecache differential gate.

`synthetic` writes a game directory holding a scenes.image no shipped compiler
would lay out, with stored and LZMA-compressed scenes, and the names to ask
for. `names` scrapes candidate scene names out of shipped archives, which hold
the image's scenes only by checksum.
"""

import argparse
import ctypes
import lzma
from pathlib import Path
import re
import struct
import zlib


def normalized_crc(name):
    clean = name.lower().replace("/", "\\")
    return zlib.crc32(clean.encode("ascii")) & 0xFFFFFFFF


def valve_lzma(payload, encoder):
    """Valve's envelope around a stream from the engine's own encoder, which
    writes no end marker."""
    library = ctypes.CDLL(str(encoder))
    library.LzmaCompress.restype = ctypes.c_int
    destination = ctypes.create_string_buffer(len(payload) * 2 + 1024)
    destination_length = ctypes.c_size_t(len(destination))
    properties = ctypes.create_string_buffer(5)
    properties_length = ctypes.c_size_t(5)
    result = library.LzmaCompress(
        destination, ctypes.byref(destination_length), payload, ctypes.c_size_t(len(payload)),
        properties, ctypes.byref(properties_length), 5, 1 << 16, 3, 0, 2, 32, 1)
    assert result == 0 and properties_length.value == 5
    stream = destination.raw[:destination_length.value]
    return b"LZMA" + struct.pack("<II", len(payload), len(stream)) + properties.raw + stream


def marked_lzma(payload):
    """The same envelope around a liblzma stream, which ends in a marker."""
    alone = lzma.compress(payload, format=lzma.FORMAT_ALONE)
    properties, stream = alone[:5], alone[13:]
    return b"LZMA" + struct.pack("<II", len(payload), len(stream)) + properties + stream


def synthetic(output, encoder):
    strings = [b"npc/first.wav", b"npc/second.wav", b"", b"npc/last.wav"]
    text = bytes(range(256)) * 9
    corrupt = bytearray(valve_lzma(text, encoder))
    corrupt[-40:] = b"\xff" * 40
    scenes = [
        ("scenes/stored.vcd", b"bvcd" + b"stored scene body", 1500, [0, 3]),
        ("scenes/sub/empty_sounds.vcd", b"bvcd", 0, []),
        ("scenes/valve_lzma.vcd", valve_lzma(text, encoder), 7, [1]),
        ("scenes/marked_lzma.vcd", marked_lzma(text), 8, [2, 1, 0]),
        ("scenes/corrupt_lzma.vcd", bytes(corrupt), 9, [3]),
        ("scenes/zero_length.vcd", b"", 10, []),
        ("scenes/x/foo.v2.vcd", b"bvcd-multi-dot", 11, [0]),
        # A sound index past the table and past a short's range.
        ("scenes/wide_sound.vcd", b"bvcd-wide", 12, [0x12345, -1]),
    ]
    scenes.sort(key=lambda scene: normalized_crc(scene[0]))

    image = bytearray(20 + 4 * len(strings))
    for index, string in enumerate(strings):
        struct.pack_into("<I", image, 20 + 4 * index, len(image))
        image += string + b"\0"
    image += b"\xaa" * 3
    entry_offset = len(image)
    image += bytes(16 * len(scenes))
    for index, (name, data, milliseconds, sounds) in enumerate(scenes):
        image += b"\xbb" * 5
        summary = len(image)
        image += struct.pack("<Ii", milliseconds, len(sounds))
        image += b"".join(struct.pack("<i", sound) for sound in sounds)
        offset = len(image)
        image += data
        struct.pack_into("<Iiii", image, entry_offset + 16 * index,
                         normalized_crc(name), offset, len(data), summary)
    image += b"\xcc" * 7
    struct.pack_into("<4siiii", image, 0, b"VSIF", 2, len(scenes), len(strings), entry_offset)

    (output / "game" / "scenes").mkdir(parents=True)
    (output / "game" / "scenes" / "scenes.image").write_bytes(image)
    (output / "bad" / "scenes").mkdir(parents=True)
    (output / "bad" / "scenes" / "scenes.image").write_bytes(b"VSIF" + image[4:8][::-1] + image[8:])
    # '!' tells the harness not to compare what a failed decode left behind.
    (output / "names.txt").write_text("".join(
        ("!" if "corrupt" in name else "") + name + "\n" for name, *_ in scenes))
    print(len(scenes))


def names(archives, output):
    pattern = re.compile(rb"scenes[/\\][A-Za-z0-9_\-./\\]{1,200}?\.vcd", re.IGNORECASE)
    found = set()
    for archive in archives:
        found.update(match.decode("ascii").replace("\\", "/") for match in pattern.findall(archive.read_bytes()))
    output.write_text("".join(name + "\n" for name in sorted(found)))
    print(len(found))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    make = commands.add_parser("synthetic")
    make.add_argument("output", type=Path)
    make.add_argument("encoder", type=Path)
    scrape = commands.add_parser("names")
    scrape.add_argument("output", type=Path)
    scrape.add_argument("archives", type=Path, nargs="+")
    args = parser.parse_args()
    if args.command == "synthetic":
        synthetic(args.output, args.encoder)
    else:
        names(args.archives, args.output)


if __name__ == "__main__":
    main()
