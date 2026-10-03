"""Verify that a built Windows executable embeds every Hastur ICO payload.

Usage: python assets/branding/verify_pe_icon.py target/release/hastur-retouch.exe
Reads PE resources through Windows without executing the application.
"""

import ctypes
from pathlib import Path
import struct
import sys


def main():
    if sys.platform != "win32":
        raise SystemExit("PE resource verification uses the Windows loader.")
    binary = Path(sys.argv[1]).resolve(strict=True)
    ico_path = Path(__file__).resolve().parent / "hastur.ico"
    ico = ico_path.read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", ico)
    assert (reserved, kind) == (0, 1)
    source = {}
    for index in range(count):
        width, height, _, _, _, _, length, offset = struct.unpack_from("<BBBBHHII", ico, 6 + 16 * index)
        source[(width or 256, height or 256)] = ico[offset:offset + length]

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint32
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    module = kernel.LoadLibraryExW(str(binary), None, 0x2 | 0x20)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())

    def resource(name, kind):
        found = kernel.FindResourceW(module, name, kind)
        if not found:
            raise ctypes.WinError(ctypes.get_last_error())
        handle = kernel.LoadResource(module, found)
        data = kernel.LockResource(handle)
        return ctypes.string_at(data, kernel.SizeofResource(module, found))

    try:
        group = resource(1, 14)
        reserved, kind, embedded_count = struct.unpack_from("<HHH", group)
        assert (reserved, kind, embedded_count) == (0, 1, count)
        verified = []
        for index in range(embedded_count):
            width, height, _, _, _, _, length, resource_id = struct.unpack_from("<BBBBHHIH", group, 6 + 14 * index)
            size = (width or 256, height or 256)
            image = resource(resource_id, 3)
            assert len(image) == length
            assert image == source[size], f"Icon payload differs at {size}"
            verified.append(size)
        assert set(verified) == set(source)
        print(f"{binary.name}: all {count} embedded icon frames match hastur.ico byte for byte.")
        print("Sizes:", ", ".join(f"{width}x{height}" for width, height in sorted(verified)))
    finally:
        kernel.FreeLibrary(module)


if __name__ == "__main__":
    main()
