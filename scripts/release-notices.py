"""Collect verbatim notices from the locked Windows dependency graph. Build tool only."""
import argparse
import hashlib
import json
import re
import shutil
import tomllib
import urllib.request
from pathlib import Path


def upstream_notices(package, root, cache):
    """Fetch only license text at the crate's recorded source commit, never HEAD."""
    vcs_file = root / ".cargo_vcs_info.json"
    repository_url = package.get("repository")
    if package["name"] == "cubecl-zspace" and not repository_url:
        repository_url = "https://github.com/tracel-ai/cubecl"
    repository = re.match(r"https://github.com/([^/]+/[^/]+)", repository_url or "")
    if not repository or not vcs_file.exists():
        return []
    repository = repository.group(1).removesuffix(".git")
    vcs = json.loads(vcs_file.read_text())
    commit = vcs["git"]["sha1"]
    directory = cache / repository.replace("/", "_") / commit
    directory.mkdir(parents=True, exist_ok=True)
    tree_file = directory / "tree.json"
    if not tree_file.exists():
        request = urllib.request.Request(f"https://api.github.com/repos/{repository}/git/trees/{commit}?recursive=1", headers={"User-Agent": "Hastur-release-notices"})
        with urllib.request.urlopen(request, timeout=30) as response:
            tree_file.write_bytes(response.read())
    tree = json.loads(tree_file.read_text())
    if tree.get("truncated"):
        raise RuntimeError(f"Truncated license source tree: {repository}")
    location = Path(vcs.get("path_in_vcs", ""))
    parents = {location.as_posix(), *(p.as_posix() for p in location.parents), "."}
    candidates = []
    for entry in tree["tree"]:
        path = Path(entry["path"])
        if entry["type"] != "blob" or path.parent.as_posix() not in parents:
            continue
        if path.name.lower().startswith(("license", "licence", "copying", "notice", "copyright")):
            candidates.append(entry["path"])
    records = []
    for relative in candidates:
        url = f"https://raw.githubusercontent.com/{repository}/{commit}/{relative}"
        file = directory / relative
        file.parent.mkdir(parents=True, exist_ok=True)
        if not file.exists():
            with urllib.request.urlopen(url, timeout=30) as response:
                file.write_bytes(response.read())
        records.append((f"upstream/{relative}", file.read_bytes(), url))
    return records


def generate(metadata_path, tree_path, output, cache):
    metadata = json.loads(metadata_path.read_text(encoding="utf-8-sig"))
    lock = tomllib.loads((Path(metadata["workspace_root"]) / "Cargo.lock").read_text(encoding="utf-8"))
    locked_hashes = {(p["name"], p["version"]): p.get("checksum") for p in lock["package"]}
    # cargo tree resolves the actual active features; metadata includes optional
    # unbuilt packages retained in Cargo.lock as well.
    selected = set(re.findall(r"^([\w-]+) v([^\s]+)", tree_path.read_text(encoding="utf-8-sig"), re.MULTILINE))
    records = []
    missing = []
    text = ["Hastur Retouch: third-party Rust dependency notices\n",
            "Includes normal and build dependencies for the locked Windows x64 graph.\n",
            "Original notice files are reproduced below without modification.\n",
            "Exact registry source downloads are linked for source availability.\n"]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if (package["name"], package["version"]) not in selected or package["source"] is None:
            continue
        root = Path(package["manifest_path"]).parent
        files = []
        for path in sorted(root.rglob("*")):
            if not path.is_file() or any(part in (".git", "target") for part in path.relative_to(root).parts):
                continue
            name = path.name.lower()
            if name.startswith(("license", "licence", "copying", "notice")) or "license" in name or name in ("ofl.txt", "ufl.txt"):
                if path.suffix.lower() not in (".rs", ".py", ".json", ".png", ".pdf"):
                    files.append(path)
        if package.get("license_file"):
            path = root / package["license_file"]
            if path.is_file() and path not in files:
                files.append(path)
        contents = [(p.relative_to(root).as_posix(), p.read_bytes(), None) for p in files]
        if not files or package["name"] == "epaint_default_fonts":
            contents += upstream_notices(package, root, cache)
        standards = {"buildid": "MPL-2.0", "hexf-parse": "CC0-1.0", "utf8-chars": "Apache-2.0"}
        if not contents and package["name"] in standards:
            identifier = standards[package["name"]]
            standard = Path(__file__).resolve().parent.parent / "release" / "licenses" / f"{identifier}.txt"
            contents.append((f"standard-license/{identifier}.txt", standard.read_bytes(), f"https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/{identifier}.txt"))
            # Retain the publisher's actual license declaration and any README notices.
            for name in ("Cargo.toml.orig", "README.md"):
                if (root / name).is_file():
                    contents.append((name, (root / name).read_bytes(), None))
        if not contents:
            missing.append(f'{package["name"]} {package["version"]} ({package.get("license")})')
            continue
        source = f'https://crates.io/api/v1/crates/{package["name"]}/{package["version"]}/download'
        text.append(f'\n{"=" * 72}\n{package["name"]} {package["version"]}\nSPDX: {package.get("license")}\nSource: {source}\n')
        notices = []
        for relative, data, url in contents:
            # Fail rather than silently omit a non-text license.
            notice = data.decode("utf-8-sig")
            notices.append({"file": relative, "sha256": hashlib.sha256(data).hexdigest(), "source": url})
            text.append(f'\n--- {relative} ---\n{notice}\n')
        records.append({"name": package["name"], "version": package["version"], "license": package.get("license"), "authors": package["authors"], "source": source, "notices": notices})
        if package["license"] == "MPL-2.0":
            archive = root.parents[2] / "cache" / root.parent.name / f'{package["name"]}-{package["version"]}.crate'
            if hashlib.sha256(archive.read_bytes()).hexdigest() != locked_hashes[(package["name"], package["version"])]:
                raise RuntimeError(f"MPL source archive differs from Cargo.lock: {archive.name}")
            source_output = output / "source-code"
            source_output.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(archive, source_output / archive.name)
    if missing:
        raise SystemExit("Missing verbatim licenses:\n" + "\n".join(missing))
    output.mkdir(parents=True, exist_ok=True)
    (output / "Rust-THIRD-PARTY-NOTICES.txt").write_text("".join(text), encoding="utf-8")
    (output / "rust-dependencies.json").write_text(json.dumps(records, indent=2) + "\n", encoding="utf-8")
    print(f"Collected {len(records)} dependency notice sets.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("tree", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cache", type=Path, default=Path("output/release-license-cache"))
    args = parser.parse_args()
    generate(args.metadata, args.tree, args.output, args.cache)
