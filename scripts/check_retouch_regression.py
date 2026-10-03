"""Check a paired candidate against the previous single-photo synthetic benchmark."""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageOps

from train_paired_retouch import ort_session, sha256
from train_personalized_retouch import face_crop, load_face, make_masks, synthesize


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("image", type=Path)
    parser.add_argument("landmarks", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    landmarks, bounds = load_face(args.landmarks)
    image = ImageOps.exif_transpose(Image.open(args.image)).convert("RGB")
    clean, points = face_crop(image, landmarks, bounds)
    skin, under = make_masks(points)
    sessions = [ort_session(path) for path in (args.baseline, args.candidate)]
    def predict(session, source):
        x = np.ascontiguousarray(source.transpose(2, 0, 1)[None] * 2 - 1)
        mask = session.run(None, {session.get_inputs()[0].name: x})[0][0].transpose(1, 2, 0)
        return (1 - 2 * mask) * source ** 2 + 2 * mask * source
    old_clean, new_clean = [predict(session, clean) for session in sessions]
    repairs = [[], []]
    for seed in range(8119, 8135):
        source, damage, target, _ = synthesize(clean, skin, under, seed)
        for i, session in enumerate(sessions):
            predicted = predict(session, source)
            error = float((np.abs(predicted - target) * damage[..., None]).sum() / (damage.sum() * 3 + 1e-6))
            repairs[i].append(error)
    baseline, candidate = [float(np.mean(values)) for values in repairs]
    clean_shift = float(np.abs(old_clean - new_clean).mean())
    report = {"scope": "previous portrait's synthetic defects; preservation regression, not a new general benchmark",
              "baseline_sha256": sha256(args.baseline), "candidate_sha256": sha256(args.candidate),
              "samples": 16, "previous_repair_mae": baseline, "candidate_repair_mae": candidate,
              "relative_repair_improvement": 1 - candidate / baseline, "clean_output_shift_mae": clean_shift,
              "gates": {"repair_regression_below_3_percent": candidate <= baseline * 1.03,
                        "clean_output_shift_below_0_005": clean_shift < .005}}
    report["passed"] = all(report["gates"].values())
    args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
    if not report["passed"]:
        raise AssertionError("Prior portrait regression exceeds limits")


if __name__ == "__main__":
    main()
