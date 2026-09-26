#!/usr/bin/env python3
"""Summarize local_bench run files into the committed results.

usage: summarize.py <run.json>... > benchmarks/local-models/results.json

Each input is one model/configuration file written by
crates/pegoles-agent/examples/local_bench.rs. The output keeps only
aggregate metrics and per-task outcomes (no screenshots, no raw traces).
"""

import json
import statistics
import sys


def pct(values, p):
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    return values[round((len(values) - 1) * p)]


def gb(v):
    return None if v is None else round(v / 1e9, 2)


def code_for(seed):
    return (seed * 7919 + 1234) % 9000 + 1000


def fix_answer_check(r, seed=7):
    # Runs recorded before the harness compared digits only marked a
    # correct spaced answer ("3 6 6 7") as wrong; re-derive from the
    # recorded final answer.
    if r["task"] == "read_code" and r["end"] == "Completed":
        digits = "".join(ch for ch in r["summary"] if ch.isdigit())
        ok = str(code_for(seed)) in digits
        r["success"] = ok
        r["clean_completion"] = ok
    return r


def summarize(path):
    d = json.load(open(path))
    res = [fix_answer_check(r) for r in d["results"]]
    n = len(res)
    traces = [t for r in res for t in r["traces"]]
    infer = [t["wall_ms"] for t in traces]
    first = [t["first_token_ms"] for t in traces]
    ground = [r["grounding"] for r in res if r.get("grounding")]
    by_cat = {}
    for r in res:
        c = by_cat.setdefault(r["category"], [0, 0])
        c[0] += r["success"]
        c[1] += 1
    actions = sum(r["model_actions"] for r in res)
    executed = sum(r["actions_executed"] for r in res)
    spec = d["model"]
    life = d.get("memory_lifecycle", {})
    return {
        "model": spec["id"],
        "family": spec["family"],
        "quantization": spec["quantization"],
        "source": spec["source"],
        "disk_bytes": sum(f["size"] for f in spec["files"]),
        "observe_long_side": d["long_side"],
        "history_images": d["history_images"],
        "tasks": n,
        "task_completion": round(sum(r["success"] for r in res) / n, 3),
        "clean_completion": round(sum(r.get("clean_completion", False) for r in res) / n, 3),
        "by_category": {k: f"{v[0]}/{v[1]}" for k, v in sorted(by_cat.items())},
        "action_success": round(executed / actions, 3) if actions else None,
        "actions_per_task": round(actions / n, 2),
        "observations_per_task": round(sum(r["observations"] for r in res) / n, 2),
        "inferences": len(traces),
        "invalid_output_rate": round(sum(1 for t in traces if "Err" in t["parsed"]) / max(1, len(traces)), 3),
        "policy_blocked_actions": sum(r["actions_blocked"] for r in res),
        "loop_brake_tasks": sum(r["loop_brake"] for r in res),
        "grounding_first_click_hit": f"{sum(g.get('hit', False) for g in ground)}/{len(ground)}",
        "grounding_median_distance_px": pct([g.get("distance_px") for g in ground], 0.5),
        "inference_ms_p50": pct(infer, 0.5),
        "inference_ms_p95": pct(infer, 0.95),
        "first_token_ms_p50": pct(first, 0.5),
        "prompt_tokens_p50": pct([t["prompt_tokens"] for t in traces], 0.5),
        "time_to_first_action_ms_p50": pct([r["time_to_first_action_ms"] for r in res], 0.5),
        "task_duration_ms_p50": pct([r["duration_ms"] for r in res], 0.5),
        "model_load_ms": (d.get("load") or {}).get("load_ms"),
        "verify_ms": d.get("verify_ms"),
        "memory_gb": {
            "worker_after_load": gb(d["after_load"]["worker"]),
            "worker_peak": gb(max(r["peak_worker_bytes"] for r in res)),
            "worker_after_tasks": gb(life.get("after_tasks", {}).get("worker")),
            "worker_after_unload": gb(life.get("after_unload", {}).get("worker")),
            "vm_idle": gb(d.get("vm_idle_footprint_bytes")),
            "vm_peak": gb(max(r["peak_vm_bytes"] for r in res)),
            "harness_idle": gb(d.get("harness_idle_footprint_bytes")),
            "system_used_before_load": gb(d["host"].get("system_used_before_bytes")),
            "system_used_peak": gb(max(r["peak_system_used_bytes"] for r in res)),
            "system_used_after_worker_exit": gb(life.get("after_worker_exit", {}).get("system_used")),
            "max_pressure_level": max(r["max_pressure_level"] for r in res),
        },
        "results": [
            {
                "task": r["task"],
                "category": r["category"],
                "success": r["success"],
                "clean": r.get("clean_completion", False),
                "turns": r["turns"],
                "actions": r["model_actions"],
                "duration_ms": r["duration_ms"],
                "end": r["end"],
                "note": r["summary"][:140],
            }
            for r in res
        ],
    }


def markdown(configs):
    rows = [
        ("goals achieved", lambda c: f"{round(c['task_completion'] * c['tasks'])}/{c['tasks']}"),
        ("clean completion", lambda c: f"{round(c['clean_completion'] * c['tasks'])}/{c['tasks']}"),
        ("first-click hit (grounding)", lambda c: c["grounding_first_click_hit"]),
        ("median click error px", lambda c: c["grounding_median_distance_px"]),
        ("invalid output rate", lambda c: f"{c['invalid_output_rate']:.1%}"),
        ("loop-brake stops", lambda c: c["loop_brake_tasks"]),
        ("actions per task", lambda c: c["actions_per_task"]),
        ("step latency p50 / p95 s", lambda c: f"{c['inference_ms_p50'] / 1000:.2f} / {c['inference_ms_p95'] / 1000:.2f}"),
        ("first token p50 s", lambda c: f"{c['first_token_ms_p50'] / 1000:.2f}"),
        ("time to first action p50 s", lambda c: f"{c['time_to_first_action_ms_p50'] / 1000:.1f}"),
        ("task duration p50 s", lambda c: f"{c['task_duration_ms_p50'] / 1000:.1f}"),
        ("disk GB", lambda c: f"{c['disk_bytes'] / 1e9:.2f}"),
        ("worker after load / steady / peak GB", lambda c: f"{c['memory_gb']['worker_after_load']} / {c['memory_gb']['worker_after_tasks']} / {c['memory_gb']['worker_peak']}"),
        ("load / verify s", lambda c: f"{c['model_load_ms'] / 1000:.1f} / {c['verify_ms'] / 1000:.1f}"),
    ]
    head = "| | " + " | ".join(f"{c['model']} @{c['observe_long_side']}" for c in configs) + " |"
    out = [head, "|---" * (len(configs) + 1) + "|"]
    for name, f in rows:
        out.append(f"| {name} | " + " | ".join(str(f(c)) for c in configs) + " |")
    cats = sorted({k for c in configs for k in c["by_category"]})
    for k in cats:
        out.append(f"| {k} | " + " | ".join(c["by_category"].get(k, "-") for c in configs) + " |")
    return "\n".join(out)


def main():
    args = sys.argv[1:]
    md = "--md" in args
    configs = [summarize(p) for p in args if p != "--md"]
    if md:
        print(markdown(configs))
    else:
        print(json.dumps({"schema": 1, "configs": configs}, indent=2))


if __name__ == "__main__":
    main()
