#!/usr/bin/env python3
"""Emit a catalog entry for a model converted on this machine.

usage: pin_local_model.py <dir> <id> <display_name> <family> <quantization> \
         <upstream_repo> <upstream_revision> <recipe>

The entry pins every file's size and SHA-256; the store imports the
directory only if its bytes match (`models import <id> <dir>`). Such an
entry is not downloadable: it documents a benchmark candidate, it is not
a distribution channel. BUILD TIME ONLY.
"""

import hashlib
import json
import os
import sys

ALLOWED = (".safetensors", ".json", ".txt", ".jinja", ".model", ".tiktoken")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    d, mid, name, family, quant, repo, rev, recipe = sys.argv[1:9]
    files = []
    for fn in sorted(os.listdir(d)):
        p = os.path.join(d, fn)
        if not os.path.isfile(p) or fn.startswith(".") or fn == "README.md":
            continue
        if not fn.endswith(ALLOWED):
            sys.exit(f"refusing file type: {fn}")
        files.append({"path": fn, "size": os.path.getsize(p), "sha256": sha256(p)})
    config = json.load(open(os.path.join(d, "config.json")))
    print(json.dumps({
        "id": mid, "display_name": name, "family": family,
        "architecture": config.get("model_type", "unknown"), "parameters": "2B",
        "quantization": quant,
        "source": {"kind": "local_conversion", "upstream_repo": repo,
                   "upstream_revision": rev, "recipe": recipe},
        "license": "apache-2.0", "pegoles_min_version": "0.1.0",
        "recommended_min_ram_gb": None, "published": "local", "files": files,
    }, indent=2))


if __name__ == "__main__":
    main()
