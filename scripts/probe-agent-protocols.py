#!/usr/bin/env python3
import asyncio
import json
import tempfile


async def exchange(argv, request, request_id, accepted, workdir):
    process = await asyncio.create_subprocess_exec(
        *argv, cwd=workdir, stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
    )
    ready = False
    try:
        assert process.stdin is not None and process.stdout is not None
        process.stdin.write((json.dumps(request) + "\n").encode())
        await process.stdin.drain()
        async with asyncio.timeout(30):
            while True:
                line = await process.stdout.readline()
                if not line:
                    raise RuntimeError("protocol exited before metadata response")
                try:
                    frame = json.loads(line)
                except (ValueError, UnicodeDecodeError) as error:
                    raise RuntimeError("non-JSON protocol frame") from error
                if not isinstance(frame, dict):
                    raise RuntimeError("protocol frame is not an object")
                ready = ready or frame.get("type") == "ready"
                if frame.get("id") == request_id:
                    if not accepted(frame):
                        raise RuntimeError("metadata response rejected")
                    return {"ok": True, "ready": ready}
    finally:
        if process.returncode is None:
            process.terminate()
        try:
            await asyncio.wait_for(process.wait(), 5)
        except asyncio.TimeoutError:
            process.kill()
            await process.wait()


async def main():
    with tempfile.TemporaryDirectory(prefix="conex-agent-probe-") as workdir:
        omp = await exchange(
            ["omp", "--mode", "rpc", "--no-session", "--no-extensions",
             "--no-skills", "--no-rules", "--no-context-files", "--no-lsp",
             "--cwd", workdir],
            {"id": "metadata", "type": "get_state"}, "metadata",
            lambda frame: frame.get("type") == "response"
            and frame.get("command") == "get_state"
            and frame.get("success") is True,
            workdir,
        )
        if not omp["ready"]:
            raise RuntimeError("OMP did not announce ready")
        codex = await exchange(
            ["codex", "app-server"],
            {"id": 1, "method": "initialize", "params": {
                "clientInfo": {"name": "conex-probe", "version": "0.0.0"},
                "capabilities": {"experimentalApi": False},
            }}, 1,
            lambda frame: "error" not in frame
            and isinstance(frame.get("result"), dict)
            and "userAgent" in frame["result"],
            workdir,
        )
        print(json.dumps({"omp": omp, "codex": {"ok": codex["ok"]}}))


if __name__ == "__main__":
    asyncio.run(main())
