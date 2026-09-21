"""Synthetic management protocol worker; never loads a model or uses the network."""
import json
import pathlib
import sys
import time

mode = sys.argv[1]
root = pathlib.Path(sys.argv[sys.argv.index("--model-dir") + 1])
if mode == "process-running":
    pid = int((root / "started").read_text())
    if sys.platform == "win32":
        import ctypes
        api = ctypes.WinDLL("kernel32", use_last_error=True)
        api.OpenProcess.restype = ctypes.c_void_p
        api.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        api.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        api.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = api.OpenProcess(0x100000, False, pid)
        running = bool(handle) and api.WaitForSingleObject(handle, 0) == 258
        if handle:
            api.CloseHandle(handle)
    else:
        import os
        try:
            os.kill(pid, 0)
            running = True
        except ProcessLookupError:
            running = False
    print("running" if running else "exited")
    sys.exit()
if mode == "inference":
    from fake_sidecar import configure_result
    for line in sys.stdin:
        message = json.loads(line)
        if message["type"] == "configure":
            print(json.dumps({"id": message["id"], "type": "configure_result", "payload": configure_result(message)}), flush=True)
        else:
            (root / "started").write_text(str(__import__("os").getpid()))
            time.sleep(60)
    (root / "eof").write_text("eof")
    time.sleep(2)
    sys.exit()
if mode in ("real", "writer-probe", "prepare-probe"):
    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[3] / "sidecar" / "src"))
    from redactio_sidecar.model_manager import run_management, mutation_lock
    if mode == "real":
        run_management(sys.stdin.buffer, sys.stdout.buffer, root)
    else:
        from redactio_sidecar.ipc import EngineError
        try:
            if mode == "prepare-probe":
                import runpy
                prepare = runpy.run_path(str(pathlib.Path(__file__).resolve().parents[4] / "scripts" / "prepare-biomedbert.py"))["prepare"]
                prepare(root, "biomedbert")
            else:
                with mutation_lock(root):
                    pass
        except EngineError as error:
            print(error.code, flush=True)
        else:
            print("acquired", flush=True)
    sys.exit()
request = json.loads(sys.stdin.readline())
registry = json.loads((root / "fixture-registry.json").read_text())
descriptor = registry["models"][0]["descriptor"]

def reply(kind, payload, identity=None):
    print(json.dumps({"id": identity or request["id"], "type": kind, "payload": payload}), flush=True)

if request["type"] == "check_model":
    reply("result", descriptor)
    sys.exit()
if mode == "early-exit": sys.exit(2)
if mode == "malformed":
    print("not json", flush=True)
    sys.exit()
if mode == "oversized":
    print("x" * (2 * 1024 * 1024 + 1), flush=True)
    sys.exit()
total = sum(f["size"] for f in descriptor["files"])
job = {"job_id": request["id"], "model_name": descriptor["name"], "stage": "downloading", "downloaded_bytes": 0, "total_bytes": total, "error": None}
if mode == "wrong-model": job["model_name"] = "wrong-model"
if mode == "bad-total": job["total_bytes"] += 1
reply("progress", job, "00000000-0000-0000-0000-000000000000" if mode == "wrong-id" else ("{" + request["id"] + "}" if mode == "alternate-id" else None))
job.update(stage="validating", downloaded_bytes=total)
reply("progress", job)
if mode not in ("no-publish", "block"):
    (root / "manifest.json").write_text(json.dumps(registry))
if mode in ("block", "publish-block"):
    (root / "started").write_text(str(__import__("os").getpid()))
    time.sleep(60)
if mode == "wrong-result": descriptor["version"] = "f" * 40
reply("result", descriptor)
