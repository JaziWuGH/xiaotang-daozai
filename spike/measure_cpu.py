"""在 Windows 桌面会话从仓库根目录运行，参数为双引擎程序和全新结果目录。"""
import argparse
import ctypes
from ctypes import wintypes
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

import psutil


def read_counters(path):
    for _ in range(30):
        if path.exists():
            counts = path.read_text().split()
            if len(counts) == 3:
                return [int(x) for x in counts]
        time.sleep(0.03)
    raise RuntimeError("验证窗口未报告完整计数")


def prepare_records(directory, count):
    directory.mkdir(parents=True)
    sample = Path("spike/examples/payday")
    manifest = json.loads((sample / "plugin.json").read_text("utf8"))
    # 在相同的倒计时上记录实际调用数，区分成功刷新和仅被列为已安装。
    source = (sample / "main.js").read_text("utf8").replace("settings, today", "settings, today, storage").replace(
        "lastDays: days", "lastDays: days, calls: (storage?.calls || 0) + 1")
    assert "calls:" in source
    for number in range(count):
        manifest["id"] = f"bench.p{number}"
        record = dict(origin=str((sample / "plugin.json").resolve()),
                      package=dict(manifest=manifest, source=source), enabled=True,
                      values={f["key"]: f["default"] for f in manifest["settings"]}, storage=None)
        (directory / f"local-{manifest['id']}.json").write_text(json.dumps(record), "utf8")


def window_state(pid):
    # 记录焦点、最小化和鼠标所在状态，便于识别采样期间的桌面干扰。
    user = ctypes.windll.user32
    user.GetForegroundWindow.restype = wintypes.HWND
    foreground = wintypes.DWORD()
    user.GetWindowThreadProcessId(user.GetForegroundWindow(), ctypes.byref(foreground))
    point = wintypes.POINT()
    user.GetCursorPos(ctypes.byref(point))
    state = {"foreground": foreground.value == pid, "hovered": False, "visible": False}
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    @callback_type
    def visit(hwnd, unused):
        owner = wintypes.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user.IsWindowVisible(hwnd) and not user.IsIconic(hwnd):
            rect = wintypes.RECT()
            user.GetWindowRect(hwnd, ctypes.byref(rect))
            if rect.right > rect.left and rect.bottom > rect.top:
                state["visible"] = True
                state["hovered"] |= rect.left <= point.x < rect.right and rect.top <= point.y < rect.bottom
        return True

    user.EnumWindows(visit, 0)
    return state


def measure(exe, directory, kind, interval, installed, legacy_vsync):
    directory.mkdir()
    data = directory / "data"
    counters = directory / "frames.txt"
    env = dict(os.environ, DAOZAI_DATA_DIR=str(data), DAOZAI_SPIKE_DATA=str(directory / "preview"),
               APPDATA=str(directory / "roaming"), DAOZAI_SPIKE_COUNTERS=str(counters))
    env.pop("DAOZAI_SPIKE_REDRAW_MS", None)
    env.pop("DAOZAI_SPIKE_LEGACY_VSYNC", None)
    if legacy_vsync:
        env["DAOZAI_SPIKE_LEGACY_VSYNC"] = "1"
    if interval:
        env["DAOZAI_SPIKE_REDRAW_MS"] = str(interval)
    preview = installed is None
    if preview:
        args = ["--sdk-spike", kind, "ui"]
    else:
        prepare_records(data / "local-plugins", installed)
        args = ["--sdk-profile-island"]
    proc = subprocess.Popen([str(exe), *args], env=env)
    try:
        process = psutil.Process(proc.pid)
        if preview:
            end = time.monotonic() + 25
            while not counters.exists() and proc.poll() is None and time.monotonic() < end:
                time.sleep(0.2)
        time.sleep(5)
        before = read_counters(counters) if preview else None
        def calls():
            return [(json.loads(p.read_text("utf8"))["storage"] or {}).get("calls", 0)
                    for p in sorted((data / "local-plugins").glob("local-*.json"))]
        if installed:
            ready = time.monotonic() + 45
            while not all(calls()) and time.monotonic() < ready:
                time.sleep(0.2)
        calls_before = [] if preview else calls()
        rss = []
        desktop_cpu = []
        psutil.cpu_percent()
        window_samples = dict(foreground=0, hovered=0, visible=0)
        cpu = sum(process.cpu_times()[:2])
        start = time.perf_counter()
        for _ in range(20 if preview else 70):
            time.sleep(1)
            rss.append(process.memory_info().rss)
            desktop_cpu.append(psutil.cpu_percent())
            for state, active in window_state(proc.pid).items():
                window_samples[state] += active
        cpu_ms = (sum(process.cpu_times()[:2]) - cpu) * 1000
        seconds = time.perf_counter() - start
        row = dict(mode=kind, legacy_vsync=legacy_vsync, redraw_ms=interval, installed=installed, elapsed_s=round(seconds, 3),
                   cpu_ms=round(cpu_ms, 3), single_core_pct=round(cpu_ms / (seconds * 10), 3),
                   rss_median_mib=round(statistics.median(rss) / 1048576, 2),
                   rss_peak_mib=round(max(rss) / 1048576, 2), window_samples=window_samples,
                   desktop_cpu_median_pct=statistics.median(desktop_cpu), samples=len(rss))
        if preview:
            frames, calls_delta, work = [b-a for a, b in zip(before, read_counters(counters))]
            row.update(frames=frames, calls=calls_delta, callback_us=work)
            assert frames and calls_delta
        else:
            row.update(calls_before=calls_before, calls_after=calls())
            assert len(calls_before) == installed and all(
                b >= 1 and a > b for b, a in zip(calls_before, row["calls_after"])), row
        return row
    finally:
        if proc.poll() is None:
            proc.terminate()
        proc.wait(timeout=10)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("exe", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
exe = args.exe.resolve()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
report = dict(started=datetime.now().astimezone().isoformat(), platform=platform.platform(),
              sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), runs=[])
groups = [
    ("js", None, None, True), ("wasm", None, None, True),
    ("wasm", None, None, True), ("js", None, None, True),
    ("js", None, None, False), ("wasm", None, None, False),
    ("js", 16, None, False), ("wasm", 16, None, False),
    ("island", None, 0, False), ("island", None, 10, False),
    ("island", None, 10, False), ("island", None, 0, False),
]
for i, group in enumerate(groups):
    row = measure(exe, out / f"run-{i}", *group)
    report["runs"].append(row)
    (out / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", "utf8")
    print(row, flush=True)
