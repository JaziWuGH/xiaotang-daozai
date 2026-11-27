import argparse
import math
import re
import statistics
from pathlib import Path


FRAME = re.compile(
    r"^间隔 (\d+) ms｜页面 (\d+)｜展开 (true|false)｜.*"
    r"｜过渡 (true|false) 交互起点 (true|false) 兼容 (true|false)$"
)


def read_frames(path):
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        match = FRAME.fullmatch(line)
        if match:
            interval, page, expanded, transition, interactive, compatible = match.groups()
            yield int(interval), page, expanded, transition, interactive, compatible


def check(animation, idle):
    active_frames = list(read_frames(animation))
    intervals = []
    entry_intervals = []
    for previous, current in zip(active_frames, active_frames[1:]):
        if previous[1:4] == current[1:4] and current[2:4] == ("true", "true"):
            if current[4:] != ("true", "true"):
                raise ValueError("动画未按兼容交互帧交付")
            if previous[4:] == ("true", "true"):
                intervals.append(current[0])
            else:
                entry_intervals.append(current[0])
    if len(intervals) < 45:
        raise ValueError(f"连续动画帧样本不足 45 个，实际 {len(intervals)} 个")
    median = statistics.median(intervals)
    p95 = sorted(intervals)[math.ceil(len(intervals) * 0.95) - 1]
    print(f"交互帧间隔 {len(intervals)} 个，平均 {statistics.mean(intervals):.1f} ms，P50 {median:g} ms，P95 {p95} ms")
    if entry_intervals:
        mixed = sorted(intervals + entry_intervals)
        mixed_p95 = mixed[math.ceil(len(mixed) * 0.95) - 1]
        startup = "、".join(map(str, entry_intervals))
        print(f"含启动的 {len(mixed)} 个更新间隔，平均 {statistics.mean(mixed):.1f} ms，P95 {mixed_p95} ms")
        print(f"各次启动的更新入口差值 {startup} ms")

    idle_animation = sum(
        transition == "true" or (interactive == "true" and interval <= 34)
        for interval, _, _, transition, interactive, _ in read_frames(idle)
    )
    print(f"静止采样有 {idle_animation} 个动画或高频交互读回帧")
    if not 12 <= median <= 21 or p95 > 25 or idle_animation:
        raise ValueError("帧间隔或静止读回不符合验收要求")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="检查兼容渲染的画布动画和静止帧记录")
    parser.add_argument("animation", type=Path, help="数字翻动页的动画帧日志")
    parser.add_argument("idle", type=Path, help="静止画布稳定后另采集的 30 秒日志")
    args = parser.parse_args()
    try:
        check(args.animation, args.idle)
    except ValueError as error:
        parser.exit(1, f"{error}\n")
