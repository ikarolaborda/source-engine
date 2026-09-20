#!/usr/bin/env python3
"""Tiny on-device regression for partial writes inside a full lightmap lock.

No game/window is launched. Usage: metal_texture_lock.py libsource_abi.dylib
"""
import ctypes as c
import sys


def main():
    lib = c.CDLL(sys.argv[1])
    handle, u32 = c.c_uint64, c.c_uint32

    def api(name, result, *args):
        fn = getattr(lib, "source_d3d9_" + name)
        fn.restype, fn.argtypes = result, args
        return fn

    create = api("device_create", handle, u32, u32)
    destroy = api("device_destroy", None, handle)
    texture_create = api("texture_create", handle, handle, *([u32] * 7), c.c_char_p)
    texture_destroy = api("texture_destroy", None, handle, handle)
    lock = api("texture_lock", c.c_int32, handle, handle, u32, u32, c.c_void_p,
               u32, u32, c.c_int32, c.POINTER(c.c_void_p),
               c.POINTER(c.c_int32), c.POINTER(c.c_int32))
    unlock = api("texture_unlock", None, handle, handle, u32, u32)
    device = create(4, 4)
    assert device, "Metal device unavailable"
    try:
        # D3D formats used for LDR, integer HDR and floating-point HDR lightmaps.
        for fmt, pixel_bytes in ((3, 4), (19, 8), (18, 8)):
            texture = texture_create(device, 0, 4, 4, 1, 1, 0, fmt, b"lock regression")
            assert texture, f"texture format {fmt} unavailable"
            try:
                def locked(readonly=False):
                    bits, pitch, slice_pitch = c.c_void_p(), c.c_int32(), c.c_int32()
                    assert lock(device, texture, 0, 0, None, 0, 0, int(readonly),
                                c.byref(bits), c.byref(pitch), c.byref(slice_pitch))
                    assert pitch.value == 4 * pixel_bytes
                    return bits.value, slice_pitch.value

                bits, size = locked()
                original = bytes((i % 127) + 1 for i in range(size))
                c.memmove(bits, original, size)
                unlock(device, texture, 0, 0)

                # Engine relocks a whole atlas but updates only one texel.
                bits, size = locked()
                c.memmove(bits + 5 * pixel_bytes, b"\x33" * pixel_bytes, pixel_bytes)
                unlock(device, texture, 0, 0)
                bits, size = locked(True)
                expected = bytearray(original)
                expected[5 * pixel_bytes:6 * pixel_bytes] = b"\x33" * pixel_bytes
                actual = c.string_at(bits, size)
                unlock(device, texture, 0, 0)
                assert actual == expected, f"format {fmt}: untouched lightmap texels erased"
                print(f"format {fmt}: full-page relock preserves untouched texels")
            finally:
                texture_destroy(device, texture)
    finally:
        destroy(device)


if __name__ == "__main__":
    main()
