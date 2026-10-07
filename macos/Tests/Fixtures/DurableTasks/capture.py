#!/usr/bin/env python3
"""Capture actual AppKit task views with synthetic data through an isolated local fixture API.

This is a test-only runner. It never starts a Lorca CLI, connects a provider, or captures the
desktop. Images are NSView bitmap captures from the existing production controllers.
Requires the Python websockets package and the repository's normal Swift build prerequisites.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path

from websockets.asyncio.server import serve


async def run(destination: Path) -> int:
    async def handle(socket):
        async for raw in socket:
            request = json.loads(raw)
            method = request.get("method")
            if method == "tasks.get":
                reply = {"id": request["id"], "result": request["params"]["fixture"]}
            elif method == "tasks.update":
                reply = {"id": request["id"], "error": {"message": "Task revision conflict: expected 7, current 8. Read the task again."}}
            else:
                reply = {"id": request["id"], "result": {}}
            await socket.send(json.dumps(reply))

    async with serve(handle, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        assert port not in (4862, 4863)
        environment = os.environ.copy()
        environment.update(LORCA_MOCK="1", LORCA_PORT=str(port), LORCA_TASK_CAPTURE_DIR=str(destination))
        repository = Path(__file__).resolve().parents[4]
        process = await asyncio.create_subprocess_exec(
            "swift", "test", "--package-path", "macos", "--filter", "DurableTaskCaptureTests",
            cwd=repository, env=environment,
        )
        return await process.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("evidence/issue-72"))
    arguments = parser.parse_args()
    raise SystemExit(asyncio.run(run(arguments.output.resolve())))
