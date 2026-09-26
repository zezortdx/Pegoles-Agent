#!/usr/bin/env python3
"""Emit a Pegoles model-catalog entry pinned to exact bytes.

usage: pin_hf_model.py <repo> <revision> <id> <display_name> <family> <quantization> <published>

Reads the file list of a Hugging Face repo at an immutable commit, takes
SHA-256 digests from LFS metadata, downloads the small non-LFS files to
hash them, and prints JSON for crates/pegoles-inference/catalog/models.json.
BUILD TIME ONLY: the app never trusts a remote file list; it trusts the
catalog compiled into it.
"""

import hashlib
import json
import sys
import urllib.request

SKIP = {".gitattributes", "README.md"}
ALLOWED = (".safetensors", ".json", ".txt", ".jinja", ".model", ".tiktoken")


def get(url):
    with urllib.request.urlopen(url, timeout=60) as r:
        return r.read()


def main():
    repo, rev, mid, name, family, quant, published = sys.argv[1:8]
    if len(rev) != 40:
        sys.exit("revision must be a full commit hash")
    meta = json.loads(get(f"https://huggingface.co/api/models/{repo}/revision/{rev}?blobs=true"))
    if meta.get("sha") != rev:
        sys.exit(f"API returned revision {meta.get('sha')}, expected {rev}")
    files = []
    for s in meta["siblings"]:
        path = s["rfilename"]
        if path in SKIP:
            continue
        if not path.endswith(ALLOWED):
            sys.exit(f"refusing file type: {path}")
        lfs = s.get("lfs")
        if lfs:
            sha, size = lfs["sha256"], lfs["size"]
        else:
            data = get(f"https://huggingface.co/{repo}/resolve/{rev}/{path}")
            sha, size = hashlib.sha256(data).hexdigest(), len(data)
            if s.get("size") not in (None, size):
                sys.exit(f"size mismatch for {path}")
        files.append({"path": path, "size": size, "sha256": sha})
    config = json.loads(get(f"https://huggingface.co/{repo}/resolve/{rev}/config.json"))
    entry = {
        "id": mid,
        "display_name": name,
        "family": family,
        "architecture": config.get("model_type", "unknown"),
        "parameters": "2B",
        "quantization": quant,
        "source": {"kind": "huggingface", "repo": repo, "revision": rev},
        "license": (meta.get("cardData") or {}).get("license", "unknown"),
        "pegoles_min_version": "0.1.0",
        "recommended_min_ram_gb": None,
        "published": published,
        "files": sorted(files, key=lambda f: f["path"]),
    }
    print(json.dumps(entry, indent=2))


if __name__ == "__main__":
    main()
