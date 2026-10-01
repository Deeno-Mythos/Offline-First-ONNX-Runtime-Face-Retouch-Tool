"""Personalize the pretrained blend head with synthetic, reversible defects.

This is a one-portrait adaptation experiment, not a substitute for a diverse,
paired retouch dataset. It trains only the final 1x1 blend head and keeps the
candidate separate until synthetic validation and clean-image preservation pass.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import sys
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from PIL import Image, ImageDraw, ImageFilter, ImageOps


ROOT = Path(__file__).resolve().parent.parent
EXPORTER = ROOT / "scripts" / "third_party" / "export_skin_retouching_onnx.py"
sys.path.insert(0, str(EXPORTER.parent))
from export_skin_retouching_onnx import UNet  # noqa: E402

OVAL = [10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365,
        379, 378, 400, 377, 152, 148, 176, 149, 150, 136, 172, 58, 132, 93,
        234, 127, 162, 21, 54, 103, 67, 109]
EYE_R = [33, 7, 163, 144, 145, 153, 154, 155, 133, 173, 157, 158, 159, 160, 161, 160, 246]
EYE_L = [263, 249, 390, 373, 374, 380, 381, 382, 362, 398, 384, 385, 386, 387, 388, 466]
LIPS = [61, 146, 91, 181, 84, 17, 314, 405, 321, 375, 291, 409, 270, 269, 267, 0, 37, 39, 40, 185]
BROW_R = [70, 63, 105, 66, 107, 55, 65, 52, 53, 46]
BROW_L = [300, 293, 334, 296, 336, 285, 295, 282, 283, 276]


def load_face(ron_path: Path) -> tuple[np.ndarray, tuple[float, ...]]:
    text = ron_path.read_text(encoding="utf-8")
    points_match = re.search(r"landmarks:\s*\[(.*?)\]\s*,\s*bounds:", text, re.S)
    bounds_match = re.search(r"bounds:\s*\(([^)]*)\)", text)
    if not points_match or not bounds_match:
        raise ValueError(f"Could not read a FaceMesh from {ron_path}")
    points = np.asarray(
        [[float(v.strip()) for v in row.split(",")] for row in re.findall(r"\(([^)]*)\)", points_match.group(1))],
        dtype=np.float32,
    )
    bounds = tuple(float(v.strip()) for v in bounds_match.group(1).split(","))
    if points.shape != (478, 3) or len(bounds) != 4:
        raise ValueError(f"Expected 478 landmarks and 4 bounds; got {points.shape}, {bounds}")
    return points, bounds


def face_crop(image: Image.Image, landmarks: np.ndarray, bounds: tuple[float, ...]) -> tuple[np.ndarray, np.ndarray]:
    image = ImageOps.exif_transpose(image).convert("RGB")
    width, height = image.size
    side = max(bounds[2] * width, bounds[3] * height) * 1.5
    center_x = (bounds[0] + bounds[2] * 0.5) * width
    center_y = (bounds[1] + bounds[3] * 0.5) * height
    left = max(center_x - side * 0.5, 0.0)
    top = max(center_y - side * 0.5, 0.0)
    right = min(center_x + side * 0.5, float(width))
    bottom = min(center_y + side * 0.5, float(height))
    rect_w, rect_h = right - left, bottom - top
    crop = image.crop((left, top, right, bottom)).resize((512, 512), Image.Resampling.LANCZOS)
    crop_points = landmarks.copy()
    crop_points[:, 0] = (landmarks[:, 0] * width - left) * 512.0 / rect_w
    crop_points[:, 1] = (landmarks[:, 1] * height - top) * 512.0 / rect_h
    return np.asarray(crop, dtype=np.float32) / 255.0, crop_points


def eye_region(points: np.ndarray, a: int, b: int, lower: int, chin: int):
    first, second = points[a, :2], points[b, :2]
    delta = second - first
    width = float(np.linalg.norm(delta))
    if width < 1.0:
        return None
    axis = delta / width
    down = np.asarray([-axis[1], axis[0]])
    center = (first + second) * 0.5
    if float(np.dot(points[chin, :2] - center, down)) < 0.0:
        down = -down
    return points[lower, :2], axis, down, width


def make_masks(points: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    skin_img = Image.new("L", (512, 512), 0)
    draw = ImageDraw.Draw(skin_img)
    draw.polygon([tuple(points[i, :2]) for i in OVAL], fill=255)
    for feature in (EYE_R, EYE_L, LIPS, BROW_R, BROW_L):
        draw.polygon([tuple(points[i, :2]) for i in feature], fill=0)
    skin_img = skin_img.filter(ImageFilter.GaussianBlur(2.0))
    skin = np.asarray(skin_img, dtype=np.float32) / 255.0

    yy, xx = np.mgrid[0:512, 0:512].astype(np.float32)
    under = np.zeros((512, 512), dtype=np.float32)
    for spec in ((33, 133, 145), (362, 263, 374)):
        region = eye_region(points, *spec, 152)
        if region is None:
            continue
        lower, axis, down, width = region
        delta = np.stack((xx - lower[0] - down[0] * width * 0.42,
                          yy - lower[1] - down[1] * width * 0.42), axis=-1)
        along = delta[..., 0] * axis[0] + delta[..., 1] * axis[1]
        below = delta[..., 0] * down[0] + delta[..., 1] * down[1]
        distance = np.sqrt((along / (width * 0.64)) ** 2 + (below / (width * 0.46)) ** 2)
        weight = np.clip(1.0 - distance * distance, 0.0, 1.0) ** 2
        under = np.maximum(under, weight)
    under *= skin
    # Disallow synthetic acne directly on eye, brow, lip and hair pixels.
    return skin, under


def synthesize(clean: np.ndarray, skin: np.ndarray, under: np.ndarray, seed: int,
               mode: str | None = None):
    rng = np.random.default_rng(seed)
    image = clean.copy()
    damage = np.zeros((512, 512), dtype=np.float32)
    mode = mode or ("under-eye" if seed % 2 == 0 else "blemish")

    if mode in ("under-eye", "combined"):
        strength = float(rng.uniform(0.18, 0.48))
        shape_scale = float(rng.uniform(0.7, 1.0))
        field = np.clip(under * shape_scale, 0.0, 1.0)
        field = np.asarray(Image.fromarray(np.uint8(field * 255)).filter(
            ImageFilter.GaussianBlur(float(rng.uniform(0.5, 2.0)))), dtype=np.float32) / 255.0
        alpha = field * strength
        shadow = np.asarray([0.32, 0.22, 0.22], dtype=np.float32)
        image = image * (1.0 - alpha[..., None]) + shadow * alpha[..., None]
        damage = np.maximum(damage, alpha)

    if mode in ("blemish", "combined"):
        allowed = (skin > 0.28) & (under < 0.025)
        # The clean photo is the target, so training only corrupts pixels that lie on
        # the detected face oval and outside protected features.
        ys, xs = np.where(allowed)
        if len(xs):
            yy, xx = np.mgrid[0:512, 0:512].astype(np.float32)
            for _ in range(int(rng.integers(2, 8))):
                index = int(rng.integers(0, len(xs)))
                cx, cy = float(xs[index]), float(ys[index])
                rx, ry = float(rng.uniform(1.5, 5.0)), float(rng.uniform(1.5, 5.0))
                distance = ((xx - cx) / rx) ** 2 + ((yy - cy) / ry) ** 2
                alpha = np.clip(1.0 - distance, 0.0, 1.0) ** 2 * float(rng.uniform(0.25, 0.65))
                color = np.asarray(rng.choice([[0.36, 0.15, 0.12], [0.24, 0.16, 0.12],
                                               [0.50, 0.22, 0.20]]), dtype=np.float32)
                image = image * (1.0 - alpha[..., None]) + color * alpha[..., None]
                damage = np.maximum(damage, alpha)

    flipped = rng.random() < 0.5
    if flipped:
        image, damage = image[:, ::-1].copy(), damage[:, ::-1].copy()
        clean = clean[:, ::-1].copy()
    return image, damage, clean, flipped


def blend(image: torch.Tensor, mask: torch.Tensor) -> torch.Tensor:
    return (1.0 - 2.0 * mask) * image.square() + 2.0 * mask * image


def features_until_up4(model: UNet, image: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
    x1 = model.inc(image)
    x2 = model.down1(x1)
    x3 = model.down2(x2)
    x4 = model.down3(x3)
    x5 = model.down4(x4)
    x44 = model.up1(x5, x4)
    x33 = model.up2(x44, x3)
    x22 = model.up3(x33, x2)
    return x22, x1


def features(model: UNet, image: torch.Tensor) -> torch.Tensor:
    x22, x1 = features_until_up4(model, image)
    return model.up4(x22, x1)


def as_tensor(data: np.ndarray) -> torch.Tensor:
    return torch.from_numpy(np.ascontiguousarray(data.transpose(2, 0, 1))).unsqueeze(0)


def predict(model: UNet, image: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
    with torch.no_grad():
        feat = features(model, image * 2.0 - 1.0)
        mask = torch.sigmoid(model.outc.conv(feat))
        return mask, blend(image, mask)


def evaluate(model: UNet, clean: np.ndarray, skin: np.ndarray, under: np.ndarray,
             base_teacher: torch.Tensor, seeds: range) -> dict[str, float]:
    repair, objective, preservation = [], [], []
    clean_tensor = as_tensor(clean)
    for seed in seeds:
        mode = "under-eye" if seed % 2 == 0 else "blemish"
        corrupted, damage, target, flipped = synthesize(clean, skin, under, seed, mode)
        x = as_tensor(corrupted)
        target_t = as_tensor(target)
        damage_t = torch.from_numpy(damage.copy()).view(1, 1, 512, 512)
        teacher = base_teacher.flip(-1) if flipped else base_teacher
        _, prediction = predict(model, x)
        repair_error = ((prediction - target_t).abs() * damage_t).sum() / (damage_t.sum() * 3.0 + 1e-6)
        preservation_error = ((prediction - teacher).abs() * (1.0 - damage_t)).sum() / (
            (1.0 - damage_t).sum() * 3.0 + 1e-6
        )
        # Normalize each region independently. A whole-image mean almost erased the
        # training signal because the synthetic eye/blemish masks cover few pixels.
        repair.append(float(repair_error))
        preservation.append(float(preservation_error))
        objective.append(float(repair_error + preservation_error * 0.25))
    base_mask, base_clean = predict(model, clean_tensor)
    return {
        "objective_mae": float(np.mean(objective)),
        "repair_mae": float(np.mean(repair)),
        "preservation_mae": float(np.mean(preservation)),
        "clean_output_mae_from_teacher": float((base_clean - base_teacher).abs().mean()),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", type=Path, default=ROOT / "output/example-img-analysis/original.png")
    parser.add_argument("--landmarks", type=Path, default=ROOT / "output/example-img-analysis/landmarks.ron")
    parser.add_argument("--steps", type=int, default=64)
    parser.add_argument("--seed", type=int, default=7919)
    parser.add_argument("--learning-rate", type=float, default=1e-4)
    parser.add_argument("--train-up4", action="store_true",
                        help="Fine-tune the final decoder block as well as its blend head")
    args = parser.parse_args()
    if args.steps < 8:
        raise ValueError("Use at least 8 steps so both synthetic defect families are sampled")
    torch.manual_seed(args.seed)
    torch.set_num_threads(min(4, torch.get_num_threads()))
    np.random.seed(args.seed)

    image = ImageOps.exif_transpose(Image.open(args.image)).convert("RGB")
    landmarks, bounds = load_face(args.landmarks)
    clean, crop_points = face_crop(image, landmarks, bounds)
    skin, under = make_masks(crop_points)
    if float(skin.sum()) < 1000 or float(under.sum()) < 50:
        raise ValueError("Face/under-eye masks are too small; refusing to train on a bad crop")

    checkpoint = torch.load(ROOT / "models/pytorch_model.pt", map_location="cpu", weights_only=False)
    model = UNet(3, 3).eval()
    state = checkpoint["generator"] if isinstance(checkpoint, dict) and "generator" in checkpoint else checkpoint
    model.load_state_dict(state)
    for parameter in model.parameters():
        parameter.requires_grad_(False)
    head = model.outc.conv
    trainable_modules = [model.up4, model.outc] if args.train_up4 else [model.outc]
    for parameter in (p for module in trainable_modules for p in module.parameters()):
        parameter.requires_grad_(True)
    original_parameters = {
        f"{index}.{name}": parameter.detach().clone()
        for index, module in enumerate(trainable_modules)
        for name, parameter in module.named_parameters()
    }

    clean_tensor = as_tensor(clean)
    _, teacher = predict(model, clean_tensor)
    with torch.no_grad():
        clean_skip, clean_shallow = features_until_up4(model, clean_tensor * 2.0 - 1.0)
        clean_features = model.up4(clean_skip, clean_shallow)
    before = evaluate(model, clean, skin, under, teacher, range(args.seed + 200, args.seed + 216))
    parameters = [p for module in trainable_modules for p in module.parameters()]
    optimizer = torch.optim.Adam(parameters, lr=args.learning_rate, weight_decay=1e-5)
    started = time.perf_counter()
    for step in range(args.steps):
        mode = ("under-eye", "under-eye", "blemish", "combined")[step % 4]
        corrupted, damage, target, flipped = synthesize(clean, skin, under, args.seed + step, mode)
        x, target_t = as_tensor(corrupted), as_tensor(target)
        damage_t = torch.from_numpy(damage.copy()).view(1, 1, 512, 512)
        teacher_target = teacher.flip(-1) if flipped else teacher
        x_skip, x_shallow = features_until_up4(model, x * 2.0 - 1.0)
        feat = model.up4(x_skip, x_shallow)
        mask = torch.sigmoid(head(feat))
        prediction = blend(x, mask)
        repair_loss = ((prediction - target_t).abs() * damage_t).sum() / (damage_t.sum() * 3.0 + 1e-6)
        preserve_mask = 1.0 - damage_t
        preserve_loss = ((prediction - teacher_target).abs() * preserve_mask).sum() / (
            preserve_mask.sum() * 3.0 + 1e-6
        )
        current_clean_features = (
            model.up4(clean_skip, clean_shallow) if args.train_up4 else clean_features
        )
        clean_distill = (
            blend(clean_tensor, torch.sigmoid(head(current_clean_features))) - teacher
        ).abs().mean()
        loss = repair_loss + preserve_loss * 0.25 + clean_distill * 0.1
        current_parameters = {
            f"{index}.{name}": parameter
            for index, module in enumerate(trainable_modules)
            for name, parameter in module.named_parameters()
        }
        loss = loss + sum(
            (current_parameters[key] - original_parameters[key]).square().mean()
            for key in original_parameters
        ) * 0.002
        optimizer.zero_grad(set_to_none=True)
        loss.backward()
        optimizer.step()
        if (step + 1) % 8 == 0 or step == 0:
            print(f"step {step + 1}/{args.steps} · loss {loss.item():.6f} · {time.perf_counter() - started:.1f}s", flush=True)

    after = evaluate(model, clean, skin, under, teacher, range(args.seed + 200, args.seed + 216))
    _, clean_after = predict(model, clean_tensor)
    clean_shift = float((clean_after - teacher).abs().mean())
    improves = after["objective_mae"] < before["objective_mae"] * 0.99
    natural = clean_shift <= 0.01 and after["preservation_mae"] <= before["preservation_mae"] + 0.005
    passed = improves and natural

    output_dir = ROOT / "output" / "model-training" / "personalized-retouch"
    output_dir.mkdir(parents=True, exist_ok=True)
    metrics = {
        "scope": "personalized synthetic-pair fine-tune from one portrait; not a general model benchmark",
        "steps": args.steps,
        "trainable_modules": ["up4", "outc"] if args.train_up4 else ["outc"],
        "learning_rate": args.learning_rate,
        "training_seconds": time.perf_counter() - started,
        "before": before,
        "after": after,
        "clean_shift_mae": clean_shift,
        "gates": {"heldout_objective_improved_1_percent": improves,
                  "clean_and_preservation_limits_passed": natural},
        "accepted_for_export": passed,
    }
    (output_dir / "metrics.json").write_text(json.dumps(metrics, indent=2), encoding="utf-8")
    print(json.dumps(metrics, indent=2), flush=True)
    if not passed:
        print("Candidate rejected; the active ONNX and Burn runtime assets were left unchanged.", flush=True)
        return

    candidate = output_dir / "retouch_generator_personalized.onnx"
    torch.onnx.export(
        model,
        torch.zeros(1, 3, 512, 512),
        str(candidate),
        input_names=["image"],
        output_names=["pred_mg"],
        opset_version=11,
        do_constant_folding=True,
    )
    torch.save({"generator": model.state_dict()}, output_dir / "retouch_generator_personalized.pt")
    print(f"Exported validated candidate: {candidate}", flush=True)


if __name__ == "__main__":
    main()
