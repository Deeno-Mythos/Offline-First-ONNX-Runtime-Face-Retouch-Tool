"""Local, subject-disjoint fine-tuning of the existing ONNX skin blend model.

Source JPEGs are read only. Face metadata/sRGB previews come from training_faces,
using the app's EXIF/ICC importer and face detector. Edited crop/shape/color changes
are registered and removed from the learning target. Candidates remain separate
unless explicit held-out quality and ONNX equivalence checks pass.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import shutil
import tempfile
import time
from pathlib import Path

import cv2
import numpy as np
import onnx
import onnxruntime as ort
import torch
import torch.nn.functional as F
from PIL import Image, ImageDraw

from train_personalized_retouch import UNet, as_tensor, blend, features, load_face, make_masks

ROOT = Path(__file__).resolve().parent.parent
SIZE = 512


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, ensure_ascii=False), encoding="utf-8")


def copy_atomic(source: Path, destination: Path):
    with tempfile.NamedTemporaryFile(dir=destination.parent, suffix=".training-tmp", delete=False) as stream:
        temporary = Path(stream.name)
    try:
        shutil.copyfile(source, temporary)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


def promote(output: Path, active: Path, baseline_checkpoint: Path):
    report = json.loads((output / "metrics.json").read_text(encoding="utf-8"))
    candidate = output / "retouch_generator_candidate.onnx"
    checkpoint = output / "retouch_generator_candidate.pt"
    if not report["accepted_for_runtime"] or not report["gates"] or not all(report["gates"].values()):
        raise ValueError("Candidate failed quality gates; active models will not be changed")
    if sha256(candidate) != report["candidate_sha256"]:
        raise ValueError("Candidate changed after validation")
    if sha256(active) != report["active_onnx_sha256"]:
        raise ValueError("Active model changed since baseline evaluation")
    if sha256(baseline_checkpoint) != report["checkpoint_sha256"]:
        raise ValueError("Baseline checkpoint changed since training")
    preservation_path = output / "prior-portrait-regression.json"
    if preservation_path.exists():
        preservation = json.loads(preservation_path.read_text(encoding="utf-8"))
        if (not preservation["passed"] or preservation.get("candidate_sha256") != sha256(candidate)
                or preservation.get("baseline_sha256") != report["active_onnx_sha256"]):
            raise ValueError("Prior portrait preservation failed or refers to a different candidate")
        report["gates"]["prior_portrait_preservation"] = True
    onnx.checker.check_model(onnx.load(candidate))
    backup = output / "prior-active"
    backup.mkdir(exist_ok=True)
    if (backup / active.name).exists():
        if sha256(backup / active.name) != report["active_onnx_sha256"]:
            raise ValueError("Backup directory contains a different model; refusing to replace it")
    else:
        shutil.copy2(active, backup / active.name)
    for source in (active.parent / "burn/retouch_generator.bpk", active.parent / "retouch-training.json"):
        if source.exists() and not (backup / source.name).exists():
            shutil.copy2(source, backup / source.name)
    if not (backup / "retouch_generator.pt").exists():
        shutil.copy2(baseline_checkpoint, backup / "retouch_generator.pt")
    metadata = {"kind": "local-paired-skin-head", "accepted_for_runtime": True,
                "onnx_sha256": sha256(candidate), "checkpoint_sha256": sha256(checkpoint),
                "baseline_sha256": report["active_onnx_sha256"], "split_counts": report["split_counts"],
                "seed": report["seed"], "selected_step": report["selected_step"],
                "adaptation_weight": report["adaptation_weight"],
                "validation": {n: report["validation"][n] for n in ("before", "after", "identity")},
                "test": {n: report["test"][n] for n in ("before", "after", "identity")},
                "scope": report["scope"], "gates": report["gates"]}
    copy_atomic(checkpoint, active.with_suffix(".pt"))
    copy_atomic(candidate, active)
    write_json(active.parent / "retouch-training.json", metadata)
    print(f"Installed validated skin model: {active}. Previous model retained in {backup}.")
    print("Rebuild with Cargo to regenerate its Burn pack, then verify ONNX/Burn actual outputs.")


def inventory(root: Path, output: Path, seed: int):
    pairs = []
    for folder in sorted(p for p in root.iterdir() if p.is_dir()):
        match = re.match(r"JK\s*-\s*(\d+)\s*-", folder.name)
        if not match:
            raise ValueError(f"Folder lacks a stable subject ID: {folder}")
        originals = sorted(p for p in folder.iterdir() if p.suffix.lower() in (".jpg", ".jpeg"))
        edits = sorted(p for p in (folder / "EDITED").iterdir() if p.suffix.lower() in (".jpg", ".jpeg"))
        if len(originals) != 1 or len(edits) != 1:
            raise ValueError(f"Ambiguous pairing for subject {match[1]}; expected one original/edited JPEG")
        original, edited = originals[0], edits[0]
        if not edited.stem.casefold().startswith(original.stem.casefold() + "-"):
            raise ValueError(f"Filenames do not match for subject {match[1]}")
        pairs.append({"subject": match[1], "original": str(original), "edited": str(edited),
                      "original_sha256": sha256(original), "edited_sha256": sha256(edited)})
    if len(pairs) < 18:
        raise ValueError("At least 18 distinct paired subjects are required for train/validation/test")
    if len({p["original_sha256"] for p in pairs}) != len(pairs):
        raise ValueError("Duplicate source images would leak between subject splits")
    rng = np.random.default_rng(seed)
    order = rng.permutation(len(pairs))
    holdout = max(3, round(len(pairs) * 0.15))
    for i, index in enumerate(order):
        pairs[index]["split"] = "test" if i < holdout else "validation" if i < holdout * 2 else "train"
    write_json(output / "manifest.json", {"seed": seed, "source_root": str(root.resolve()),
                                          "pairing": "same subject folder and exact original filename prefix", "pairs": pairs})
    return pairs


def crop_face(image: np.ndarray, points: np.ndarray, bounds):
    height, width = image.shape[:2]
    side = max(bounds[2] * width, bounds[3] * height) * 1.5
    cx, cy = (bounds[0] + bounds[2] / 2) * width, (bounds[1] + bounds[3] / 2) * height
    left, top = max(cx - side / 2, 0), max(cy - side / 2, 0)
    right, bottom = min(cx + side / 2, width), min(cy + side / 2, height)
    # Match portrait::blob's source-pixel grid, including its half-pixel convention.
    yy, xx = np.mgrid[:SIZE, :SIZE].astype(np.float32)
    sx, sy = left + xx * (right - left) / SIZE, top + yy * (bottom - top) / SIZE
    crop = cv2.remap(image, sx, sy, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    pts = points.copy()
    pts[:, 0] = (points[:, 0] * width - left) * SIZE / (right - left)
    pts[:, 1] = (points[:, 1] * height - top) * SIZE / (bottom - top)
    return crop, pts


def align_pair(source, edited, source_points, edited_points):
    anchors = [33, 133, 362, 263, 1, 4, 6, 168, 61, 291]
    matrix, inliers = cv2.estimateAffinePartial2D(edited_points[anchors, :2], source_points[anchors, :2],
                                                method=cv2.RANSAC, ransacReprojThreshold=4)
    if matrix is None or int(inliers.sum()) < 6:
        raise ValueError("Insufficient same-face registration anchors")
    aligned = cv2.warpAffine(edited, matrix, (SIZE, SIZE), flags=cv2.INTER_LINEAR,
                             borderMode=cv2.BORDER_REPLICATE)
    gray_source = cv2.cvtColor(np.uint8(source * 255), cv2.COLOR_RGB2GRAY)
    gray_edit = cv2.cvtColor(np.uint8(aligned * 255), cv2.COLOR_RGB2GRAY)
    # Forward/backward optical flow removes local edited reshaping from the target.
    kwargs = dict(pyr_scale=0.5, levels=4, winsize=31, iterations=5, poly_n=7, poly_sigma=1.5, flags=0)
    flow = cv2.calcOpticalFlowFarneback(gray_source, gray_edit, None, **kwargs)
    back = cv2.calcOpticalFlowFarneback(gray_edit, gray_source, None, **kwargs)
    yy, xx = np.mgrid[:SIZE, :SIZE].astype(np.float32)
    mx, my = xx + flow[..., 0], yy + flow[..., 1]
    registered = cv2.remap(aligned, mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    back_at = cv2.remap(back, mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT)
    consistency = np.linalg.norm(flow + back_at, axis=-1)
    confidence = np.clip(1 - consistency / 2, 0, 1)
    confidence *= (mx >= 2) & (my >= 2) & (mx < SIZE - 2) & (my < SIZE - 2)
    confidence *= np.linalg.norm(flow, axis=-1) < 18
    corr = float(np.corrcoef(gray_source.ravel(), cv2.cvtColor(np.uint8(registered * 255), cv2.COLOR_RGB2GRAY).ravel())[0, 1])
    return registered, confidence.astype(np.float32), {"registration_correlation": corr,
                                                       "flow_median_pixels": float(np.median(np.linalg.norm(flow, axis=-1))),
                                                       "consistent_fraction": float((confidence > 0.75).mean())}


def normalize_target(source, edited, skin, confidence):
    # Robust monotone global RGB tone fits: no learned global contrast/color grade.
    allowed = (confidence > 0.75) & (skin > 0.7)
    if int(allowed.sum()) < 3000:
        raise ValueError("Too little aligned facial skin")
    corrected = edited.copy()
    for channel in range(3):
        x, y = edited[..., channel][allowed][::5], source[..., channel][allowed][::5]
        design = np.stack((np.ones_like(x), x, x * x), axis=1)
        weights = np.ones_like(x)
        coef = np.array([0, 1, 0], dtype=np.float32)
        for _ in range(8):
            weighted = design * np.sqrt(weights)[:, None]
            coef = np.linalg.lstsq(weighted, y * np.sqrt(weights), rcond=None)[0]
            residual = np.abs(design @ coef - y)
            weights = np.clip(0.012 / np.maximum(residual, 0.001), 0.03, 1)
        ramp = np.linspace(float(x.min()), float(x.max()), 100)
        if np.min(coef[1] + 2 * coef[2] * ramp) < 0.1:
            raise ValueError("Non-monotone color correction")
        z = edited[..., channel]
        corrected[..., channel] = np.clip(coef[0] + coef[1] * z + coef[2] * z * z, 0, 1)
    delta = cv2.GaussianBlur(corrected - source, (0, 0), 0.65)
    # Remove broad light/color changes, retaining under-eye, wrinkle and spot edits.
    mask = skin * confidence
    low_weight = cv2.GaussianBlur(mask, (0, 0), 28)
    for c in range(3):
        broad = cv2.GaussianBlur(delta[..., c] * mask, (0, 0), 28) / np.maximum(low_weight, 0.01)
        delta[..., c] -= broad
    reliability = np.clip(1 - np.maximum(np.max(np.abs(delta), axis=-1) - 0.045, 0) / 0.06, 0, 1)
    target = np.clip(source + np.clip(delta, -0.06, 0.06) * skin[..., None], 0, 1)
    weight = mask * reliability
    return target.astype(np.float32), weight.astype(np.float32)


def prepare(pairs, output):
    prepared = output / "prepared"
    prepared.mkdir(exist_ok=True)
    for pair in pairs:
        sid = pair["subject"]
        arrays = []
        for label in ("original", "edited"):
            stem = output / "faces" / f"subject-{sid}-{label}"
            points, bounds = load_face(stem.with_suffix(".ron"))
            image = np.asarray(Image.open(stem.with_suffix(".png")).convert("RGB"), dtype=np.float32) / 255
            arrays.append(crop_face(image, points, bounds))
        (source, points), (edited, edited_points) = arrays
        skin, under = make_masks(points)
        stem = output / "faces" / f"subject-{sid}-original"
        original_points, original_bounds = load_face(stem.with_suffix(".ron"))
        app_mask = np.asarray(Image.open(stem.parent / f"{stem.name}-skin.png"), dtype=np.float32) / 255
        app_skin, _ = crop_face(app_mask, original_points, original_bounds)
        skin *= app_skin
        # Erode face boundary and expand eye/brow/lip holes before learning.
        skin = cv2.GaussianBlur(cv2.erode(np.uint8(skin * 255), np.ones((9, 9), np.uint8)), (0, 0), 2).astype(np.float32) / 255
        registered, confidence, quality = align_pair(source, edited, points, edited_points)
        if quality["registration_correlation"] < 0.85:
            raise ValueError(f"Subject {sid}: registration rejected: {quality}")
        target, weight = normalize_target(source, registered, skin, confidence)
        if float((weight > 0.5).sum()) < 3000:
            raise ValueError(f"Subject {sid}: insufficient reliable target pixels")
        np.savez_compressed(prepared / f"{sid}.npz", source=source, target=target, skin=skin,
                            under=under * skin, weight=weight)
        pair["preparation"] = quality | {"reliable_skin_pixels": int((weight > 0.5).sum()),
                                         "local_target_mae": float((np.abs(target - source) * weight[..., None]).sum() / (weight.sum() * 3))}
        print(f"prepare {sid}: correlation {quality['registration_correlation']:.3f}, skin pixels {pair['preparation']['reliable_skin_pixels']}", flush=True)
        row = np.concatenate((source, registered, target, np.repeat(weight[..., None], 3, axis=2)), axis=1)
        Image.fromarray(np.uint8(np.clip(row, 0, 1) * 255)).save(prepared / f"{sid}-alignment.jpg", quality=94)
    write_json(output / "preparation.json", {"target": "local aligned skin residual; global grade/shape excluded",
                                             "pairs": [{"subject": p["subject"], "split": p["split"], **p["preparation"]} for p in pairs]})


def load_model(checkpoint):
    model = UNet(3, 3).eval()
    saved = torch.load(checkpoint, map_location="cpu", weights_only=True)
    model.load_state_dict(saved["generator"] if "generator" in saved else saved)
    model.requires_grad_(False)
    return model


def ort_session(path):
    options = ort.SessionOptions()
    options.intra_op_num_threads = 4
    options.inter_op_num_threads = 1
    return ort.InferenceSession(str(path), sess_options=options, providers=["CPUExecutionProvider"])


def cache_features(model, pairs, output, seed, checkpoint):
    cache = output / "features"
    cache.mkdir(exist_ok=True)
    fingerprint = {"seed": seed, "checkpoint_sha256": sha256(checkpoint),
                   "architecture_sha256": sha256(ROOT / "scripts/third_party/export_skin_retouching_onnx.py"),
                   "outc": hashlib.sha256(model.outc.conv.weight.detach().numpy().tobytes()
                     + model.outc.conv.bias.detach().numpy().tobytes()).hexdigest(),
                   "prepared": {p["subject"]: sha256(output / "prepared" / f"{p['subject']}.npz") for p in pairs}}
    metadata = cache / "provenance.json"
    if metadata.exists() and json.loads(metadata.read_text()) != fingerprint:
        raise ValueError("Feature cache no longer matches data/model; use a new output directory")
    write_json(metadata, fingerprint)
    rng = np.random.default_rng(seed)
    for pair in pairs:
        path = cache / f"{pair['subject']}.npz"
        data = np.load(output / "prepared" / f"{pair['subject']}.npz")
        skin, under, weight = (data[n] for n in ("skin", "under", "weight"))
        groups = [(weight > 0.45, 10000), (under * weight > 0.02, 6000), (skin < 0.05, 6000)]
        def importance(sample):
            # Inverse mixture sampling probability: oversampling small under-eye
            # regions must not silently bias the global skin accuracy objective.
            probabilities = np.zeros_like(sample["skin"])
            selectors = [sample["weight"] > .45, sample["under"] * sample["weight"] > .02, sample["skin"] < .05]
            for (mask, count), selected in zip(groups, selectors):
                if mask.sum():
                    probabilities += selected * count / float(mask.sum())
            result = 1 / np.maximum(probabilities, 1e-6)
            return result / result.mean()
        # Consume the same RNG draws even when a cached subject is skipped;
        # regenerating a partial cache must reproduce an uninterrupted run.
        indices = np.concatenate([rng.choice(np.flatnonzero(mask), count, replace=int(mask.sum()) < count)
                                  for mask, count in groups if mask.sum() > 0])
        if path.exists():
            sample = None
            with np.load(path) as saved:
                if "importance" not in saved.files:
                    sample = {k: saved[k] for k in saved.files}
                    sample["importance"] = importance(sample).astype(np.float32)
            if sample is not None:
                np.savez_compressed(path, **sample)
            continue
        # Store stratified pixels, not entire multi-gigabyte feature tensors.
        with torch.no_grad():
            feat = features(model, as_tensor(data["source"]) * 2 - 1)
            baseline = blend(as_tensor(data["source"]), torch.sigmoid(model.outc.conv(feat)))
            feat = feat[0].permute(1, 2, 0).reshape(-1, 64)[indices].numpy()
            teacher = baseline[0].permute(1, 2, 0).reshape(-1, 3)[indices].numpy()
        sample = dict(feature=feat, source=data["source"].reshape(-1, 3)[indices],
                      target=data["target"].reshape(-1, 3)[indices], teacher=teacher,
                      skin=skin.ravel()[indices], under=under.ravel()[indices], weight=weight.ravel()[indices])
        sample["importance"] = importance(sample).astype(np.float32)
        np.savez_compressed(path, **sample)
        print(f"features {pair['subject']}: {len(indices)} cached pixels", flush=True)


def load_samples(pairs, output, split):
    datasets = [np.load(output / "features" / f"{p['subject']}.npz") for p in pairs if p["split"] == split]
    return {key: torch.from_numpy(np.concatenate([d[key] for d in datasets], axis=0)) for key in datasets[0].files}


def head_predict(head, data):
    mg = torch.sigmoid(F.linear(data["feature"], head.weight[:, :, 0, 0], head.bias))
    return blend(data["source"], mg)


def errors(prediction, data):
    diff = (prediction - data["target"]).abs().mean(-1)
    def average(values, mask):
        mask = mask * data.get("importance", 1)
        return (values * mask).sum() / mask.sum().clamp_min(1)
    return {"skin_mae": average(diff, data["weight"]),
            "under_eye_mae": average(diff, data["under"] * data["weight"]),
            "protected_shift_mae": average((prediction - data["teacher"]).abs().mean(-1), 1 - data["skin"])}


def evaluate_head(head, samples):
    with torch.no_grad():
        return {k: float(v) for k, v in errors(head_predict(head, samples), samples).items()}


def evaluate_full(model, original_head, pairs, output, split):
    """Whole-face metrics averaged by subject, independent of training pixel sampling."""
    per_subject = []
    with torch.no_grad():
        for pair in (p for p in pairs if p["split"] == split):
            data = np.load(output / "prepared" / f"{pair['subject']}.npz")
            x = as_tensor(data["source"])
            feat = features(model, x * 2 - 1)
            old = blend(x, torch.sigmoid(original_head(feat)))
            new = blend(x, torch.sigmoid(model.outc.conv(feat)))
            samples = {n: torch.from_numpy(data[n].reshape(-1, 3) if n in ("source", "target") else data[n].ravel())
                       for n in ("source", "target", "skin", "under", "weight")}
            samples["teacher"] = old[0].permute(1, 2, 0).reshape(-1, 3)
            row = {"subject": pair["subject"]}
            for name, prediction in (("before", old), ("after", new), ("identity", x)):
                pixels = prediction[0].permute(1, 2, 0).reshape(-1, 3)
                row[name] = {k: float(v) for k, v in errors(pixels, samples).items()}
                lum_delta = (pixels - samples["source"]) @ torch.tensor([0.2126, 0.7152, 0.0722])
                row[name]["skin_luma_bias"] = float((lum_delta * samples["weight"]).sum() / samples["weight"].sum())
            per_subject.append(row)
    return {"per_subject": per_subject, **{name: {key: float(np.mean([p[name][key] for p in per_subject]))
             for key in per_subject[0][name]} for name in ("before", "after", "identity")}}


def train(pairs, output, checkpoint, active_onnx, steps, seed, lr, adaptation_weight):
    torch.set_num_threads(4)
    torch.manual_seed(seed)
    model = load_model(checkpoint)
    # Prevent accidentally adapting a stale checkpoint while judging a different live model.
    session = ort_session(active_onnx)
    probe = np.load(output / "prepared" / f"{pairs[0]['subject']}.npz")["source"]
    x = as_tensor(probe) * 2 - 1
    with torch.no_grad():
        tensor_output = model(x).numpy()
    runtime_output = session.run(None, {session.get_inputs()[0].name: x.numpy()})[0]
    equivalence = float(np.max(np.abs(tensor_output - runtime_output)))
    if equivalence > 0.0001:
        raise ValueError(f"Checkpoint does not reproduce active ONNX: max difference {equivalence}")
    cache_features(model, pairs, output, seed, checkpoint)
    training, validation = [load_samples(pairs, output, split) for split in ("train", "validation")]
    before = evaluate_head(model.outc.conv, validation)
    identity = {k: float(v) for k, v in errors(validation["source"], validation).items()}
    head = model.outc.conv
    head.requires_grad_(True)
    original = copy.deepcopy(head.state_dict())
    optimizer = torch.optim.Adam(head.parameters(), lr=lr)
    best, best_score, best_step = copy.deepcopy(original), before["skin_mae"] + before["under_eye_mae"], 0
    rng = np.random.default_rng(seed)
    started = time.perf_counter()
    curve = []
    for step in range(1, steps + 1):
        idx = rng.integers(0, len(training["source"]), 8192)
        batch = {k: v[idx] for k, v in training.items()}
        prediction = head_predict(head, batch)
        metric = errors(prediction, batch)
        loss = metric["skin_mae"] + metric["under_eye_mae"] * 0.75 + metric["protected_shift_mae"] * 0.3
        # Gentle spatial-map and teacher regularization keep frozen features useful.
        loss += sum((p - original[n]).square().mean() for n, p in head.named_parameters()) * 0.005
        optimizer.zero_grad(set_to_none=True)
        loss.backward()
        optimizer.step()
        if step % 25 == 0 or step == 1:
            metric = evaluate_head(head, validation)
            score = metric["skin_mae"] + metric["under_eye_mae"]
            if score < best_score and metric["protected_shift_mae"] < 0.008:
                best, best_score, best_step = copy.deepcopy(head.state_dict()), score, step
            curve.append({"step": step, "validation": metric})
            print(f"step {step}/{steps}: validation skin {metric['skin_mae']:.6f}, under-eye {metric['under_eye_mae']:.6f}, preservation {metric['protected_shift_mae']:.6f}", flush=True)
    head.load_state_dict(best)
    # Conservative parameter interpolation retains the prior model's behavior
    # without running two networks or adding any runtime latency.
    with torch.no_grad():
        for name, parameter in head.named_parameters():
            parameter.copy_(original[name] + (parameter - original[name]) * adaptation_weight)
    after = evaluate_head(head, validation)
    original_head = copy.deepcopy(head)
    original_head.load_state_dict(original)
    validation_full = evaluate_full(model, original_head, pairs, output, "validation")
    # Test identities are first evaluated only after validation has selected the checkpoint.
    test_full = evaluate_full(model, original_head, pairs, output, "test")
    before, after, identity = (validation_full[n] for n in ("before", "after", "identity"))
    test_before, test_after, test_identity = (test_full[n] for n in ("before", "after", "identity"))
    gates = {"validation_skin_improves_3_percent": after["skin_mae"] < before["skin_mae"] * .97,
             "test_skin_improves_3_percent": test_after["skin_mae"] < test_before["skin_mae"] * .97,
             "test_under_eye_improves_3_percent": test_after["under_eye_mae"] < test_before["under_eye_mae"] * .97,
             "test_better_than_unretouched_skin": test_after["skin_mae"] < test_identity["skin_mae"],
             "test_better_than_unretouched_under_eyes": test_after["under_eye_mae"] < test_identity["under_eye_mae"],
             "protected_head_shift_below_two_255_levels": test_after["protected_shift_mae"] < 2 / 255,
             "local_skin_luma_bias_below_two_255_levels": abs(test_after["skin_luma_bias"]) < 2 / 255,
             "nonzero_training_selected": best_step > 0}
    report = {"scope": "paired studio portraits; subject-disjoint local skin blend-head adaptation, not general AI training",
              "split_counts": {split: sum(p["split"] == split for p in pairs) for split in ("train", "validation", "test")},
              "seed": seed, "steps": steps, "selected_step": best_step, "learning_rate": lr,
              "adaptation_weight": adaptation_weight,
              "trainable_parameters": sum(p.numel() for p in head.parameters()),
              "pixel_sampling": "stratified; inverse probability corrected global/region losses",
              "loss_weights": {"skin": 1, "under_eyes": 0.75, "protected": 0.3},
              "active_onnx_sha256": sha256(active_onnx), "checkpoint_sha256": sha256(checkpoint),
              "checkpoint_active_onnx_max_error": equivalence, "training_seconds": time.perf_counter() - started,
              "validation": validation_full,
              "test": test_full,
              "gates": gates, "accepted_for_runtime": all(gates.values()), "curve": curve}
    write_json(output / "metrics.json", report)
    torch.save({"generator": model.state_dict()}, output / "retouch_generator_candidate.pt")
    candidate = output / "retouch_generator_candidate.onnx"
    torch.onnx.export(model, torch.zeros(1, 3, SIZE, SIZE), str(candidate), input_names=["image"],
                      output_names=["pred_mg"], opset_version=11, do_constant_folding=True, dynamo=False)
    onnx.checker.check_model(onnx.load(candidate))
    candidate_session = ort_session(candidate)
    max_error = 0.0
    for pair in [p for p in pairs if p["split"] == "test"]:
        data = np.load(output / "prepared" / f"{pair['subject']}.npz")
        x = as_tensor(data["source"]) * 2 - 1
        with torch.no_grad():
            expected = model(x).numpy()
        actual = candidate_session.run(None, {candidate_session.get_inputs()[0].name: x.numpy()})[0]
        max_error = max(max_error, float(np.max(np.abs(actual - expected))))
        old = session.run(None, {session.get_inputs()[0].name: x.numpy()})[0]
        src = data["source"]
        render = lambda mask: np.clip((1 - 2 * mask[0].transpose(1, 2, 0)) * src ** 2 + 2 * mask[0].transpose(1, 2, 0) * src, 0, 1)
        # Runtime skin guard is reproduced for visual checks. The network's raw
        # output is used above for accuracy metrics so masking cannot hide errors.
        skin = data["skin"][..., None]
        old_render, new_render = src + (render(old) - src) * skin, src + (render(actual) - src) * skin
        row = np.concatenate((src, data["target"], old_render, new_render), axis=1)
        board = Image.new("RGB", (SIZE * 4, SIZE + 28), (18, 18, 18))
        board.paste(Image.fromarray(np.uint8(np.clip(row, 0, 1) * 255)), (0, 28))
        draw = ImageDraw.Draw(board)
        for i, label in enumerate(("Original", "Aligned local-skin target", "Current AI", "Trained candidate")):
            draw.text((i * SIZE + 8, 8), f"Subject {pair['subject']} | {label}", fill="white")
        board.save(output / f"test-{pair['subject']}-review.png")
    report["onnx_max_error"] = max_error
    report["candidate_sha256"] = sha256(candidate)
    report["gates"]["onnx_matches_pytorch"] = max_error < 0.0001
    report["accepted_for_runtime"] = all(report["gates"].values())
    write_json(output / "metrics.json", report)
    print(json.dumps({k: v for k, v in report.items() if k != "curve"}, indent=2), flush=True)
    print("Candidate is eligible for runtime review." if report["accepted_for_runtime"] else
          "Candidate rejected for runtime; active ONNX/Burn assets remain unchanged.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "output/model-training/day-2")
    parser.add_argument("--phase", choices=["inventory", "prepare", "train", "promote"], required=True)
    parser.add_argument("--checkpoint", type=Path, default=(ROOT / "models/retouch_generator.pt" if (ROOT / "models/retouch_generator.pt").exists()
                         else ROOT / "output/model-training/personalized-retouch/retouch_generator_head_only.pt"))
    parser.add_argument("--active-onnx", type=Path, default=ROOT / "models/retouch_generator.onnx")
    parser.add_argument("--seed", type=int, default=20261002)
    parser.add_argument("--steps", type=int, default=3000)
    parser.add_argument("--learning-rate", type=float, default=0.0002)
    parser.add_argument("--adaptation-weight", type=float, default=0.8,
                        help="Fraction of the learned blend-head update to retain; no extra runtime model")
    args = parser.parse_args()
    if not 0 < args.adaptation_weight <= 1:
        raise ValueError("Adaptation weight must be in (0, 1]")
    if args.output.resolve() == args.dataset.resolve() or args.dataset.resolve() in args.output.resolve().parents:
        raise ValueError("Output must be separate from the read-only source dataset")
    args.output.mkdir(parents=True, exist_ok=True)
    if args.phase == "inventory":
        pairs = inventory(args.dataset, args.output, args.seed)
        print(f"Matched {len(pairs)} original/edited subjects. Split:", {s: sum(p["split"] == s for p in pairs) for s in ("train", "validation", "test")})
    else:
        manifest = json.loads((args.output / "manifest.json").read_text(encoding="utf-8"))
        if manifest["seed"] != args.seed or Path(manifest["source_root"]) != args.dataset.resolve():
            raise ValueError("Dataset/seed differ from manifest; use a new output directory")
        pairs = manifest["pairs"]
        for pair in pairs:
            for label in ("original", "edited"):
                if sha256(Path(pair[label])) != pair[f"{label}_sha256"]:
                    raise ValueError(f"Subject {pair['subject']} {label} changed after inventory")
        if args.phase == "promote":
            promote(args.output, args.active_onnx, args.checkpoint)
        elif args.phase == "prepare":
            prepare(pairs, args.output)
        else:
            if args.steps < 25:
                raise ValueError("Use at least 25 training steps")
            train(pairs, args.output, args.checkpoint, args.active_onnx, args.steps, args.seed, args.learning_rate, args.adaptation_weight)


if __name__ == "__main__":
    main()
