#!/usr/bin/env python3
"""0059 actual framed-backend completion pressure observations.

Run a production build (not Cargo's test-support executable). These are external
input-to-semantic-view timings, not terminal paint or isolated enqueue latency.
The unchanged bench_ui_input_frame.py supplies the same-profile before/after
comparison; stage-separated and real-PTY observations are separate evidence.
"""
from __future__ import annotations

import argparse
import json
import platform
import time
from pathlib import Path

from bench_worker import cpu_model, percentiles
from completion_driver import CompletionEditor, control_key, key


def report(editor: CompletionEditor, samples: list[float]) -> dict:
    editor.resources()
    publications = [snapshot["publication"] for snapshot in editor.snapshots]
    work = [snapshot["work"] for snapshot in editor.snapshots if snapshot.get("work")]
    started = time.perf_counter_ns()
    retired = editor.shutdown()
    return {
        "input_to_semantic_view_ms": percentiles(samples) if samples else None,
        "raw_ms": samples,
        "rebased_actions": editor.rebased_actions,
        "publication_charged_high_water_bytes": max((value["high_water_bytes"] for value in publications), default=0),
        "observed_native_snapshot_destructions": max((value["destroyed"] for value in publications), default=0),
        "word_work_last_observed": work[-1] if work else None,
        "editor_sampled_peak_rss_kib": editor.editor_peak_kib,
        "worker_sampled_peak_rss_kib": editor.worker_peak_kib,
        "completion_threads_high_water": editor.completion_threads_peak,
        "shutdown_and_owned_worker_exit_ms": (time.perf_counter_ns() - started) / 1_000_000,
        **retired,
    }


def word_case(binary: Path, source: str, mode: str, iterations: int) -> dict:
    with CompletionEditor(binary, source, enabled=mode != "disabled", automatic=mode == "automatic") as editor:
        editor.keys("G$a")
        started = time.perf_counter_ns()
        editor.keys("e")  # Every fixture ends in r; this is the automatic threshold.
        if mode != "automatic":
            if editor.completion().get("query") is not None or editor.resources()["completion_threads"] not in (None, 0):
                raise RuntimeError(f"{mode} typing started a completion query/worker")
            editor.act([control_key(" ")])
        if mode == "disabled":
            if editor.completion().get("query") is not None:
                raise RuntimeError("disabled manual invocation started completion")
            first = None
        else:
            editor.wait(lambda: editor.provider("buf") == "ready", "first useful source words")
            first = (time.perf_counter_ns() - started) / 1_000_000
        samples = []
        for _ in range(iterations):
            samples.append(editor.keys("s"))
            samples.append(editor.act([key("Backspace")]))
            editor.resources()
        latest = time.perf_counter_ns()
        if mode != "disabled":
            editor.wait(lambda: editor.provider("buf") == "ready", "latest query after sustained edits")
            state = editor.completion()
            if state["source"]["revision"] != editor.peer.panes[0]["revision"]:
                raise RuntimeError("word result was relabelled against a different source revision")
            if not any(row["label"] == "response" for row in editor.menu()["rows"]):
                raise RuntimeError("latest source words do not match the restored prefix")
        latest_ready = (time.perf_counter_ns() - latest) / 1_000_000 if mode != "disabled" else None
        cancel = editor.act([control_key("e")]) if mode != "disabled" else None
        if editor.completion().get("query") is not None:
            raise RuntimeError("dismissal retained obsolete query authority")
        editor.keys("s")
        if mode == "manual" and editor.completion().get("query") is not None:
            raise RuntimeError("closed manual-only completion restarted automatically")
        return {"mode": mode, "source_bytes": len(source.encode()), "first_useful_ms": first,
                "latest_query_after_typing_ms": latest_ready,
                "logical_cancel_to_view_ms": cancel, **report(editor, samples)}


def slow_case(binary: Path, mode: str, iterations: int) -> dict:
    source = "// fixture\nresponse result\nre\n"
    with CompletionEditor(binary, source, server=mode) as editor:
        editor.keys("G$a")
        started = time.perf_counter_ns()
        editor.act([control_key(" ")])
        editor.wait(lambda: editor.provider("buf") == "ready", "words while language is held")
        first_words = (time.perf_counter_ns() - started) / 1_000_000
        editor.control("await", counter="queries", at_least=1)
        editor.act([control_key("n")])
        chosen = next(row for row in editor.menu()["rows"] if row["index"] == editor.menu()["selected"])
        if chosen["source"] != "buf":
            raise RuntimeError("a held language response supplied an invented candidate")
        editor.keys("s")
        editor.control("await", counter="queries", at_least=2)
        samples = [editor.act([key("Backspace")])]
        for _ in range(iterations):
            samples += [editor.keys("s"), editor.act([key("Backspace")])]
            editor.resources()
        wire = editor.control("await", counter="cancels", at_least=2)
        if wire["counts"]["queries"] != 2 or len(wire["held"]) != 2:
            raise RuntimeError("cancelled sent requests lost their two physical ownership slots")
        if mode == "hold":
            editor.control("release")
            newest = editor.control("await", counter="queries", at_least=3)
            editor.control("release")
            editor.wait(lambda: editor.provider("lsp") == "ready", "newest language result after capacity returns")
            editor.wait(lambda: editor.provider("buf") == "ready", "newest source words")
            selected = editor.menu()["selected"]
            choice = next(row for row in editor.menu()["rows"] if row["index"] == selected)
            if (choice["source"], choice["label"]) != ("buf", chosen["label"]):
                raise RuntimeError("a late language window moved deliberate source-word selection")
            wire = newest
        cancel = editor.act([control_key("e")])
        editor.act([key("Escape")])
        editor.keys("gd")
        editor.wait(lambda: editor.peer.state["line"] == 1, "ordinary LSP navigation under completion pressure")
        return {"mode": mode, "first_independent_words_ms": first_words,
                "logical_cancel_to_view_ms": cancel, "wire": wire, **report(editor, samples)}


def resolve_case(binary: Path) -> dict:
    source = "// fixture\nresponse result\nre\n"
    with CompletionEditor(binary, source, server="resolve-hold") as editor:
        editor.keys("G$a")
        editor.act([control_key(" ")])
        editor.wait(lambda: editor.provider("lsp") == "ready", "language candidates")
        editor.act([control_key("n")])
        editor.control("await", counter="resolves", at_least=1)
        editor.act([control_key("y")])
        typed = editor.keys("s")
        editor.control("release", kind="resolve")
        editor.act([control_key("e")])
        editor.act([key("Escape")])
        editor.keys("gd")
        editor.wait(lambda: editor.peer.state["line"] == 1, "barrier after stale resolve")
        lines = editor.peer.panes[0]["lines"]
        if "\n".join(lines) != source.replace("\nre\n", "\nres\n"):
            raise RuntimeError("a stale resolve/acceptance inserted an import or completion after typing")
        return {"mode": "resolve-hold", "wire": editor.control(), **report(editor, [typed])}


def oversized_case(binary: Path, mode: str) -> dict:
    with CompletionEditor(binary, "// fixture\nresponse result\nre\n", server=mode) as editor:
        editor.keys("G$a")
        started = time.perf_counter_ns()
        editor.act([control_key(" ")])
        editor.wait(lambda: editor.provider("buf") == "ready", "words beside oversized server data")
        editor.wait(lambda: editor.provider("lsp") in ("ready", "failed", "unavailable"), "bounded oversized response settlement")
        elapsed = (time.perf_counter_ns() - started) / 1_000_000
        if mode == "oversized" and (not editor.menu()["limited"] or editor.menu()["count"] > 256):
            raise RuntimeError("oversized candidates bypassed the bounded/truncated window")
        if mode == "oversized-frame" and editor.provider("lsp") == "ready":
            raise RuntimeError("an over-bound raw response reached completion decoding")
        editor.act([control_key("e")])
        return {"mode": mode, "settlement_ms": elapsed, **report(editor, [])}


def reopen_case(binary: Path, iterations: int) -> dict:
    source = "response result\nre\n"
    with CompletionEditor(binary, source, automatic=False) as editor:
        (editor.root / "second.txt").write_text("reference reloaded\nre\n")
        samples = []
        for index in range(iterations):
            name = "second.txt" if index % 2 == 0 else "input.txt"
            samples.append(editor.act([key(char) for char in ":e " + name] + [key("Enter")]))
            editor.wait(lambda: editor.peer.panes[0]["lines"][0] == ("reference reloaded" if index % 2 == 0 else "response result"), "source switch")
            editor.keys("G$a")
            editor.act([control_key("n")])
            editor.wait(lambda: editor.provider("buf") == "ready", "reopened source index")
            labels = {row["label"] for row in editor.menu()["rows"]}
            expected = {"reference", "reloaded"} if index % 2 == 0 else {"response", "result"}
            if labels != expected:
                raise RuntimeError(f"words crossed source ownership: {labels} != {expected}")
            editor.resources()
            editor.act([key("Escape")])
        return {"mode": "reopen", "source_switches": iterations, **report(editor, samples)}


def acceptance_case(binary: Path) -> dict:
    source = "// fixture\nresponse result\nre\n"
    expected = "// represented completion import\n" + source.replace("\nre\n", "\nresponse\n")
    with CompletionEditor(binary, source, server="import") as editor:
        editor.keys("G$a")
        editor.act([control_key(" ")])
        editor.wait(lambda: editor.provider("lsp") == "ready", "language candidates")
        editor.act([control_key("n")])
        editor.act([control_key("y")])
        editor.wait(lambda: not editor.menu()
                    and editor.peer.panes[0]["lines"][0] == "// represented completion import",
                    "represented primary edit and import")
        editor.act([key("Escape")])
        editor.act([key(char) for char in ":w"] + [key("Enter")])
        editor.wait(lambda: editor.file.read_text() == expected, "save after invalid formatter range")
        wire = editor.control()
        if wire["counts"]["resolves"] != 1 or wire["counts"]["formats"] != 1:
            raise RuntimeError("acceptance did not exercise both lazy resolve and invalid mutation coordinates")
        editor.keys("u")
        editor.wait(lambda: "\n".join(editor.peer.panes[0]["lines"]) == source, "coherent import undo")
        result = report(editor, [])
        return {"mode": "import", "wire": wire, "opaque_data_roundtripped": True,
                "primary_and_import_undo_together": True, "invalid_formatter_did_not_mutate": True,
                **result}


def measure(binary: Path, profile: str, iterations: int = 32, case: str = "all") -> dict:
    cases = {}
    if case in ("all", "words"):
        small = "response result reusable return_value\n\nr\n"
        for mode in ("disabled", "manual", "automatic"):
            cases[mode] = word_case(binary, small, mode, iterations)
        cases["large"] = word_case(binary, "response result padding\n" * 700_000 + "r\n", "automatic", iterations)
        cases["line_1mib"] = word_case(binary, "response result\n" + "padding " * 131_072 + "r\n", "automatic", iterations)
        cases["unique_words"] = word_case(binary, "response result\n" + " ".join(f"word{i:08}" for i in range(90_000)) + "\nr\n", "automatic", iterations)
    if case in ("all", "slow"):
        for mode in ("hold", "ignore"):
            cases[mode] = slow_case(binary, mode, iterations)
    if case in ("all", "resolve"):
        cases["resolve"] = resolve_case(binary)
    if case in ("all", "oversized"):
        for mode in ("oversized", "oversized-frame"):
            cases[mode] = oversized_case(binary, mode)
    if case in ("all", "reopen"):
        cases["reopen"] = reopen_case(binary, iterations)
    if case in ("all", "acceptance"):
        cases["acceptance"] = acceptance_case(binary)
    return {
        "profile": profile, "platform": {"machine": platform.machine(), "kernel": platform.release(), "cpu": cpu_model()},
        "method": "actual framed UI input to semantic view; sampled RSS, charged native retention and controlled wire ownership",
        "iterations": iterations, "cases": cases}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--profile", choices=("development", "release-musl-stripped", "release-gnu-native", "release-macos-native"), required=True)
    parser.add_argument("--iterations", type=int, default=32)
    parser.add_argument("--case", choices=("all", "words", "slow", "resolve", "oversized", "reopen", "acceptance"), default="all")
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("iterations must be positive")
    print(json.dumps(measure(args.binary.resolve(), args.profile, args.iterations, args.case), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
