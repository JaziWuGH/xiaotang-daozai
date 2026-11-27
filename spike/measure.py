"""从仓库根目录运行。依赖 Python 3、psutil；结果写入指定的新目录。"""
import argparse
from datetime import date, timedelta
import ctypes
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import time

import psutil

parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
args = parser.parse_args()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
repo = Path.cwd()
fixtures = out / "plugins"
shutil.copytree(repo / "spike/plugins", fixtures)
env = dict(os.environ, DAOZAI_SPIKE_DATA=str(out / "data"), DAOZAI_SPIKE_PLUGINS=str(fixtures))
result = {"platform": platform.platform(), "cpu_logical": psutil.cpu_count(), "runs": {}}
target = repo / "target"
base_size = (target / "spike-base.exe").stat().st_size
result["base_bytes"] = base_size

def compile_wasm(source, destination):
    subprocess.run(["rustc", "+stable-x86_64-pc-windows-msvc", "--target", "wasm32-unknown-unknown",
                    "--crate-type", "cdylib", "-C", "opt-level=s", "-C", "panic=abort",
                    "-C", "link-arg=-zstack-size=65536", "-C", "strip=symbols",
                    str(source), "-o", str(destination)], check=True)

def measure_without_window(prefix, report, environment):
    """单独测量每秒取数、调用插件和存储比较的进程开销。"""
    with subprocess.Popen([*prefix, "steady", str(report)], env=environment) as child:
        process = psutil.Process(child.pid)
        time.sleep(2)
        cpu_start = sum(process.cpu_times()[:2])
        started = time.perf_counter()
        rss = []
        for _ in range(30):
            rss.append(process.memory_info().rss)
            time.sleep(1)
        cpu_ms = (sum(process.cpu_times()[:2]) - cpu_start) * 1000
        duration = time.perf_counter() - started
        if child.wait(timeout=12) != 0 or json.loads(report.read_text("utf8"))["refresh"] != 35:
            raise RuntimeError("无窗口刷新未完成")
        return {"cpu_ms": cpu_ms, "sample_s": duration, "rss_bytes_median": statistics.median(rss)}

for kind in ["js", "wasm"]:
    exe = target / ("spike-" + kind + ".exe")
    prefix = [str(exe), "--sdk-spike", kind]
    def run(mode, name, *extra):
        file = out / (kind + "-" + name + ".json")
        subprocess.run([*prefix, mode, str(file), *extra], env=env, check=True, timeout=40)
        return json.loads(file.read_text(encoding="utf8"))
    metrics = {"exe_bytes": exe.stat().st_size, "delta_bytes": exe.stat().st_size - base_size}
    result["runs"][kind] = metrics
    rows = run("load", "load")
    metrics["load_ms"] = {}
    for n in [1,10]:
        warm = sorted(row["ms"] for row in rows if row["n"] == n and row["repeat"])
        metrics["load_ms"][n] = {"first": next(row["ms"] for row in rows if row["n"] == n),
                                  "p50": statistics.median(warm), "p95": warm[18]}
    metrics["fault"] = run("fault","fault")
    one = run("once","first","旅行","2026-12-25")
    two = run("once","restart")
    assert one["previous"] is None and two["previous"] == one["days"] and two["title"] == "旅行" and two["target"] == "2026-12-25"
    metrics["restart"] = [one,two]
    image = out / (kind + ".png")
    subprocess.run([*prefix, "screenshot", str(image)], env=env, timeout=40, check=True)
    assert image.is_file(), image
    # 复制件上改一行业务表达式；从保存起计时，包含启动进程及生成第一张可见画面。
    ext = "js" if kind == "js" else "rs"
    source = fixtures / ("countdown." + ext)
    original = source.read_bytes()
    pattern = b"target - today"
    assert original.count(pattern) == 1
    started = time.perf_counter()
    source.write_bytes(original.replace(pattern, pattern+b" + 1"))
    if kind == "wasm":
        compile_wasm(source, fixtures / "countdown.wasm")
    compiled = time.perf_counter()
    changed = out / (kind + "-edited.png")
    subprocess.run([*prefix,"screenshot",str(changed)], env=env, timeout=40, check=True)
    assert changed.is_file()
    metrics["edit_to_frame_ms"] = (time.perf_counter()-started)*1000
    metrics["prepare_ms"] = (compiled-started)*1000
    assert run("once","edited")["days"] == one["days"]+1
    source.write_bytes(original)
    if kind=="wasm":
        shutil.copyfile(repo/"spike/plugins/countdown.wasm",fixtures/"countdown.wasm")

    if kind == "js":
        # 除了捕获抛错，再确认文件模块入口本身不存在。
        check = b"if (fault(5) !== -1 || typeof fetch !== 'undefined') throw Error('unexpected host API');"
        source.write_bytes(original + b"\n" + check)
        run("once", "host-api")
        metrics["unavailable_globals"] = ["require", "std", "os", "fetch"]
        source.write_bytes(original)

    # 用同一宿主把现有数字控件的上下限迁入两种运行环境。
    migration=repo/"spike/migration"/("interval."+ext)
    if kind=="js":
        shutil.copyfile(migration,source)
    else:
        compile_wasm(migration,fixtures/"countdown.wasm")
    limits=[]
    for value in [4,45,181]:
        target_date=(date.today()+timedelta(days=value)).isoformat()
        received=run("once","interval-"+str(value),"提醒间隔",target_date)["days"]
        assert received == max(5,min(180,value))
        limits.append(received)
    metrics["migration_values"]=limits
    shutil.copyfile(repo/"spike/plugins"/("countdown."+("js" if kind=="js" else "wasm")),fixtures/("countdown."+("js" if kind=="js" else "wasm")))

    proc = subprocess.Popen([*prefix,"ui"], env=env)
    try:
        p = psutil.Process(proc.pid)
        time.sleep(3)
        start = time.perf_counter()
        cpu = sum(p.cpu_times()[:2])
        samples = []
        # 进程时间分辨率可能大于整段 CPU 增量，另取 Windows 硬件周期计数。
        kernel = ctypes.WinDLL("kernel32",use_last_error=True)
        def cycles():
            value=ctypes.c_ulonglong()
            if not kernel.QueryProcessCycleTime(ctypes.c_void_p(proc._handle),ctypes.byref(value)):
                raise ctypes.WinError(ctypes.get_last_error())
            return value.value
        first_cycles=cycles()
        for _ in range(30):
            samples.append(p.memory_info().rss)
            time.sleep(1)
        metrics["cycles"]=cycles()-first_cycles
        metrics["cpu_ms"]=(sum(p.cpu_times()[:2])-cpu)*1000
        metrics["sample_s"]=time.perf_counter()-start
        metrics["rss_bytes_median"]=statistics.median(samples)
        metrics["rss_bytes_peak"]=max(samples)
        # 只关闭本次采样创建的窗口，通过进程 ID 匹配。
        user = ctypes.WinDLL("user32", use_last_error=True)
        callback_type = ctypes.WINFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)
        def close_owned_window(hwnd, unused):
            pid = ctypes.c_ulong()
            user.GetWindowThreadProcessId(ctypes.c_void_p(hwnd), ctypes.byref(pid))
            if pid.value == proc.pid and user.IsWindowVisible(ctypes.c_void_p(hwnd)):
                user.PostMessageW(ctypes.c_void_p(hwnd), 0x0010, 0, 0)
            return 1
        callback = callback_type(close_owned_window)
        user.EnumWindows(callback, 0)
        assert proc.wait(timeout=12)==0
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    metrics["without_window"] = measure_without_window(prefix, out / (kind + "-steady.json"), env)
    (out/"results.json").write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding="utf8")
    print(kind, "完成", flush=True)
