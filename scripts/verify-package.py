"""Verify curated Windows payload files and PE imports before compiling the installer."""
import json
import struct
import sys
from pathlib import Path


def pe(path):
    data = path.read_bytes()
    offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[offset:offset + 4] != b"PE\0\0":
        raise ValueError(f"Not a PE binary: {path}")
    machine, count, _, _, _, optional_size, _ = struct.unpack_from("<HHIIIHH", data, offset + 4)
    optional = offset + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    directory = optional + (112 if magic == 0x20B else 96)
    sections = []
    for index in range(count):
        section = optional + optional_size + index * 40
        virtual_size, rva, size, raw = struct.unpack_from("<IIII", data, section + 8)
        sections.append((rva, max(size, virtual_size), raw))

    def address(rva):
        for start, size, raw in sections:
            if start <= rva < start + size:
                return raw + rva - start
        raise ValueError(f"Invalid PE RVA: {path}")

    imports = []
    import_rva = struct.unpack_from("<I", data, directory + 8)[0]
    if import_rva:
        descriptor = address(import_rva)
        while any(data[descriptor:descriptor + 20]):
            name_rva = struct.unpack_from("<I", data, descriptor + 12)[0]
            name_offset = address(name_rva)
            imports.append(data[name_offset:data.index(0, name_offset)].decode("ascii").lower())
            descriptor += 20
    return machine, imports


def verify(payload):
    binaries = sorted([*payload.rglob("*.exe"), *payload.rglob("*.dll")])
    records = []
    for path in binaries:
        machine, imports = pe(path)
        if machine != 0x8664:
            raise SystemExit(f"Expected x64 binary: {path}")
        for name in imports:
            # These are not Windows OS files. Never accept a developer machine's
            # globally installed VC runtime as proof that the payload is complete.
            if name.startswith(("msvcp", "vcruntime", "concrt", "vccorlib")):
                if not (payload / name).is_file():
                    raise SystemExit(f"Missing application-local runtime {name} required by {path}")
            if name in ("directml.dll", "onnxruntime_providers_shared.dll"):
                if not (path.parent / name).is_file():
                    raise SystemExit(f"Missing runtime peer {name} required by {path}")
        records.append({"path": path.relative_to(payload).as_posix(), "machine": "x64", "imports": imports})
    forbidden = {".jpg", ".jpeg", ".png", ".ron", ".pyc", ".pt", ".pth", ".nupkg", ".zip", ".rs", ".py", ".ps1"}
    for path in payload.rglob("*"):
        if path.is_file() and path.suffix.lower() in forbidden:
            raise SystemExit(f"Unexpected photo, workspace or developer file in payload: {path}")
    (payload / "binary-dependencies.json").write_text(json.dumps(records, indent=2) + "\n", encoding="utf-8")
    print(f"Verified {len(binaries)} x64 binaries and their application-local VC dependencies; no loose photos or workspaces.")


if __name__ == "__main__":
    verify(Path(sys.argv[1]))
