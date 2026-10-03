"""Compare registered maps and native exported pixels without a full-image delta allocation."""
import argparse
import importlib.util
import json
from pathlib import Path
import numpy as np
from PIL import Image


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("cpu", type=Path)
    parser.add_argument("gpu", type=Path)
    parser.add_argument("profiles", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("backend_comparison", Path(__file__).with_name("compare-ai-backends.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    cpu, gpu = module.read_maps(args.cpu / "maps.bin"), module.read_maps(args.gpu / "maps.bin")
    report = {"maps": {}, "images": {}, "gpu_profiles": {}}
    for name, reference in cpu.items():
        candidate = gpu[name]
        assert reference.shape == candidate.shape, name
        delta = np.abs(reference.astype(np.float64) - candidate)
        assert np.isfinite(delta).all(), name
        report["maps"][name] = {"mean_absolute_difference": float(delta.mean()), "maximum_absolute_difference": float(delta.max())}
    for filename in ["auto-overview.png", "auto-native.png"]:
        with Image.open(args.cpu / filename) as a, Image.open(args.gpu / filename) as b:
            assert a.size == b.size, filename
            changed, total, maximum, count = 0, 0, 0, 0
            for top in range(0, a.height, 256):
                box = (0, top, a.width, min(top + 256, a.height))
                delta = np.abs(np.asarray(a.crop(box).convert("RGBA"), dtype=np.int16) - np.asarray(b.crop(box).convert("RGBA"), dtype=np.int16))
                changed += int(np.any(delta != 0, axis=2).sum())
                total += int(delta.sum())
                maximum = max(maximum, int(delta.max()))
                count += delta.size
            report["images"][filename] = {"dimensions": list(a.size), "changed_pixels": changed, "mean_absolute_byte_difference": total / count, "maximum_byte_difference": maximum}
            assert maximum <= 2 and total / count < 0.05, f"{filename} differs beyond export tolerance"
    for profile in sorted(args.profiles.glob("*.json")):
        providers = {}
        for event in json.loads(profile.read_text(encoding="utf-8")):
            provider = event.get("args", {}).get("provider")
            if provider:
                providers[provider] = providers.get(provider, 0) + 1
        report["gpu_profiles"][profile.name] = providers
    assert len(report["gpu_profiles"]) >= 5, "Missing model profiles"
    assert all(p.get("DmlExecutionProvider", 0) > 0 for p in report["gpu_profiles"].values()), "A model has no GPU kernel proof"
    report["equivalent_at_export_precision"] = True
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
