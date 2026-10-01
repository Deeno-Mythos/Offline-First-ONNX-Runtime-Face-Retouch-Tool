"""Read isolated runtime_validation outputs; compare maps and actual rendered pixels."""
import argparse
import json
from pathlib import Path
import struct
import numpy as np
from PIL import Image


def read_maps(path):
    content = path.read_bytes()
    offset = 0
    result = {}
    for name in ("skin", "under_eyes", "eyes", "blemish", "neural_blend", "repair_delta"):
        length = struct.unpack_from("<Q", content, offset)[0]
        offset += 8
        result[name] = np.frombuffer(content, dtype="<f4", count=length, offset=offset)
        offset += length * 4
    assert offset == len(content), "Unexpected trailing map data"
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("cpu", type=Path)
    parser.add_argument("burn", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    cpu = read_maps(args.cpu / "maps.bin")
    burn = read_maps(args.burn / "maps.bin")
    report = {"maps": {}}
    for name in cpu:
        a, b = cpu[name], burn[name]
        assert a.shape == b.shape, f"Different registered map dimensions: {name}"
        delta = np.abs(a.astype(np.float64) - b)
        report["maps"][name] = {
            "values": int(delta.size),
            "mean_absolute_difference": float(delta.mean()),
            "maximum_absolute_difference": float(delta.max()),
        }
        assert np.isfinite(delta).all(), f"Nonfinite values: {name}"
    a = np.asarray(Image.open(args.cpu / "auto-overview.png").convert("RGBA"), dtype=np.int16)
    b = np.asarray(Image.open(args.burn / "auto-overview.png").convert("RGBA"), dtype=np.int16)
    assert a.shape == b.shape, "Different rendered overview dimensions"
    delta = np.abs(a - b)
    report["render"] = {
        "dimensions": [int(a.shape[1]), int(a.shape[0])],
        "changed_pixels": int(np.any(delta != 0, axis=2).sum()),
        "mean_absolute_byte_difference": float(delta.mean()),
        "maximum_byte_difference": int(delta.max()),
    }
    # Actual quantized output comparison, rather than a claim based only on model weights.
    report["equivalent_at_export_precision"] = bool(delta.max() <= 2 and delta.mean() < 0.05)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    assert report["equivalent_at_export_precision"], "Backend output differences exceed validation tolerance"


if __name__ == "__main__":
    main()
