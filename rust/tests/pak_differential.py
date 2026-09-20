#!/usr/bin/env python3
"""Cross-read Python ZIPs / installed BSP ZIP ranges through the public Rust ABI.

Usage: python3 rust/tests/pak_differential.py <libsource_abi> [HL2-content-root]
No game content is modified. Generated fixtures live in a temporary directory.
"""

import ctypes as C
from concurrent.futures import ThreadPoolExecutor
import fnmatch
import io
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import tempfile
import zipfile


class Slice(C.Structure):
    _fields_ = [("data", C.c_void_p), ("length", C.c_uint64)]


class Config(C.Structure):
    _fields_ = [("size", C.c_uint32), ("version", C.c_uint32),
                ("log", C.c_void_p), ("user", C.c_void_p)]


class PakEntry(C.Structure):
    _fields_ = [("offset", C.c_uint64), ("length", C.c_uint32),
                ("compressed", C.c_uint32), ("method", C.c_uint32),
                ("index", C.c_uint32), ("crc", C.c_uint32), ("reserved", C.c_uint32)]


class ArchiveInfo(C.Structure):
    _fields_ = [("offset", C.c_uint64), ("length", C.c_uint64),
                ("modified", C.c_int64), ("count", C.c_uint32), ("reserved", C.c_uint32)]


class SearchPath(C.Structure):
    _fields_ = [("path_id", Slice), ("store_id", C.c_int32), ("flags", C.c_uint32)]


def span(data):
    storage = C.create_string_buffer(bytes(data))
    result = Slice(C.cast(storage, C.c_void_p), len(data))
    result.storage = storage  # Keep borrowed memory alive through the call.
    return result


def check(status):
    assert status == 0, f"ABI status {status}"


class Reader:
    def __init__(self, library):
        self.lib = C.CDLL(str(Path(library).resolve()))
        assert C.sizeof(PakEntry) == 32
        assert C.sizeof(ArchiveInfo) == 32
        for name, args in {
            "create": [Slice, C.POINTER(C.c_uint64), C.POINTER(C.c_uint32)],
            "open_file": [Slice, C.c_uint64, C.c_uint64,
                          C.POINTER(C.c_uint64), C.POINTER(C.c_uint32)],
            "open_archive": [Slice, C.c_uint32, C.POINTER(C.c_uint64), C.POINTER(ArchiveInfo)],
            "destroy": [C.c_uint64],
            "find": [C.c_uint64, Slice, C.POINTER(PakEntry)],
            "entry": [C.c_uint64, C.c_uint32, C.POINTER(PakEntry), C.c_void_p,
                      C.c_uint64, C.POINTER(C.c_uint64)],
        }.items():
            function = getattr(self.lib, "source_pak_index_" + name)
            function.argtypes, function.restype = args, C.c_int32
            setattr(self, "index_" + name, function)
        signatures = {
            "create": [C.POINTER(Config), C.POINTER(C.c_uint64)],
            "destroy": [C.c_uint64],
            "read_paths_clear": [C.c_uint64],
            "read_path_add_pak_flags": [C.c_uint64, Slice, C.c_uint64,
                                       C.c_uint64, Slice, C.c_uint8, C.c_uint8],
            "read_path_add_pak_index": [C.c_uint64, C.c_uint64, Slice, C.c_uint8, C.c_uint8],
            "file_open_read": [C.c_uint64, Slice, Slice,
                               C.POINTER(C.c_uint64), C.POINTER(C.c_uint64)],
            "file_open_pak": [C.c_uint64, C.c_uint64, C.c_uint32,
                              C.POINTER(C.c_uint64), C.POINTER(C.c_uint64),
                              C.POINTER(C.c_uint64)],
            "file_read": [C.c_uint64, C.c_uint64, Slice, C.POINTER(C.c_uint64)],
            "file_seek": [C.c_uint64, C.c_uint64, C.c_int64,
                          C.c_uint32, C.POINTER(C.c_uint64)],
            "file_close": [C.c_uint64, C.c_uint64],
            "find_first_pak": [C.c_uint64, C.c_uint64, Slice, Slice,
                               C.POINTER(C.c_uint64), C.POINTER(C.c_uint32), C.POINTER(C.c_uint64)],
            "find_first": [C.c_uint64, Slice, Slice, Slice,
                           C.POINTER(C.c_uint64), C.POINTER(C.c_uint32), C.POINTER(C.c_uint64)],
            "find_first_bounded": [C.c_uint64, Slice, Slice, C.c_uint32, Slice,
                                   C.POINTER(C.c_uint64), C.POINTER(C.c_uint32), C.POINTER(C.c_uint64)],
            "find_pack_candidates": [C.c_uint64, Slice, C.c_uint32, Slice, Slice,
                                     C.POINTER(C.c_uint64), C.POINTER(C.c_uint32), C.POINTER(C.c_uint64)],
            "find_next": [C.c_uint64, C.c_uint64, Slice, C.POINTER(C.c_uint64), C.POINTER(C.c_uint32)],
            "find_close": [C.c_uint64, C.c_uint64],
        }
        for name, args in signatures.items():
            function = getattr(self.lib, "source_context_" + name)
            function.argtypes = args
            function.restype = C.c_int32
            setattr(self, name, function)
        self.handle = C.c_uint64()
        check(self.create(C.byref(Config(C.sizeof(Config), 1, None, None)),
                          C.byref(self.handle)))

    def mount(self, path, offset, length):
        check(self.read_paths_clear(self.handle))
        return self.read_path_add_pak_flags(self.handle, span(str(path).encode()),
                                            offset, length, span(b"GAME"), 0, 0)

    def read(self, name, expected):
        handle, size = C.c_uint64(), C.c_uint64()
        check(self.file_open_read(self.handle, span(name.encode()), span(b"GAME"),
                                  C.byref(handle), C.byref(size)))
        self.read_opened(handle, size, name, expected)

    def read_opened(self, handle, size, name, expected):
        try:
            assert size.value == len(expected), name
            output = C.create_string_buffer(max(len(expected), 1))
            count = C.c_uint64()
            check(self.file_read(self.handle, handle,
                                Slice(C.cast(output, C.c_void_p), len(expected)),
                                C.byref(count)))
            assert count.value == len(expected) and output.raw[:count.value] == expected, name
            position = C.c_uint64()
            tail = min(17, len(expected))
            check(self.file_seek(self.handle, handle, -tail, 2, C.byref(position)))
            assert position.value == len(expected) - tail
            check(self.file_read(self.handle, handle,
                                Slice(C.cast(output, C.c_void_p), tail), C.byref(count)))
            assert output.raw[:count.value] == expected[len(expected) - tail:]
        finally:
            check(self.file_close(self.handle, handle))

    def compare(self, path, offset, data):
        self.compare_index(data)
        self.compare_index(data, path, offset)
        self.compare_index(data, path, offset, discover=True)
        check(self.mount(path, offset, len(data)))
        count = 0
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for entry in archive.infolist():
                if not entry.is_dir():
                    self.read(entry.filename.replace("\\", "/").lower(), archive.read(entry))
                    count += 1
        return count

    def find_index(self, index, pattern):
        output = C.create_string_buffer(1025)
        target = Slice(C.cast(output, C.c_void_p), len(output))
        written, directory, cursor = C.c_uint64(), C.c_uint32(), C.c_uint64()
        status = self.find_first_pak(self.handle, index, span(pattern.encode()), target,
                                    C.byref(written), C.byref(directory), C.byref(cursor))
        if status == 9:
            assert cursor.value == 0
            return []
        check(status)
        result = []
        try:
            while status == 0:
                assert directory.value in [0, 1]
                result.append((output.raw[:written.value].decode(), bool(directory.value)))
                status = self.find_next(self.handle, cursor, target, C.byref(written), C.byref(directory))
            assert status == 9
        finally:
            check(self.find_close(self.handle, cursor))
        return result

    def compare_index(self, data, path=None, offset=0, discover=False):
        handle, count = C.c_uint64(), C.c_uint32()
        if discover:
            info = ArchiveInfo()
            check(self.index_open_archive(span(str(path).encode()), int(path.suffix == ".bsp"),
                                          C.byref(handle), C.byref(info)))
            assert (info.offset, info.length, info.reserved) == (offset, len(data), 0)
            assert info.modified == path.stat().st_mtime_ns // 1_000_000_000
            count.value = info.count
        elif path is None:
            check(self.index_create(span(data), C.byref(handle), C.byref(count)))
        else:
            check(self.index_open_file(span(str(path).encode()), offset, len(data),
                                      C.byref(handle), C.byref(count)))
        # span(data) is already released: index owns its names/metadata.
        try:
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                entries = {e.filename.replace("\\", "/").lower(): e for e in archive.infolist()
                           if not e.is_dir() and e.filename.lower() != "__preload_section.pre"}
                assert count.value == len(entries)
                for at, (name, expected) in enumerate(sorted(entries.items())):
                    entry, size = PakEntry(), C.c_uint64()
                    assert self.index_entry(handle, at, C.byref(entry), None, 0, C.byref(size)) == 4
                    output = C.create_string_buffer(size.value + 1)
                    check(self.index_entry(handle, at, C.byref(entry), output, size.value, C.byref(size)))
                    assert output.raw[:size.value].decode() == name
                    name_len, extra_len = struct.unpack_from("<HH", data, expected.header_offset + 26)
                    assert entry.offset == expected.header_offset + 30 + name_len + extra_len
                    assert (entry.length, entry.compressed, entry.method, entry.index, entry.crc, entry.reserved) == (
                        expected.file_size, expected.compress_size, expected.compress_type, at, expected.CRC, 0)
                    found = PakEntry()
                    check(self.index_find(handle, span(("./" + name.upper()).encode()), C.byref(found)))
                    assert bytes(found) == bytes(entry)
                    if path is not None:
                        file, file_size, absolute = C.c_uint64(), C.c_uint64(), C.c_uint64()
                        check(self.file_open_pak(self.handle, handle, at, C.byref(file),
                                                C.byref(file_size), C.byref(absolute)))
                        assert absolute.value == offset + entry.offset
                        self.read_opened(file, file_size, name, archive.read(expected))
                assert self.index_entry(handle, count, C.byref(PakEntry()), None, 0, C.byref(C.c_uint64())) == 9
                assert self.index_find(handle, span(b"../escape"), C.byref(PakEntry())) == 9
                if path is None:
                    for pattern in ["*", "*.*", "materials/*.vmt", "materials/c*", "MATERIALS/*.?MT"]:
                        assert self.find_index(handle, pattern) == find_oracle(entries, pattern), pattern
        finally:
            check(self.index_destroy(handle))
        assert self.index_destroy(handle) == 3
        assert self.index_find(handle, span(b"a"), C.byref(PakEntry())) == 3


def synthetic(reader, root):
    rng = random.Random(227078363354759168)
    count = 0
    for method in [zipfile.ZIP_STORED, zipfile.ZIP_LZMA]:
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w", compression=method) as archive:
            for index, length in enumerate([0, 1, 2, 3, 4, 15, 16, 255, 256, 4096, 65536]):
                archive.writestr(f"materials/random{index}.vmt", rng.randbytes(length))
                archive.writestr(f"materials/repeat{index}.vmt", b"a" * length)
        data = stream.getvalue()
        if seed_dir := os.environ.get("SOURCE_PAK_FUZZ_CORPUS"):
            # Optional generated seeds, never installed or proprietary content.
            (Path(seed_dir) / f"method{method}.zip").write_bytes(data)
        path = root / f"method{method}.bsp"
        header = bytearray(1036)
        struct.pack_into("<4si", header, 0, b"VBSP", 20)
        struct.pack_into("<ii", header, 8 + 40 * 16, 1036, len(data))
        path.write_bytes(header + data + b"BSP suffix")
        (root / f"method{method}.zip").write_bytes(data)
        count += reader.compare(path, 1036, data)
        assert reader.mount(path, 1036, len(data) + 100) == 7
        assert reader.mount(path, 2**64 - 1, 2) == 7
        assert reader.mount(path, 0, len(data)) == 8
        # Neither an invalid bool nor a stale context is accepted by the new API.
        assert reader.read_path_add_pak_flags(reader.handle, span(str(path).encode()),
                                             1036, len(data), span(b"GAME"), 2, 0) == 1
        assert reader.read_path_add_pak_flags(0, span(str(path).encode()),
                                             1036, len(data), span(b"GAME"), 0, 0) == 3
    print(f"ZIP ABI differential: {count} stored/LZMA entries, size/read/seek agree with Python", flush=True)
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        archive.writestr("__preload_section.pre", b"cache not interpreted")
        archive.writestr("test/a.txt", b"actual payload")
    reader.compare_index(stream.getvalue())
    (root / "preload.zip").write_bytes(stream.getvalue())
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        archive.writestr("a.txt", b"payload")
    data = bytearray(stream.getvalue())
    old_cd = struct.unpack_from("<I", data, len(data) - 6)[0]
    # Local extra field intentionally differs from the central extra field.
    data[35:35] = b"\xfe\xca\x04\x00more"
    struct.pack_into("<H", data, 28, 8)
    struct.pack_into("<I", data, len(data) - 6, old_cd + 8)
    reader.compare_index(data)
    path = root / "extra.zip"
    path.write_bytes(data)
    reader.compare(path, 0, data)
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        archive.writestr("lines.txt", b"first\r\nsecond\r\nlast")
    text_path = root / "text.zip"
    text_path.write_bytes(stream.getvalue())
    reader.compare(text_path, 0, stream.getvalue())
    for bad in [b"invalid", bytes(data[:-1])]:
        handle, count = C.c_uint64(99), C.c_uint32(99)
        assert reader.index_create(span(bad), C.byref(handle), C.byref(count)) == 8
        assert handle.value == count.value == 0
    handle, count = C.c_uint64(), C.c_uint32()
    assert reader.index_create(Slice(None, 512 * 1024 * 1024 + 1), C.byref(handle), C.byref(count)) == 8
    assert reader.index_create(span(data), None, C.byref(count)) == 1
    assert reader.index_create(span(data), C.byref(handle), None) == 1
    check(reader.index_create(span(data), C.byref(handle), C.byref(count)))
    def concurrent_query(_):
        entry, written = PakEntry(), C.c_uint64()
        short = C.create_string_buffer(b"ZZ")
        assert reader.index_entry(handle, 0, C.byref(entry), short, 1, C.byref(written)) == 4
        assert short.value == b"ZZ" and written.value == len("a.txt")
        assert reader.index_find(handle, span(b"a.txt"), None) == 1
        check(reader.index_find(handle, span(b"a.txt"), C.byref(entry)))
        assert entry.length == 7
    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(concurrent_query, range(1000)))
    check(reader.index_destroy(handle))
    for _ in range(10_000):
        reader.compare_index(data)
    print("ZIP metadata index: offsets/names/metadata match Python; preload hidden; 10000 lifecycles and concurrent queries passed", flush=True)
    file_index_cases(reader, root, data)
    shared_owner_cases(reader, root)
    selection_cases(reader, root)
    search_plan_cases(reader)
    mount_table_cases(reader)
    discovery_cases(reader, root, bytes(data))
    wildcard_cases(reader, root)
    transitions = root / "map-transitions"
    transitions.mkdir()
    for name, payload in [("a", b"A"), ("b", b"B")]:
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_LZMA) as archive:
            archive.writestr("transition.txt", payload)
            archive.writestr(f"only-{name}.txt", payload)
        zip_bytes = stream.getvalue()
        header = bytearray(1036)
        struct.pack_into("<4si", header, 0, b"VBSP", 20)
        struct.pack_into("<ii", header, 8 + 40 * 16, len(header), len(zip_bytes))
        (transitions / f"{name}.bsp").write_bytes(header + zip_bytes)
    shared = root / "shared-owner"
    (shared / "extra").mkdir(parents=True)
    (shared / "a.bsp").write_bytes((transitions / "a.bsp").read_bytes())
    (shared / "replacement.bsp").write_bytes((transitions / "b.bsp").read_bytes())


def shared_owner_cases(reader, root):
    folder = root / "shared-abi"
    folder.mkdir()
    path = folder / "shared.zip"
    def write_archive(payload):
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("dir/a.txt", payload)
            archive.writestr("__preload_section.pre", b"hidden")
    write_archive(b"original")
    index, info = C.c_uint64(), ArchiveInfo()
    check(reader.index_open_archive(span(os.fsencode(path)), 0, C.byref(index), C.byref(info)))
    check(reader.read_paths_clear(reader.handle))
    for context, handle, path_id, head, private, expected in [
        (0, index.value, b"GAME", 0, 0, 3),
        (reader.handle.value, 2**64-1, b"GAME", 0, 0, 3),
        (reader.handle.value, index.value, b"", 0, 0, 1),
        (reader.handle.value, index.value, b"\xff", 0, 0, 1),
        (reader.handle.value, index.value, b"GAME", 2, 0, 1),
        (reader.handle.value, index.value, b"GAME", 0, 2, 1),
    ]:
        assert reader.read_path_add_pak_index(context, handle, span(path_id), head, private) == expected
    assert reader.read_path_add_pak_index(reader.handle, index, Slice(None, 1), 0, 0) == 1
    check(reader.read_path_add_pak_index(reader.handle, index, span(b"PRIVATE"), 1, 1))
    file, size = C.c_uint64(), C.c_uint64()
    assert reader.file_open_read(reader.handle, span(b"dir/a.txt"), span(b""), C.byref(file), C.byref(size)) == 9
    check(reader.file_open_read(reader.handle, span(b"dir/a.txt"), span(b"private"), C.byref(file), C.byref(size)))
    reader.read_opened(file, size, "private", b"original")
    path.rename(folder / "original.zip")
    write_archive(b"replacement")
    check(reader.read_path_add_pak_index(reader.handle, index, span(b"GAME"), 0, 0))
    reader.read("dir/a.txt", b"original")
    assert reader.file_open_read(reader.handle, span(b"__preload_section.pre"), span(b"GAME"), C.byref(file), C.byref(size)) == 9
    # Context references share the same archive and outlive the public index.
    second = C.c_uint64()
    check(reader.create(C.byref(Config(C.sizeof(Config), 1, None, None)), C.byref(second)))
    try:
        check(reader.read_path_add_pak_index(second, index, span(b"GAME"), 0, 0))
        check(reader.index_destroy(index))
        assert reader.read_path_add_pak_index(reader.handle, index, span(b"GAME"), 0, 0) == 3
        reader.read("dir/a.txt", b"original")
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(lambda _: reader.read("dir/a.txt", b"original"), range(1000)))
        check(reader.file_open_read(reader.handle, span(b"dir/a.txt"), span(b"GAME"), C.byref(file), C.byref(size)))
        check(reader.read_paths_clear(reader.handle))
        reader.read_opened(file, size, "unmounted", b"original")
        other_file, other_size = C.c_uint64(), C.c_uint64()
        check(reader.file_open_read(second, span(b"dir/a.txt"), span(b"GAME"), C.byref(other_file), C.byref(other_size)))
        output, count = C.create_string_buffer(8), C.c_uint64()
        check(reader.file_read(second, other_file, Slice(C.cast(output, C.c_void_p), 8), C.byref(count)))
        assert output.raw == b"original"
        check(reader.file_close(second, other_file))
    finally:
        check(reader.destroy(second))
    # A fresh open after all old mounts close sees the replacement, not a cache.
    check(reader.index_open_archive(span(os.fsencode(path)), 0, C.byref(index), C.byref(info)))
    check(reader.read_path_add_pak_index(reader.handle, index, span(b"GAME"), 1, 0))
    reader.read("dir/a.txt", b"replacement")
    entry = PakEntry()
    check(reader.index_find(index, span(b"dir/a.txt"), C.byref(entry)))
    with path.open("r+b") as writer:
        writer.seek(entry.offset)
        writer.write(b"!")
    assert reader.file_open_read(reader.handle, span(b"dir/a.txt"), span(b"GAME"), C.byref(file), C.byref(size)) == 8
    assert file.value == size.value == 0
    direct, direct_size, absolute = C.c_uint64(), C.c_uint64(), C.c_uint64()
    assert reader.file_open_pak(reader.handle, index, entry.index, C.byref(direct), C.byref(direct_size), C.byref(absolute)) == 8
    check(reader.read_paths_clear(reader.handle))
    check(reader.index_destroy(index))
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        archive.writestr("a", b"metadata only")
    count = C.c_uint32()
    check(reader.index_create(span(raw.getvalue()), C.byref(index), C.byref(count)))
    assert reader.read_path_add_pak_index(reader.handle, index, span(b"GAME"), 0, 0) == 1
    check(reader.index_destroy(index))
    print("Shared pack owner: no-reopen replacement, by-request flags, hidden preload, index destruction, cross-context mounts, 1000 concurrent reads, corruption parity and fresh-remount checks passed", flush=True)


def selection_cases(reader, root):
    matches = reader.lib.source_read_path_matches
    matches.argtypes = [Slice, Slice, C.c_uint8, C.c_uint8, C.c_uint8, C.POINTER(C.c_uint32)]
    matches.restype = C.c_int32
    ids = [b"", b"GAME", b"game", b"MOD", b"BSP", b"PRIVATE", "éID".encode(), "ÉID".encode()]
    cases = 0
    for stored in ids:
        for requested in [None, *ids, b"bSp"]:
            for private in [0, 1]:
                for is_map in [0, 1]:
                    result = C.c_uint32(99)
                    check(matches(span(stored), span(requested or b""), requested is not None,
                                  private, is_map, C.byref(result)))
                    # bytes.lower deliberately folds ASCII only, like native path symbols.
                    expected = (not private if requested is None else
                                stored.lower() == b"game" and bool(is_map) if requested.lower() == b"bsp" else
                                stored.lower() == requested.lower())
                    assert result.value == expected, (stored, requested, private, is_map)
                    cases += 1
    for stored, requested, has, private, is_map in [
        (span(b"GAME"), span(b""), 2, 0, 0),
        (span(b"GAME"), span(b""), 0, 2, 0),
        (span(b"GAME"), span(b""), 0, 0, 2),
        (span(b"GAME"), span(b"GAME"), 0, 0, 0),
        (span(b"\xff"), span(b""), 0, 0, 0),
        (span(b"GAME"), span(b"\xff"), 1, 0, 0),
        (Slice(None, 1), span(b""), 0, 0, 0),
        (span(b"GAME"), Slice(None, 1), 1, 0, 0),
    ]:
        result = C.c_uint32(99)
        assert matches(stored, requested, has, private, is_map, C.byref(result)) == 1
        assert result.value == 0
    assert matches(span(b"GAME"), span(b""), 0, 0, 0, None) == 1

    folder = root / "path-selection"
    folder.mkdir()
    for name, payload in [("loose", b"L"), ("literal", b"B"), ("zip", b"Z")]:
        directory = folder / name
        directory.mkdir()
        if name == "zip":
            with zipfile.ZipFile(directory / "zip0.zip", "w") as archive:
                archive.writestr("choice.txt", payload)
                archive.writestr("zip-only.txt", payload)
        else:
            (directory / "choice.txt").write_bytes(payload)
            (directory / f"{name}-only.txt").write_bytes(payload)
    names = ["choice.txt", "map-only.txt", "dir/a.txt", "a" * 300, "x" * 300]
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        for name in names:
            archive.writestr(name, b"M")
        archive.writestr("dir/lines.txt", b"first\r\nsecond\r\nlast")
    data = stream.getvalue()
    header = bytearray(1036)
    struct.pack_into("<4si", header, 0, b"VBSP", 20)
    struct.pack_into("<ii", header, 648, len(header), len(data))
    map_path = folder / "map.bsp"
    map_path.write_bytes(header + data)
    # Misleading extensions must not determine typed provenance.
    zip_path = folder / "standalone.bsp"
    zip_path.write_bytes(data)
    bsp_path = folder / "embedded.zip"
    bsp_path.write_bytes(header + data)
    handles = []
    check(reader.read_paths_clear(reader.handle))
    for path, kind, path_id in [(zip_path, 0, b"GAME"), (bsp_path, 1, b"MOD")]:
        index, info = C.c_uint64(), ArchiveInfo()
        check(reader.index_open_archive(span(os.fsencode(path)), kind, C.byref(index), C.byref(info)))
        handles.append(index)
        check(reader.read_path_add_pak_index(reader.handle, index, span(path_id), 0, 0))
    raw, count = C.c_uint64(), C.c_uint32()
    check(reader.index_open_file(span(os.fsencode(bsp_path)), 1036, len(data), C.byref(raw), C.byref(count)))
    handles.append(raw)
    check(reader.read_path_add_pak_index(reader.handle, raw, span(b"GAME"), 0, 0))
    file, size = C.c_uint64(), C.c_uint64()
    assert reader.file_open_read(reader.handle, span(b"choice.txt"), span(b"BSP"), C.byref(file), C.byref(size)) == 9
    check(reader.read_path_add_pak_index(reader.handle, handles[1], span(b"GAME"), 0, 1))
    check(reader.file_open_read(reader.handle, span(b"DIR\\.\\sub\\..\\A.TXT"), span(b"bSp"), C.byref(file), C.byref(size)))
    reader.read_opened(file, size, "typed BSP", b"M")
    for index in handles:
        check(reader.index_destroy(index))

    output = C.create_string_buffer(1025)
    target = Slice(C.cast(output, C.c_void_p), len(output))
    written, directory, cursor = C.c_uint64(), C.c_uint32(), C.c_uint64()
    pointers = (C.byref(written), C.byref(directory), C.byref(cursor))
    def first(limit, buffer=target):
        return reader.find_first_bounded(reader.handle, span(b"*"), span(b"BSP"), limit, buffer, *pointers)
    assert first(0) == 1 and cursor.value == 0
    assert first(255, Slice(None, 1)) == 1 and cursor.value == 0
    short = C.create_string_buffer(b"ZZZZ")
    assert first(255, Slice(C.cast(short, C.c_void_p), 4)) == 4
    assert short.value == b"ZZZZ" and written.value == len("choice.txt") and cursor.value == 0
    check(reader.find_first(reader.handle, span(b"*"), span(b"BSP"), target, *pointers))
    assert output.raw[:written.value] == b"a" * 300  # Unbounded ABI is unchanged.
    check(reader.find_close(reader.handle, cursor))
    check(first(255))
    assert output.raw[:written.value] == b"choice.txt"
    check(reader.read_paths_clear(reader.handle))  # Cursor snapshot outlives every owner.
    assert reader.find_next(reader.handle, cursor, Slice(None, 0), C.byref(written), C.byref(directory)) == 4
    assert written.value == len("map-only.txt")
    for name, is_dir in [(b"map-only.txt", 0), (b"dir", 1)]:
        check(reader.find_next(reader.handle, cursor, target, C.byref(written), C.byref(directory)))
        assert (output.raw[:written.value], directory.value) == (name, is_dir)
    assert reader.find_next(reader.handle, cursor, target, C.byref(written), C.byref(directory)) == 9
    check(reader.find_close(reader.handle, cursor))
    print(f"Read-path selection: {cases} policy combinations, fail-closed ABI, typed BSP provenance, normalized reads and bounded/unbounded cursor lifetimes passed", flush=True)


def search_plan_cases(reader):
    assert C.sizeof(SearchPath) == 24
    functions = {}
    for name, args in {
        "plan_create": [C.POINTER(SearchPath), C.c_uint32, Slice, C.c_uint8, C.c_uint32, C.POINTER(C.c_uint64)],
        "plan_next": [C.c_uint64, C.POINTER(C.c_uint32)],
        "visits_create": [C.POINTER(C.c_uint64)],
        "visits_mark": [C.c_uint64, C.c_int32, C.POINTER(C.c_uint32)],
        "state_reset": [C.c_uint64], "state_destroy": [C.c_uint64],
    }.items():
        function = getattr(reader.lib, "source_search_" + name)
        function.argtypes, function.restype = args, C.c_int32
        functions[name] = function
    create, next_, visits_create, mark, reset, destroy = (functions[name] for name in
        ["plan_create", "plan_next", "visits_create", "visits_mark", "state_reset", "state_destroy"])
    rng = random.Random(20260920)
    ids = [b"GAME", b"game", b"MOD", b"BSP", b"", b"PRIVATE"]
    for _ in range(2000):
        requested = rng.choice([None, *ids])
        filter_ = rng.randrange(3)
        entries = [(rng.choice(ids), rng.choice([-2**31, -1, 0, 1, 2, 3, 2**31-1]),
                    rng.choice([0, 1, 3]) | rng.choice([0, 4]) | rng.choice([0, 8]))
                   for _ in range(rng.randrange(65))]
        seen, expected = set(), []
        for index, (id_, store, flags) in enumerate(entries):
            if filter_ == 1 and flags & 1 or filter_ == 2 and not flags & 1:
                continue
            if requested is None:
                if flags & 4:
                    continue
            elif requested.lower() == b"bsp":
                if id_.lower() != b"game" or not flags & 2:
                    continue
            elif requested.lower() != id_.lower():
                continue
            if flags & 8 or store in seen:
                continue
            seen.add(store)
            expected.append(index)
        spans = [span(entry[0]) for entry in entries]
        paths = (SearchPath * len(entries))(*(SearchPath(s, e[1], e[2]) for s, e in zip(spans, entries)))
        handle = C.c_uint64()
        check(create(paths, len(paths), span(requested or b""), requested is not None, filter_, C.byref(handle)))
        # The caller may discard all IDs/descriptors after construction.
        for path in paths:
            path.path_id, path.store_id, path.flags = Slice(None, 0), 123, 15
        del paths, spans
        seen_out = C.c_uint32(99)
        assert mark(handle, 0, C.byref(seen_out)) == 3 and seen_out.value == 1
        assert next_(handle, None) == 1  # No cursor advance.
        def drain():
            result, index = [], C.c_uint32()
            while True:
                status = next_(handle, C.byref(index))
                if status == 9:
                    assert index.value == 2**32-1
                    break
                check(status)
                result.append(index.value)
            return result
        assert drain() == expected
        check(reset(handle))
        assert drain() == expected
        check(destroy(handle))
        assert destroy(handle) == reset(handle) == 3
        out = C.c_uint32(99)
        assert next_(handle, C.byref(out)) == 3 and out.value == 2**32-1
    handle = C.c_uint64(99)
    valid = (SearchPath * 1)(SearchPath(span(b"GAME"), 0, 0))
    for paths, count, request, has, filter_ in [
        (None, 1, span(b""), 0, 0), (None, 65537, span(b""), 0, 0),
        (valid, 1, span(b""), 2, 0), (valid, 1, span(b""), 0, 3),
        (valid, 1, span(b"GAME"), 0, 0), (valid, 1, Slice(None, 1), 1, 0),
        (valid, 1, span(b"\xff"), 1, 0), (valid, 1, Slice(None, 4097), 1, 0),
    ]:
        assert create(paths, count, request, has, filter_, C.byref(handle)) == 1 and handle.value == 0
    for id_, flags in [(span(b"GAME"), 2), (span(b"GAME"), 16), (span(b"\xff"), 0),
                       (Slice(None, 1), 0), (Slice(None, 4097), 0)]:
        invalid = (SearchPath * 1)(SearchPath(id_, 0, flags))
        assert create(invalid, 1, span(b""), 0, 0, C.byref(handle)) == 1 and handle.value == 0
    assert create(None, 0, span(b""), 0, 0, None) == 1
    assert visits_create(None) == 1
    check(visits_create(C.byref(handle)))
    out = C.c_uint32(99)
    assert next_(handle, C.byref(out)) == 3 and out.value == 2**32-1
    assert mark(handle, 55, None) == 1
    check(mark(handle, 55, C.byref(out)))
    assert out.value == 0
    check(reset(handle))
    def concurrent_mark(_):
        result = C.c_uint32()
        check(mark(handle, -2**31, C.byref(result)))
        return result.value
    with ThreadPoolExecutor(max_workers=4) as pool:
        result = list(pool.map(concurrent_mark, range(1000)))
    assert result.count(0) == 1 and result.count(1) == 999
    check(destroy(handle))
    assert mark(handle, 0, C.byref(out)) == 3 and out.value == 1
    # Repeated context-independent lifecycle: no active bridge/context needed.
    for _ in range(10000):
        check(visits_create(C.byref(handle)))
        check(destroy(handle))
    id_ = span(b"GAME")
    paths = (SearchPath * 1000)(*(SearchPath(id_, index, 0) for index in range(1000)))
    check(create(paths, len(paths), span(b"GAME"), 1, 0, C.byref(handle)))
    def concurrent_next(_):
        result = C.c_uint32()
        check(next_(handle, C.byref(result)))
        return result.value
    with ThreadPoolExecutor(max_workers=4) as pool:
        assert sorted(pool.map(concurrent_next, range(1000))) == list(range(1000))
    assert next_(handle, C.byref(out)) == 9 and out.value == 2**32-1
    check(destroy(handle))
    print("Search plan oracle: 2000 ordered/filter/dedup snapshots, released inputs, replay/exhaustion, invalid/stale/wrong-kind handles; 1000 concurrent visits/next calls and 10000 lifecycles passed", flush=True)


def mount_table_cases(reader):
    drop_type = C.CFUNCTYPE(None, C.c_void_p)
    clone_type = C.CFUNCTYPE(C.c_void_p, C.c_void_p)
    functions = {}
    for name, args in {
        "create": [drop_type, clone_type, C.POINTER(C.c_uint64)],
        "insert": [C.c_uint64, C.c_uint32, C.c_void_p],
        "count": [C.c_uint64, C.POINTER(C.c_uint32)],
        "get": [C.c_uint64, C.c_uint32, C.POINTER(C.c_void_p)],
        "remove": [C.c_uint64, C.c_uint32, C.c_uint8],
        "clear": [C.c_uint64], "snapshot": [C.c_uint64, C.POINTER(C.c_uint64)],
        "destroy": [C.c_uint64],
    }.items():
        function = getattr(reader.lib, "source_mount_table_" + name)
        function.argtypes, function.restype = args, C.c_int32
        functions[name] = function
    create, insert, count, get, remove, clear, snapshot, destroy = (functions[n] for n in
        ["create", "insert", "count", "get", "remove", "clear", "snapshot", "destroy"])
    live, drops, callback_errors, observed = {}, [], [], []
    serial = [0]
    source = C.c_uint64()
    clone_mode, clone_calls = ["normal"], [0]
    def allocate(value):
        serial[0] += 1
        live[serial[0]] = value
        return serial[0]
    def observe():
        size = C.c_uint32(999)
        status = count(source, C.byref(size))
        if status not in [0, 3]:
            raise AssertionError((status, size.value))
        observed.append((status, size.value))
    @drop_type
    def drop_resource(token):
        try:
            observe()  # Reentrant API call must not hold a registry/table lock.
            del live[token]
            drops.append(token)
        except BaseException as error:
            callback_errors.append(repr(error))
    @clone_type
    def clone_resource(token):
        try:
            observe()
            clone_calls[0] += 1
            if clone_mode[0] == "fail" and clone_calls[0] == 2:
                return None
            if clone_mode[0] == "destroy" and clone_calls[0] == 1:
                check(destroy(source))
                # All source resources remain leased through every callback.
                assert token in live
            return allocate(live[token])
        except BaseException as error:
            callback_errors.append(repr(error))
            return None
    def new_table():
        handle = C.c_uint64()
        check(create(drop_resource, clone_resource, C.byref(handle)))
        return handle
    def values(handle):
        size = C.c_uint32()
        check(count(handle, C.byref(size)))
        result = []
        for index in range(size.value):
            token = C.c_void_p()
            check(get(handle, index, C.byref(token)))
            result.append(live[token.value])
        return result
    source = new_table()
    expected = []
    rng = random.Random(7721)
    for iteration in range(2000):
        action = rng.randrange(5)
        if not expected or action <= 1:
            index = rng.randrange(len(expected) + 1)
            token = allocate(iteration)
            check(insert(source, index, token))
            expected.insert(index, iteration)
        elif action == 2:
            index = rng.randrange(len(expected))
            fast = rng.randrange(2)
            check(remove(source, index, fast))
            if fast:
                expected[index] = expected[-1]
                expected.pop()
            else:
                expected.pop(index)
        elif action == 3:
            copy = C.c_uint64()
            check(snapshot(source, C.byref(copy)))
            assert values(copy) == expected
            check(clear(source))
            assert values(copy) == expected
            expected.clear()
            check(destroy(copy))
        else:
            check(clear(source))
            expected.clear()
        assert values(source) == expected
        assert not callback_errors, callback_errors
    check(clear(source))
    token = allocate(99)
    before = len(drops)
    assert insert(source, 1, token) == 1
    assert insert(0, 0, token) == 3
    assert insert(source, 0, None) == 1
    assert token in live and len(drops) == before  # Failed transfer leaves caller ownership.
    check(insert(source, 0, token))
    assert remove(source, 0, 2) == 1
    assert remove(source, 1, 0) == 9
    assert values(source) == [99]
    out = C.c_void_p(99)
    assert get(source, 1, C.byref(out)) == 9 and out.value is None
    assert get(source, 0, None) == count(source, None) == snapshot(source, None) == 1
    check(insert(source, 1, allocate(100)))
    check(insert(source, 2, allocate(101)))
    clone_mode[0], clone_calls[0] = "fail", 0
    copy = C.c_uint64(99)
    before = set(live)
    assert snapshot(source, C.byref(copy)) == 6 and copy.value == 0
    assert set(live) == before and values(source) == [99, 100, 101]
    clone_mode[0], clone_calls[0] = "destroy", 0
    check(snapshot(source, C.byref(copy)))
    assert values(copy) == [99, 100, 101]
    assert not (set(live) & before)  # Originals released only after clone completion.
    assert destroy(source) == clear(source) == 3
    check(destroy(copy))
    assert not live and not callback_errors
    assert len(drops) == len(set(drops))
    assert (0, 0) in observed and (3, 0) in observed
    out_handle, size = C.c_uint64(99), C.c_uint32(99)
    assert create(drop_type(), clone_resource, C.byref(out_handle)) == 1 and out_handle.value == 0
    assert create(drop_resource, clone_type(), C.byref(out_handle)) == 1 and out_handle.value == 0
    assert create(drop_resource, clone_resource, None) == 1
    assert count(source, C.byref(size)) == 3 and size.value == 0
    assert get(source, 0, C.byref(out)) == 3 and out.value is None
    assert snapshot(source, C.byref(out_handle)) == 3 and out_handle.value == 0
    source = new_table()
    for index in range(65536):
        check(insert(source, index, allocate(index)))
    extra = allocate(-1)
    before = len(drops)
    assert insert(source, 65536, extra) == 1
    assert extra in live and len(drops) == before
    del live[extra]  # Caller still owns this failed transfer.
    check(count(source, C.byref(size)))
    assert size.value == 65536
    check(destroy(source))
    assert not live and not callback_errors and len(drops) == len(set(drops))
    for _ in range(10000):
        source = new_table()
        check(destroy(source))
    next_id = reader.lib.source_mount_store_id_next
    next_id.argtypes, next_id.restype = [C.POINTER(C.c_int32)], C.c_int32
    assert next_id(None) == 1
    def allocate_id(_):
        result = C.c_int32()
        check(next_id(C.byref(result)))
        return result.value
    with ThreadPoolExecutor(max_workers=4) as pool:
        ids = list(pool.map(allocate_id, range(1000)))
    assert len(set(ids)) == 1000 and min(ids) > 0
    assert max(ids) - min(ids) == 999
    print("Mount table oracle: 2000 mutation/snapshot cycles, exact callback ownership, failure rollback, reentrant query/destroy, retained clone leases, 65536-entry bound, 10000 lifecycles and 1000 concurrent store IDs passed", flush=True)


def find_oracle(paths, pattern):
    # Fixtures use only the documented * and ? syntax (no fnmatch classes).
    parts = pattern.replace("\\", "/").lower().split("/")
    directory = []
    for part in parts[:-1]:
        if part == "..":
            directory.pop()
        elif part not in ["", "."]:
            directory.append(part)
    prefix = "/".join(directory) + ("/" if directory else "")
    files, directories = set(), set()
    for path in paths:
        path = path.replace("\\", "/").lower()
        if path == "__preload_section.pre" or not path.startswith(prefix):
            continue
        name, slash, _ = path[len(prefix):].partition("/")
        if name and (parts[-1] == "*.*" or fnmatch.fnmatchcase(name, parts[-1])):
            (directories if slash else files).add(prefix + name)
    return [(name, False) for name in sorted(files)] + [(name, True) for name in sorted(directories - files)]


def wildcard_cases(reader, root):
    names = ["README", "root.multi.txt", "dir/a.vmt", "dir/b.vmt", "dir/multi.part.vmt",
             "dir/sub/x.txt", "dir/sub/y.txt", "dir/dotted.dir/z", "dir/same", "dir/same/child",
             "dir/.hidden", "dir/\u00e9.vmt", "__preload_section.pre", "x" * 300 + ".vmt"]
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        for name in names:
            archive.writestr(name, b"payload")
    data = stream.getvalue()
    header = bytearray(1036)
    struct.pack_into("<4si", header, 0, b"VBSP", 20)
    struct.pack_into("<ii", header, 8 + 40 * 16, 1036, len(data))
    (root / "wildcards.bsp").write_bytes(header + data)
    index, count = C.c_uint64(), C.c_uint32()
    check(reader.index_create(span(data), C.byref(index), C.byref(count)))
    patterns = ["*.*", "dir/*", "DIR/*.VMT", "dir/*a*.?mt", "dir/s*", "dir/dotted.*",
                "root.multi.*", "dir\\.\\sub\\..\\*.vmt", "./*", "dir//*.vmt", "*vmt", "missing/*"]
    rng = random.Random(334)
    patterns += [rng.choice(["", "dir/", "dir/sub/"]) + "".join(rng.choices("abc*.?", k=rng.randrange(1, 10)))
                 for _ in range(500)]
    patterns = [pattern for pattern in patterns if pattern.rsplit("/", 1)[-1] not in [".", ".."]]
    for pattern in patterns:
        assert reader.find_index(index, pattern) == find_oracle(names, pattern), pattern
    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(lambda _: reader.find_index(index, "dir/*"), range(1000)))
    output = C.create_string_buffer(1025)
    target = Slice(C.cast(output, C.c_void_p), len(output))
    written, directory, cursor = C.c_uint64(), C.c_uint32(), C.c_uint64()
    for pattern in ["", "../*", "dir/../../*", "/dir/*", "C:dir/*", "dir/", "dir*/x", "dir/?/x", "a\0b", "x" * 4097]:
        assert reader.find_first_pak(reader.handle, index, span(pattern.encode()), target,
                                    C.byref(written), C.byref(directory), C.byref(cursor)) == 1
        assert cursor.value == written.value == directory.value == 0
    for context, handle in [(0, index), (reader.handle, 0)]:
        assert reader.find_first_pak(context, handle, span(b"*"), target,
                                    C.byref(written), C.byref(directory), C.byref(cursor)) == 3
    for pointers in [(None, C.byref(directory), C.byref(cursor)),
                     (C.byref(written), None, C.byref(cursor)), (C.byref(written), C.byref(directory), None)]:
        assert reader.find_first_pak(reader.handle, index, span(b"*"), target, *pointers) == 1
    assert reader.find_first_pak(reader.handle, index, span(b"*"), Slice(None, 1),
                                C.byref(written), C.byref(directory), C.byref(cursor)) == 1
    short = C.create_string_buffer(b"ZZ")
    assert reader.find_first_pak(reader.handle, index, span(b"*"), Slice(C.cast(short, C.c_void_p), 1),
                                C.byref(written), C.byref(directory), C.byref(cursor)) == 4
    assert written.value == len("readme") and short.value == b"ZZ" and cursor.value == 0
    check(reader.find_first_pak(reader.handle, index, span(b"*"), target,
                               C.byref(written), C.byref(directory), C.byref(cursor)))
    check(reader.index_destroy(index))
    assert reader.find_next(reader.handle, cursor, Slice(None, 0), C.byref(written), C.byref(directory)) == 4
    assert written.value == len("root.multi.txt")
    check(reader.find_next(reader.handle, cursor, target, C.byref(written), C.byref(directory)))
    assert output.raw[:written.value] == b"root.multi.txt"
    check(reader.find_close(reader.handle, cursor))
    assert reader.find_close(reader.handle, cursor) == 9
    assert reader.find_next(reader.handle, cursor, target, C.byref(written), C.byref(directory)) == 9
    print("Pack wildcard oracle: roots/dots/partial globs/directories/long names, invalid paths, short/stale cursors, unmount survival and 1000 concurrent cursors passed", flush=True)


def discovery_cases(reader, root, data):
    header = bytearray(1036)
    struct.pack_into("<4si", header, 0, b"VBSP", 20)
    struct.pack_into("<ii", header, 8 + 40 * 16, len(header), len(data))
    good = bytes(header) + data
    valid = root / "discovery.tmp"
    valid.write_bytes(good)
    filename = span(str(valid).encode())
    handle, info = C.c_uint64(), ArchiveInfo()
    assert reader.index_open_archive(filename, 2, C.byref(handle), C.byref(info)) == 1
    assert reader.index_open_archive(filename, 1, None, C.byref(info)) == 1
    assert reader.index_open_archive(filename, 1, C.byref(handle), None) == 1
    for argument, expected in [(span(b""), 1), (span(b"\xff"), 1),
                               (span(b"bad\0path"), 1), (Slice(None, 1), 1),
                               (span(str(root).encode()), 1),
                               (span(str(root / "missing").encode()), 7)]:
        assert reader.index_open_archive(argument, 1, C.byref(handle), C.byref(info)) == expected
        assert handle.value == 0 and bytes(info) == bytes(C.sizeof(info))
    for version in [19, 20, 21]:
        changed = bytearray(good)
        struct.pack_into("<i", changed, 4, version)
        valid.write_bytes(changed)
        if seed_dir := os.environ.get("SOURCE_BSP_FUZZ_CORPUS"):
            (Path(seed_dir) / f"version{version}.bsp").write_bytes(changed)
        check(reader.index_open_archive(filename, 1, C.byref(handle), C.byref(info)))
        assert (info.offset, info.length, info.count) == (1036, len(data), 1)
        check(reader.index_destroy(handle))
    invalid = [good[:1035], b"NOPE" + good[4:]]
    for field, value in [(4, 18), (4, 22), (8 + 40 * 16, -1),
                         (8 + 40 * 16, 1035), (8 + 40 * 16, 2**31 - 1),
                         (12 + 40 * 16, -1), (12 + 40 * 16, len(data) + 1),
                         (20 + 40 * 16, 1)]:
        changed = bytearray(good)
        struct.pack_into("<i", changed, field, value)
        invalid.append(changed)
    for at, changed in enumerate(invalid):
        path = root / f"header{at}.reject.bsp"
        path.write_bytes(changed)
        if seed_dir := os.environ.get("SOURCE_BSP_FUZZ_CORPUS"):
            (Path(seed_dir) / f"invalid{at}.bsp").write_bytes(changed)
        handle.value = 99
        C.memset(C.byref(info), 0xff, C.sizeof(info))
        assert reader.index_open_archive(span(str(path).encode()), 1,
                                         C.byref(handle), C.byref(info)) == 8
        assert handle.value == 0 and bytes(info) == bytes(C.sizeof(info))
    empty = root / "no-pack.empty.bsp"
    header[8 + 40 * 16:24 + 40 * 16] = bytes(16)
    empty.write_bytes(header)
    assert reader.index_open_archive(span(str(empty).encode()), 1, C.byref(handle), C.byref(info)) == 9
    assert handle.value == 0 and bytes(info) == bytes(C.sizeof(info))
    # Auto-discovery is explicitly typed, not a filename-extension heuristic.
    valid.write_bytes(data)
    check(reader.index_open_archive(filename, 0, C.byref(handle), C.byref(info)))
    assert (info.offset, info.length) == (0, len(data))
    check(reader.index_destroy(handle))
    assert reader.index_open_archive(filename, 1, C.byref(handle), C.byref(info)) == 8
    valid.write_bytes(b"")
    assert reader.index_open_archive(filename, 0, C.byref(handle), C.byref(info)) == 8
    # Sparse file verifies the bound before allocation or ZIP parsing.
    with valid.open("wb") as output:
        output.truncate(512 * 1024 * 1024 + 1)
    assert reader.index_open_archive(filename, 0, C.byref(handle), C.byref(info)) == 8
    valid.unlink()
    series = root / "mount-series"
    series.mkdir()
    for index in [0, 2, 4]:
        with zipfile.ZipFile(series / f"zip{index}.zip", "w") as archive:
            archive.writestr("priority.txt", str(index).encode())
            archive.writestr(f"only{index}.txt", b"present")
    (series / "zip1.zip").write_bytes(b"bad directory")
    (series / "priority.txt").write_bytes(b"L")
    (series / "baseline").mkdir()
    (series / "baseline" / "priority.txt").write_bytes(b"B")
    candidate_cases(reader, root, series)
    print("Archive discovery: BSP versions/ranges/empty/header failures, typed ZIP open and sparse oversize rejection passed", flush=True)


def candidate_cases(reader, root, series):
    output = C.create_string_buffer(4096)
    target = Slice(C.cast(output, C.c_void_p), len(output))
    written, directory, cursor = C.c_uint64(), C.c_uint32(), C.c_uint64()
    pointers = [C.byref(written), C.byref(directory), C.byref(cursor)]

    def start(path=series, naming=0, language=b"", buffer=target, context=None):
        return reader.find_pack_candidates(reader.handle if context is None else context,
                                           span(os.fsencode(path)), naming, span(language), buffer, *pointers)

    def drain(status):
        result = []
        if status == 9:
            assert cursor.value == 0
            return result
        check(status)
        try:
            while status == 0:
                assert directory.value == 0
                result.append(output.raw[:written.value].decode())
                status = reader.find_next(reader.handle, cursor, target, C.byref(written), C.byref(directory))
            assert status == 9
        finally:
            check(reader.find_close(reader.handle, cursor))
        return result

    expected = [str(series / f"zip{n}.zip") for n in [2, 1, 0]]
    assert drain(start()) == expected  # Malformed zip1 is still present.
    assert start(buffer=Slice(None, 0)) == 4
    assert written.value == len(expected[0].encode()) and cursor.value == 0
    check(start())
    assert reader.find_next(reader.handle, cursor, Slice(None, 0), C.byref(written), C.byref(directory)) == 4
    check(reader.find_next(reader.handle, cursor, target, C.byref(written), C.byref(directory)))
    assert output.raw[:written.value].decode() == expected[1]
    check(reader.find_close(reader.handle, cursor))
    assert reader.find_close(reader.handle, cursor) == 9
    for path, naming, language, buffer, context in [
        (b"", 0, b"", target, None), (b"a\0b", 0, b"", target, None),
        (b"\xff", 0, b"", target, None), (series, 2, b"", target, None),
        (series, 0, b"french", target, None), (series, 1, b"../escape", target, None),
        (series, 0, b"", Slice(None, 1), None), (series, 0, b"", target, 0),
    ]:
        assert start(path, naming, language, buffer, context) in [1, 3]
        assert cursor.value == 0 and written.value == 0
    for missing in range(3):
        args = pointers.copy()
        args[missing] = None
        assert reader.find_pack_candidates(reader.handle, span(os.fsencode(series)), 0,
                                           span(b""), target, *args) == 1
    assert drain(start(root / "absent")) == []

    snapshot = root / "candidate snapshot \u00e7"
    snapshot.mkdir()
    (snapshot / "zip0.zip").write_bytes(b"present")
    (snapshot / "zip1.zip").mkdir()  # Native stat admits directories too.
    (snapshot / "zip2.zip").symlink_to(snapshot / "zip0.zip")
    (snapshot / "zip3.zip").symlink_to(snapshot / "missing")  # Broken => stop.
    (snapshot / "zip4.zip").write_bytes(b"not discovered")
    check(start(snapshot))
    (snapshot / "zip0.zip").unlink()
    assert drain(0) == [str(snapshot / f"zip{n}.zip") for n in [2, 1, 0]]
    assert drain(start(snapshot)) == []  # A new snapshot sees the missing zero.
    xbox = root / "xbox-candidates"
    xbox.mkdir()
    for name in ["zip0.360.zip", "zip1.360.zip", "zip0_french.360.zip", "zip2_french.360.zip"]:
        (xbox / name).write_bytes(b"present")
    assert drain(start(xbox, 1, b"french")) == [str(xbox / name) for name in
        ["zip0_french.360.zip", "zip1.360.zip", "zip0.360.zip"]]
    assert drain(start(xbox, 1)) == [str(xbox / name) for name in ["zip1.360.zip", "zip0.360.zip"]]
    numeric = root / "mount-series-numeric"
    numeric.mkdir()
    for n in list(range(11)) + [12]:
        with zipfile.ZipFile(numeric / f"zip{n}.zip", "w") as archive:
            archive.writestr("priority.txt", b"X" if n == 10 else b"0")
            archive.writestr(f"only{n}.txt", b"present")
    assert drain(start(numeric)) == [str(numeric / f"zip{n}.zip") for n in reversed(range(11))]
    for _ in range(1000):
        check(start())
        check(reader.find_close(reader.handle, cursor))
    print("Numbered ZIP candidates: numeric/localized precedence, first gap, malformed/directory/symlink paths, snapshot mutation, invalid/short/stale cursors and 1000 early closes passed", flush=True)


def file_index_cases(reader, root, data):
    path = root / "index space \u00e7.zip"
    path.write_bytes(b"prefix" + data + b"suffix")
    filename = span(str(path).encode())
    for argument, offset, length, expected in [
        (filename, 6, len(data) + 7, 7),
        (filename, 2**64 - 1, 2, 7),
        (filename, 2**64 - 1, 0, 7),
        (filename, 0, len(data), 8),
        (filename, 6, len(data) - 1, 8),
        (filename, 0, 512 * 1024 * 1024 + 1, 8),
        (span(str(root / "missing.zip").encode()), 0, len(data), 7),
        (span(str(root).encode()), 0, len(data), 1),
        (span(b""), 0, len(data), 1),
        (span(b"bad\0path"), 0, len(data), 1),
        (span(b"\xff"), 0, len(data), 1),
        (Slice(None, 1), 0, len(data), 1),
    ]:
        handle, count = C.c_uint64(99), C.c_uint32(99)
        status = reader.index_open_file(argument, offset, length,
                                        C.byref(handle), C.byref(count))
        assert status == expected, (offset, length, status, expected)
        assert handle.value == count.value == 0
    handle, count = C.c_uint64(), C.c_uint32()
    # Empty BSP pack lumps are valid empty indexes, as in the byte-slice API.
    check(reader.index_open_file(filename, 0, 0, C.byref(handle), C.byref(count)))
    assert handle.value != 0 and count.value == 0
    check(reader.index_destroy(handle))
    assert reader.index_open_file(filename, 6, len(data), None, C.byref(count)) == 1
    assert reader.index_open_file(filename, 6, len(data), C.byref(handle), None) == 1
    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(lambda _: reader.compare_index(data, path, 6), range(100)))
    # Rust retains the open file descriptor, including across path replacement.
    # Opened payloads are independently owned and survive index destruction.
    check(reader.index_open_file(filename, 6, len(data), C.byref(handle), C.byref(count)))
    path.unlink()
    path.write_bytes(b"replacement is not the mounted archive")
    entry = PakEntry()
    check(reader.index_find(handle, span(b"a.txt"), C.byref(entry)))
    assert entry.length == 7
    file, size, absolute = C.c_uint64(), C.c_uint64(), C.c_uint64()
    check(reader.file_open_pak(reader.handle, handle, 0, C.byref(file), C.byref(size), C.byref(absolute)))
    check(reader.index_destroy(handle))
    reader.read_opened(file, size, "unmounted", b"payload")
    path.unlink()
    print("ZIP file-index IO: range/UTF-8/path/error checks, concurrent opens and retained descriptor passed", flush=True)
    payload_cases(reader, root, data)


def payload_cases(reader, root, data):
    path = root / "payload-source.tmp"
    filename = span(str(path).encode())
    path.write_bytes(data)
    index, count = C.c_uint64(), C.c_uint32()
    check(reader.index_open_file(filename, 0, len(data), C.byref(index), C.byref(count)))
    file, size, absolute = C.c_uint64(), C.c_uint64(), C.c_uint64()

    def rejected(context, handle, at, status):
        file.value = size.value = absolute.value = 99
        assert reader.file_open_pak(context, handle, at, C.byref(file), C.byref(size), C.byref(absolute)) == status
        assert file.value == size.value == absolute.value == 0

    rejected(0, index, 0, 3)
    rejected(reader.handle, 0, 0, 3)
    rejected(reader.handle, index, count, 9)
    for pointers in [(None, C.byref(size), C.byref(absolute)),
                     (C.byref(file), None, C.byref(absolute)),
                     (C.byref(file), C.byref(size), None)]:
        assert reader.file_open_pak(reader.handle, index, 0, *pointers) == 1
    raw = C.c_uint64()
    check(reader.index_create(span(data), C.byref(raw), C.byref(count)))
    rejected(reader.handle, raw, 0, 1)
    check(reader.index_destroy(raw))

    def concurrent_read(_):
        opened, length, position = C.c_uint64(), C.c_uint64(), C.c_uint64()
        check(reader.file_open_pak(reader.handle, index, 0, C.byref(opened), C.byref(length), C.byref(position)))
        reader.read_opened(opened, length, "concurrent", b"payload")

    with ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(concurrent_read, range(1000)))
    entry = PakEntry()
    check(reader.index_find(index, span(b"a.txt"), C.byref(entry)))
    # In-place changes are not a snapshot: reject changed CRC and truncated IO.
    with path.open("r+b") as output:
        output.seek(entry.offset)
        output.write(b"!")
    rejected(reader.handle, index, 0, 8)
    with path.open("r+b") as output:
        output.truncate(entry.offset + 1)
    rejected(reader.handle, index, 0, 7)
    check(reader.index_destroy(index))
    rejected(reader.handle, index, 0, 3)
    path.unlink()

    # Valid metadata, corrupt payload: also consumed by the actual native FS gate.
    for method in [zipfile.ZIP_STORED, zipfile.ZIP_LZMA]:
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w", compression=method) as archive:
            archive.writestr("bad.txt", b"not valid after payload mutation" * 10)
        bad = bytearray(stream.getvalue())
        # Corrupt stored data or LZMA property-size header, not ignored version.
        bad[37 if method == zipfile.ZIP_STORED else 39] ^= 0xff
        path = root / f"corrupt{method}.badzip"
        path.write_bytes(bad)
        check(reader.index_open_file(span(str(path).encode()), 0, len(bad), C.byref(index), C.byref(count)))
        rejected(reader.handle, index, 0, 8)
        check(reader.index_destroy(index))
    print("ZIP payload ownership: 1000 concurrent reads, stale/null/unbound indexes, CRC/truncation failures and unmount survival passed", flush=True)


def corpus(reader, root):
    maps = entries = empty = rejected = 0
    for game in ["hl2", "episodic", "ep2"]:
        for path in sorted((root / game / "maps").glob("*.bsp")):
            with path.open("rb") as file:
                header = file.read(1036)
                if len(header) < 1036:
                    print(f"not an auditable BSP (short header): {path}", flush=True)
                    rejected += 1
                    continue
                offset, length = struct.unpack_from("<ii", header, 8 + 40 * 16)
                assert offset >= 0 and length >= 0
                file.seek(offset)
                data = file.read(length)
            if length == 0:
                empty += 1
                continue
            if not zipfile.is_zipfile(io.BytesIO(data)):
                assert reader.mount(path, offset, length) != 0
                print(f"non-ZIP/truncated lump rejected: {path}", flush=True)
                rejected += 1
                continue
            entries += reader.compare(path, offset, data)
            maps += 1
    assert maps and entries, "no installed map archives audited"
    print(f"Installed BSP differential: {maps} ZIPs, {entries} entries identical; "
          f"{empty} empty lumps, {rejected} non-auditable maps", flush=True)


def native_lzma(reader, root, library):
    """Native SDK used by Source: known output size, no EOS marker (ZIP flags 0)."""
    sdk = C.CDLL(str(Path(library).resolve()))
    encode = sdk.LzmaCompress
    encode.restype = C.c_int
    encode.argtypes = [C.c_void_p, C.POINTER(C.c_size_t), C.c_void_p, C.c_size_t,
                       C.c_void_p, C.POINTER(C.c_size_t), C.c_int, C.c_uint,
                       C.c_int, C.c_int, C.c_int, C.c_int, C.c_int]
    rng = random.Random(14)
    count = 0
    for length in [0, 1, 2, 3, 4, 15, 16, 255, 256, 4096, 65536]:
        for payload in [rng.randbytes(length), b"x" * length]:
            for level in [0, 5]:
                output = C.create_string_buffer(length * 2 + 1024)
                size = C.c_size_t(len(output))
                properties = C.create_string_buffer(5)
                property_size = C.c_size_t(5)
                source = C.create_string_buffer(payload)
                check(encode(output, C.byref(size), source, length, properties,
                             C.byref(property_size), level, 1 << 16, 3, 0, 2, 32, 1))
                compressed = bytes([9, 4, 5, 0]) + properties.raw + output.raw[:size.value]
                stream = io.BytesIO()
                with zipfile.ZipFile(stream, "w") as archive:
                    archive.writestr("a.vmt", payload)
                original = stream.getvalue()
                old_cd = 35 + length
                data = bytearray(original[:35] + compressed + original[old_cd:])
                cd = 35 + len(compressed)
                struct.pack_into("<H", data, 8, 14)
                struct.pack_into("<I", data, 18, len(compressed))
                struct.pack_into("<H", data, cd + 10, 14)
                struct.pack_into("<I", data, cd + 20, len(compressed))
                struct.pack_into("<I", data, len(data) - 6, cd)
                path = root / f"native{count}.zip"
                path.write_bytes(data)
                count += reader.compare(path, 0, data)
                if seed_dir := os.environ.get("SOURCE_PAK_FUZZ_CORPUS"):
                    (Path(seed_dir) / f"native{count}.zip").write_bytes(data)
    print(f"Native LZMA differential: {count} size-terminated entries agree with SDK/Python/Rust", flush=True)


def main():
    reader = Reader(sys.argv[1])
    try:
        with tempfile.TemporaryDirectory(prefix="source-pak-abi-") as temporary:
            synthetic(reader, Path(temporary))
            if sdk := os.environ.get("SOURCE_PAK_NATIVE_LZMA"):
                native_lzma(reader, Path(temporary), sdk)
            if native := os.environ.get("SOURCE_PAK_NATIVE_FS"):
                subprocess.run([native, os.environ["SOURCE_PAK_NATIVE_FS_LIBRARY"],
                                *map(str, sorted(Path(temporary).glob("*.zip"))),
                                *map(str, sorted(Path(temporary).glob("*.badzip"))),
                                *map(str, sorted(Path(temporary).glob("*.bsp"))),
                                str(Path(temporary) / "mount-series"),
                                str(Path(temporary) / "mount-series-numeric"),
                                str(Path(temporary) / "map-transitions"),
                                str(Path(temporary) / "shared-owner"),
                                str(Path(temporary) / "path-selection")], check=True)
        if len(sys.argv) > 2:
            corpus(reader, Path(sys.argv[2]))
    finally:
        check(reader.destroy(reader.handle))


if __name__ == "__main__":
    main()
