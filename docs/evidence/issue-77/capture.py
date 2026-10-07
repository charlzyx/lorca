"""Capture only the synthetic native windows exposed by PlaybookScreenshotTests."""

import argparse
import json
from pathlib import Path
import subprocess
import time
from PIL import Image

parser = argparse.ArgumentParser()
parser.add_argument("--ready-directory", type=Path, required=True)
parser.add_argument("--output-directory", type=Path, required=True)
parser.add_argument("--only", help="Capture one named state in an isolated fixture-test process")
args = parser.parse_args()
args.output_directory.mkdir(parents=True, exist_ok=True)

for name in (
    "draft-review", "bundled-reference", "bundled-script", "correction-history",
    "workflow-capture", "correction-evidence",
):
    if args.only and name != args.only:
        continue
    ready = args.ready_directory / f"ready-{name}.json"
    deadline = time.monotonic() + 45
    while not ready.exists() and time.monotonic() < deadline:
        time.sleep(0.05)
    info = json.loads(ready.read_text())
    windows = json.loads(subprocess.check_output([
        "cua-driver", "list_windows", json.dumps({"pid": info["pid"]}),
    ]))["windows"]
    assert any(window["window_id"] == info["window_id"]
               and window["title"] == f"Lorca #77 fixture · {name}" for window in windows)
    output = args.output_directory.resolve() / f"{name}.png"
    deadline = time.monotonic() + 20
    while True:
        result = json.loads(subprocess.check_output([
            "cua-driver", "get_window_state", json.dumps({
                "pid": info["pid"], "window_id": info["window_id"], "timeout_ms": 1000,
                "screenshot_out_file": str(output),
            }),
        ]))
        assert result.get("screenshot_frame_valid") and output.is_file()
        document = next((d for d in info["documents"] if d["text_length"] and not d["hidden"]), None)
        ink = 0
        if document:
            rect = document["clip_rect"]
            scale = result["screenshot_width"] / info["width"]
            bounds = (int((rect["x"] + 8) * scale), int((info["height"] - rect["y"] - rect["height"] + 8) * scale),
                      int((rect["x"] + rect["width"] - 8) * scale), int((info["height"] - rect["y"] - 8) * scale))
            # Inspect the native pixels without altering them. Read-only text can paint after
            # the window chrome; do not acknowledge a valid but still-empty document frame.
            with Image.open(output) as screenshot:
                ink = sum(max(pixel) < 160 for pixel in screenshot.convert("RGB").crop(bounds).getdata())
        if not document or ink > 100:
            break
        assert time.monotonic() < deadline, f"Native document did not paint: {name}"
        time.sleep(0.2)
    print(json.dumps({"name": name, "width": result["screenshot_width"],
                      "height": result["screenshot_height"], "document_ink": ink}), flush=True)
    (args.ready_directory / f"captured-{name}").touch()
