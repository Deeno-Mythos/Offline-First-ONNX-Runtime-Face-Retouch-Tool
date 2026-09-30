"""Optional Cargo.lock downloader for systems with a broken DNS resolver.

Uses curl's --resolve without changing system DNS and verifies Cargo checksums.
Normal installations should simply use cargo build.
"""
import argparse
import concurrent.futures
import hashlib
import pathlib
import subprocess
import tomllib

parser = argparse.ArgumentParser()
parser.add_argument("--ip", required=True, help="Currently resolved IPv4 address of static.crates.io")
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[1]
lock = tomllib.loads((root / "Cargo.lock").read_text())
cache = pathlib.Path.home() / ".cargo/registry/cache/index.crates.io-1949cf8c6b5b557f"
cache.mkdir(parents=True, exist_ok=True)
pending = [p for p in lock["package"] if p.get("source", "").startswith("registry+") and not (cache / f'{p["name"]}-{p["version"]}.crate').exists()]

def fetch(p):
    name = f'{p["name"]}-{p["version"]}.crate'
    target = cache / name
    temporary = target.with_suffix(".download")
    subprocess.run(["curl.exe", "--silent", "--show-error", "--fail", "--retry", "2", "--ssl-no-revoke", "--connect-timeout", "15", "--max-time", "120", "--resolve", f"static.crates.io:443:{args.ip}", f'https://static.crates.io/crates/{p["name"]}/{name}', "--output", str(temporary)], check=True)
    if hashlib.sha256(temporary.read_bytes()).hexdigest() != p["checksum"]:
        temporary.unlink()
        raise RuntimeError(f"Checksum mismatch: {name}")
    temporary.replace(target)
    return name

print(f"Fetching {len(pending)} locked crates; verifying every checksum.", flush=True)
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    for index, name in enumerate(pool.map(fetch, pending), 1):
        if index % 10 == 0 or index == len(pending):
            print(f"{index}/{len(pending)} · {name}", flush=True)
