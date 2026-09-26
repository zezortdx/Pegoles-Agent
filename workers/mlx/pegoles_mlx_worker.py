#!/usr/bin/env python3
"""Pegoles local inference worker (MLX / mlx-vlm, Apple Silicon).

A persistent host-side process that keeps one vision-language model
loaded and answers generation requests. It is an inference engine only:

* It never executes, evaluates or interprets model output. Text goes back
  to the Pegoles runtime, which parses it strictly into typed actions and
  sends every action through the deterministic policy.
* It never touches the network. The parent starts it with a cleared
  environment and offline flags, and socket connections are refused
  in-process as a second layer.
* It loads weights only from a local directory the parent has verified
  against a pinned manifest; remote code in model repositories is never
  trusted.

Protocol (version 1): one JSON object per line on stdin, one JSON reply
per line on stdout. stderr carries diagnostics only. Replies use a
private copy of the original stdout; fd 1 and sys.stdout are pointed at
stderr, so stray output from any library cannot corrupt the protocol.

  -> {"v":1,"id":7,"op":"generate", ...}
  <- {"v":1,"id":7,"ok":true, ...}   or   {"v":1,"id":7,"ok":false,"error":{...}}

A "cancel" request ({"op":"cancel","target":7}) is handled out of band
by the reader thread and stops the running generation between tokens.
"""

import base64
import gc
import io
import json
import os
import queue
import resource
import socket
import sys
import threading
import time
import traceback

PROTOCOL_VERSION = 1
WORKER_VERSION = "0.1.0"
MAX_LINE_BYTES = 24 * 1024 * 1024
MAX_IMAGE_BYTES = 16 * 1024 * 1024
MAX_IMAGE_SIDE = 4096
MAX_PROMPT_CHARS = 64 * 1024
MAX_MESSAGES = 16
MAX_TOKENS_CAP = 2048
MAX_TEXT_CHARS = 32 * 1024
MAX_ERROR_CHARS = 600


def _refuse_network(*_args, **_kwargs):
    raise OSError("network access is disabled in the Pegoles inference worker")


# Second layer after the parent's offline environment: nothing in this
# process may open an outbound connection.
socket.socket.connect = _refuse_network
socket.socket.connect_ex = _refuse_network
socket.create_connection = _refuse_network
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["HF_DATASETS_OFFLINE"] = "1"
os.environ["TOKENIZERS_PARALLELISM"] = "false"

def _private_protocol_stream():
    """Move the reply channel off fd 1 before any library is imported: a
    print() or a native write to stdout then lands on stderr (diagnostics)
    instead of the host reading it as a malformed reply."""
    proto = os.fdopen(os.dup(1), "w", encoding="utf-8")
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return proto


_proto = _private_protocol_stream()
_out_lock = threading.Lock()


def log(msg):
    sys.stderr.write("[pegoles-mlx-worker] " + str(msg)[:2000] + "\n")
    sys.stderr.flush()


def send(obj):
    line = json.dumps(obj, ensure_ascii=True, separators=(",", ":"), allow_nan=False)
    with _out_lock:
        _proto.write(line + "\n")
        _proto.flush()


def reply(req_id, **fields):
    send({"v": PROTOCOL_VERSION, "id": req_id, "ok": True, **fields})


def fail(req_id, kind, message):
    send(
        {
            "v": PROTOCOL_VERSION,
            "id": req_id,
            "ok": False,
            "error": {"kind": kind, "message": str(message)[:MAX_ERROR_CHARS]},
        }
    )


class BadRequest(Exception):
    pass


def _forbid_remote_code():
    """Model repositories may ship Python; it must never run here."""
    try:
        import transformers.dynamic_module_utils as dmu

        def refuse(*_a, **_k):
            raise BadRequest("model requires remote code, which Pegoles never runs")

        dmu.get_class_from_dynamic_module = refuse
        dmu.get_cached_module_file = refuse
    except Exception as exc:  # pragma: no cover - defensive
        log(f"could not install remote-code guard: {exc}")


class Worker:
    def __init__(self):
        self.model = None
        self.processor = None
        self.model_dir = None
        self.current = None  # (id, threading.Event)
        self.current_lock = threading.Lock()
        import mlx.core as mx  # noqa: F401  (fail fast if MLX is missing)

        self.mx = mx

    # --- ops -----------------------------------------------------------

    def hello(self, req):
        import mlx.core as mx
        import mlx_vlm

        reply(
            req["id"],
            worker="pegoles-mlx",
            worker_version=WORKER_VERSION,
            protocol=PROTOCOL_VERSION,
            mlx=getattr(mx, "__version__", "unknown"),
            mlx_vlm=getattr(mlx_vlm, "__version__", "unknown"),
            metal=bool(mx.metal.is_available()),
            python=sys.version.split()[0],
        )

    def load(self, req):
        model_dir = req.get("model_dir")
        if not isinstance(model_dir, str) or not os.path.isabs(model_dir):
            raise BadRequest("model_dir must be an absolute path")
        if not os.path.isfile(os.path.join(model_dir, "config.json")):
            raise BadRequest("model_dir has no config.json")
        self.unload_model()
        from mlx_vlm import load

        mx = self.mx
        cache_limit = req.get("cache_limit_bytes")
        if cache_limit is not None:
            if not isinstance(cache_limit, int) or not (0 <= cache_limit <= 64 * 1024**3):
                raise BadRequest("cache_limit_bytes out of range")
            # Freed Metal buffers above this are returned to the system
            # instead of being kept for reuse (lower resident footprint).
            mx.set_cache_limit(cache_limit)
        mx.reset_peak_memory()
        t0 = time.monotonic()
        model, processor = load(model_dir, trust_remote_code=False)
        load_ms = (time.monotonic() - t0) * 1000.0
        self.model, self.processor, self.model_dir = model, processor, model_dir
        reply(req["id"], load_ms=round(load_ms, 1), **self.memory())

    def unload_model(self):
        self.model = None
        self.processor = None
        self.model_dir = None
        gc.collect()
        self.mx.clear_cache()

    def unload(self, req):
        self.unload_model()
        reply(req["id"], **self.memory())

    def stats(self, req):
        if req.get("reset_peak"):
            self.mx.reset_peak_memory()
        reply(req["id"], loaded=self.model is not None, **self.memory())

    def memory(self):
        mx = self.mx
        # ru_maxrss is bytes on macOS.
        return {
            "active_bytes": int(mx.get_active_memory()),
            "peak_bytes": int(mx.get_peak_memory()),
            "cache_bytes": int(mx.get_cache_memory()),
            "max_rss_bytes": int(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss),
        }

    def generate(self, req):
        if self.model is None:
            raise BadRequest("no model is loaded")
        messages = req.get("messages")
        if not isinstance(messages, list) or not (1 <= len(messages) <= MAX_MESSAGES):
            raise BadRequest("messages must be a non-empty list")
        images_b64 = req.get("images") or []
        if not isinstance(images_b64, list) or len(images_b64) > 2:
            raise BadRequest("at most two images per request")
        prep = req.get("image_prep") or [{}] * len(images_b64)
        if not isinstance(prep, list) or len(prep) != len(images_b64):
            raise BadRequest("image_prep must match images")
        max_tokens = int(req.get("max_tokens", 256))
        if not (1 <= max_tokens <= MAX_TOKENS_CAP):
            raise BadRequest("max_tokens out of range")
        temperature = float(req.get("temperature", 0.0))
        if not (0.0 <= temperature <= 2.0):
            raise BadRequest("temperature out of range")

        chat, n_image_slots = self.sanitize_messages(messages)
        if n_image_slots != len(images_b64):
            raise BadRequest("image placeholders do not match images")

        t0 = time.monotonic()
        images = [decode_image(b, p) for b, p in zip(images_b64, prep)]
        t_img = time.monotonic()

        prompt = self.processor.apply_chat_template(
            chat, tokenize=False, add_generation_prompt=True
        )
        if len(prompt) > MAX_PROMPT_CHARS * 2:
            raise BadRequest("prompt too long")

        from mlx_vlm import stream_generate

        cancel = threading.Event()
        with self.current_lock:
            self.current = (req["id"], cancel)
        text_parts = []
        n_chars = 0
        last = None
        first_token_ms = None
        finish = "length"
        try:
            self.mx.reset_peak_memory()
            for res in stream_generate(
                self.model,
                self.processor,
                prompt,
                image=images if images else None,
                max_tokens=max_tokens,
                temperature=temperature,
            ):
                if first_token_ms is None:
                    first_token_ms = (time.monotonic() - t_img) * 1000.0
                last = res
                if res.text:
                    text_parts.append(res.text)
                    n_chars += len(res.text)
                if cancel.is_set():
                    finish = "cancelled"
                    break
                if n_chars > MAX_TEXT_CHARS:
                    finish = "text_limit"
                    break
                if res.finish_reason:
                    finish = res.finish_reason
        finally:
            with self.current_lock:
                self.current = None
        t_end = time.monotonic()
        text = "".join(text_parts)[:MAX_TEXT_CHARS]
        reply(
            req["id"],
            text=text,
            finish=finish,
            prompt_tokens=int(getattr(last, "prompt_tokens", 0) or 0),
            generation_tokens=int(getattr(last, "generation_tokens", 0) or 0),
            prompt_tps=round(float(getattr(last, "prompt_tps", 0.0) or 0.0), 2),
            generation_tps=round(float(getattr(last, "generation_tps", 0.0) or 0.0), 2),
            image_sizes=[list(im.size) for im in images],
            timings_ms={
                "image": round((t_img - t0) * 1000.0, 1),
                "first_token": round(first_token_ms or 0.0, 1),
                "generate": round((t_end - t_img) * 1000.0, 1),
                "total": round((t_end - t0) * 1000.0, 1),
            },
            **self.memory(),
        )

    def sanitize_messages(self, messages):
        """Rebuild the chat from a strict subset: roles and text/image parts."""
        out = []
        slots = 0
        total = 0
        for m in messages:
            if not isinstance(m, dict):
                raise BadRequest("message must be an object")
            role = m.get("role")
            if role not in ("system", "user", "assistant"):
                raise BadRequest("bad role")
            content = m.get("content")
            if not isinstance(content, list) or not content:
                raise BadRequest("content must be a non-empty list")
            parts = []
            for p in content:
                kind = p.get("type") if isinstance(p, dict) else None
                if kind == "text":
                    t = p.get("text")
                    if not isinstance(t, str):
                        raise BadRequest("text part must be a string")
                    total += len(t)
                    parts.append({"type": "text", "text": t})
                elif kind == "image":
                    slots += 1
                    parts.append({"type": "image"})
                else:
                    raise BadRequest("unknown content part")
            out.append({"role": role, "content": parts})
        if total > MAX_PROMPT_CHARS:
            raise BadRequest("prompt text too long")
        return out, slots

    def cancel(self, target):
        with self.current_lock:
            cur = self.current
        if cur is not None and cur[0] == target:
            cur[1].set()


def decode_image(b64, prep):
    from PIL import Image

    if not isinstance(b64, str) or len(b64) > MAX_IMAGE_BYTES * 4 // 3 + 8:
        raise BadRequest("image too large")
    raw = base64.b64decode(b64, validate=True)
    Image.MAX_IMAGE_PIXELS = MAX_IMAGE_SIDE * MAX_IMAGE_SIDE
    im = Image.open(io.BytesIO(raw))
    if im.format != "PNG":
        raise BadRequest("images must be PNG")
    w, h = im.size
    if not (1 <= w <= MAX_IMAGE_SIDE and 1 <= h <= MAX_IMAGE_SIDE):
        raise BadRequest("image dimensions out of range")
    im = im.convert("RGB")
    if not isinstance(prep, dict):
        raise BadRequest("image_prep entries must be objects")
    crop = prep.get("crop")
    if crop is not None:
        if not (isinstance(crop, list) and len(crop) == 4 and all(isinstance(v, int) for v in crop)):
            raise BadRequest("crop must be four integers")
        x0, y0, x1, y1 = crop
        if not (0 <= x0 < x1 <= w and 0 <= y0 < y1 <= h):
            raise BadRequest("crop outside the image")
        im = im.crop((x0, y0, x1, y1))
    size = prep.get("resize")
    if size is not None:
        if not (isinstance(size, list) and len(size) == 2 and all(isinstance(v, int) for v in size)):
            raise BadRequest("resize must be two integers")
        rw, rh = size
        if not (28 <= rw <= MAX_IMAGE_SIDE and 28 <= rh <= MAX_IMAGE_SIDE):
            raise BadRequest("resize out of range")
        if (rw, rh) != im.size:
            im = im.resize((rw, rh), Image.Resampling.LANCZOS)
    return im


def reader(worker, jobs):
    stdin = sys.stdin.buffer
    while True:
        line = stdin.readline(MAX_LINE_BYTES + 1)
        if not line:
            jobs.put(None)
            return
        if len(line) > MAX_LINE_BYTES:
            # Drain the rest of the oversized line, then report.
            while line and not line.endswith(b"\n"):
                line = stdin.readline(MAX_LINE_BYTES + 1)
            fail(None, "bad_request", "request line too long")
            continue
        try:
            req = json.loads(line)
        except Exception:
            fail(None, "bad_request", "request is not valid JSON")
            continue
        if not isinstance(req, dict) or req.get("v") != PROTOCOL_VERSION:
            fail(req.get("id") if isinstance(req, dict) else None, "bad_request", "unsupported protocol version")
            continue
        if not isinstance(req.get("id"), int):
            fail(None, "bad_request", "request id must be an integer")
            continue
        if req.get("op") == "cancel":
            target = req.get("target")
            if isinstance(target, int):
                worker.cancel(target)
            reply(req["id"])
            continue
        jobs.put(req)


def main():
    _forbid_remote_code()
    try:
        worker = Worker()
    except Exception as exc:
        fail(None, "runtime_missing", f"MLX is not available: {exc}")
        return 3
    jobs = queue.Queue()
    threading.Thread(target=reader, args=(worker, jobs), daemon=True).start()
    ops = {
        "hello": worker.hello,
        "load": worker.load,
        "unload": worker.unload,
        "stats": worker.stats,
        "generate": worker.generate,
    }
    while True:
        req = jobs.get()
        if req is None:
            return 0
        op = req.get("op")
        if op == "shutdown":
            reply(req["id"])
            return 0
        handler = ops.get(op)
        if handler is None:
            fail(req["id"], "bad_request", "unknown op")
            continue
        try:
            handler(req)
        except BadRequest as exc:
            fail(req["id"], "bad_request", exc)
        except MemoryError as exc:
            fail(req["id"], "out_of_memory", exc)
        except Exception as exc:  # report, keep serving
            msg = str(exc)
            kind = "out_of_memory" if "memory" in msg.lower() and "metal" in msg.lower() else "internal"
            log(traceback.format_exc(limit=6))
            fail(req["id"], kind, f"{type(exc).__name__}: {msg}")


if __name__ == "__main__":
    sys.exit(main())
