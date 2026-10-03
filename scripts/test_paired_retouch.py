"""Regression checks for paired-data safety and geometry/color target isolation."""
import tempfile
import unittest
import importlib.util
import json
from pathlib import Path

import cv2
import numpy as np
import torch
from PIL import Image

from train_paired_retouch import align_pair, copy_atomic, crop_face, errors, inventory, normalize_target, promote, sha256


class PairedRetouchTests(unittest.TestCase):
    def test_rejected_or_changed_candidate_cannot_replace_active_model(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            active = output / "active.onnx"
            active.write_bytes(b"original")
            candidate = output / "retouch_generator_candidate.onnx"
            candidate.write_bytes(b"candidate")
            report = {"accepted_for_runtime": False, "gates": {"test": False},
                      "candidate_sha256": sha256(candidate), "active_onnx_sha256": sha256(active)}
            (output / "metrics.json").write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, "failed quality"):
                promote(output, active, output / "unused.pt")
            report.update(accepted_for_runtime=True, gates={"test": True})
            (output / "metrics.json").write_text(json.dumps(report))
            candidate.write_bytes(b"altered after validation")
            with self.assertRaisesRegex(ValueError, "changed after validation"):
                promote(output, active, output / "unused.pt")
            self.assertEqual(active.read_bytes(), b"original")

    def test_atomic_copy_and_setup_preserve_verified_training(self):
        spec = importlib.util.spec_from_file_location("setup_ai", Path(__file__).with_name("setup-ai.py"))
        setup = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(setup)
        with tempfile.TemporaryDirectory() as directory:
            setup.MODELS = Path(directory)
            source = setup.MODELS / "candidate"
            source.write_bytes(b"trained")
            for name in ("retouch_generator.onnx", "retouch_generator.pt"):
                copy_atomic(source, setup.MODELS / name)
            record = {"accepted_for_runtime": True, "gates": {"quality": True},
                      "onnx_sha256": sha256(source), "checkpoint_sha256": sha256(source)}
            (setup.MODELS / "retouch-training.json").write_text(json.dumps(record))
            self.assertTrue(setup.keep_trained_generator())
            (setup.MODELS / "retouch_generator.onnx").write_bytes(b"broken")
            with self.assertRaisesRegex(RuntimeError, "not overwritten"):
                setup.keep_trained_generator()
            self.assertEqual((setup.MODELS / "retouch_generator.onnx").read_bytes(), b"broken")
            self.assertFalse(list(setup.MODELS.glob("*.training-tmp")))

    def test_sampling_importance_removes_region_oversampling_bias(self):
        prediction = torch.cat((torch.zeros(10, 3), torch.ones(100, 3)))
        data = {"target": torch.zeros_like(prediction), "teacher": prediction,
                "weight": torch.ones(110), "skin": torch.ones(110),
                "under": torch.cat((torch.zeros(10), torch.ones(100))),
                "importance": torch.cat((torch.ones(10), torch.full((100,), .01)))}
        metric = errors(prediction, data)
        self.assertAlmostEqual(float(metric["skin_mae"]), 1 / 11, places=6)
        self.assertAlmostEqual(float(metric["under_eye_mae"]), 1, places=6)
        self.assertEqual(float(metric["protected_shift_mae"]), 0)

    def test_subject_split_is_deterministic_and_disjoint(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "data"
            root.mkdir()
            for subject in range(18):
                folder = root / f"JK - {subject:03} - EXAMPLE"
                (folder / "EDITED").mkdir(parents=True)
                for path in (folder / "photo.JPG", folder / "EDITED/photo-OF-02.JPG"):
                    Image.new("RGB", (24, 24), (subject * 10, 80, 40)).save(path)
            before = {str(p): p.read_bytes() for p in root.rglob("*.JPG")}
            first = inventory(root, Path(directory) / "run1", 123)
            second = inventory(root, Path(directory) / "run2", 123)
            self.assertEqual(first, second)
            splits = [{p["subject"] for p in first if p["split"] == s} for s in ("train", "validation", "test")]
            self.assertEqual([len(s) for s in splits], [12, 3, 3])
            self.assertFalse(splits[0] & splits[1] or splits[1] & splits[2] or splits[0] & splits[2])
            self.assertEqual(before, {str(p): p.read_bytes() for p in root.rglob("*.JPG")})
            Image.new("RGB", (24, 24)).save(root / "JK - 000 - EXAMPLE/extra.JPG")
            with self.assertRaisesRegex(ValueError, "Ambiguous pairing"):
                inventory(root, Path(directory) / "bad", 123)

    def test_crop_uses_runtime_source_grid(self):
        yy, xx = np.mgrid[:100, :120].astype(np.float32)
        image = np.stack((xx / 120, yy / 100, xx * 0), axis=-1)
        points = np.array([[.5, .5, 0]], dtype=np.float32)
        crop, pts = crop_face(image, points, (.4, .4, .2, .2))
        np.testing.assert_allclose(crop[256, 256, :2], [.5, .5], atol=.001)
        np.testing.assert_allclose(pts[0, :2], [256, 256], atol=.001)

    def test_global_grade_excluded_but_local_retouch_retained(self):
        yy, xx = np.mgrid[:512, :512].astype(np.float32)
        ramp = .3 + .45 * xx / 512 + .04 * np.sin(yy / 10)
        source = np.stack((ramp, ramp * .82, ramp * .67), axis=-1)
        edited = source * .84 + .08
        skin = np.zeros((512, 512), np.float32)
        skin[40:470, 40:470] = 1
        confidence = np.ones_like(skin)
        clean, _ = normalize_target(source, edited, skin, confidence)
        self.assertLess(float(np.abs(clean - source).mean()), .0001)
        edited[230:242, 220:246] += .025
        target, weight = normalize_target(source, edited, skin, confidence)
        self.assertGreater(float((target - source)[232:240, 224:242].mean()), .018)
        np.testing.assert_array_equal(target[skin == 0], source[skin == 0])
        self.assertTrue(np.isfinite(target).all() and np.isfinite(weight).all())

    def test_alignment_recovers_known_crop_offset(self):
        rng = np.random.default_rng(31)
        source = cv2.GaussianBlur(rng.uniform(.15, .85, (512, 512, 3)).astype(np.float32), (0, 0), 2)
        matrix = np.array([[1, 0, 8], [0, 1, -6]], np.float32)
        edited = cv2.warpAffine(source, matrix, (512, 512), borderMode=cv2.BORDER_REFLECT)
        points = rng.uniform(80, 430, (478, 3)).astype(np.float32)
        points[:, 2] = 0
        edit_points = points + np.array([8, -6, 0], np.float32)
        registered, confidence, quality = align_pair(source, edited, points, edit_points)
        self.assertGreater(quality["registration_correlation"], .96)
        self.assertLess(float(np.abs(registered[30:-30, 30:-30] - source[30:-30, 30:-30]).mean()), .002)
        self.assertGreater(float(confidence[30:-30, 30:-30].mean()), .9)


if __name__ == "__main__":
    unittest.main()
