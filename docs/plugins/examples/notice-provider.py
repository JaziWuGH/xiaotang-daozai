"""演示岛仔短时通知、常驻状态和用户点击回执。"""

import argparse
import json
import socket
import time
from pathlib import Path
from uuid import uuid4


def send(credentials_path, method, **fields):
    credentials = json.loads(credentials_path.read_text(encoding="utf-8"))
    if credentials["version"] != 1 or credentials["host"] != "127.0.0.1":
        raise RuntimeError("需要 v1 本机通知凭据")
    request = {key: credentials[key] for key in ("version", "source", "token")}
    request.update(method=method, **fields)
    frame = (json.dumps(request, ensure_ascii=False) + "\n").encode("utf-8")
    with socket.create_connection(("127.0.0.1", credentials["port"]), timeout=1) as conn:
        conn.sendall(frame)
        with conn.makefile("rb") as response:
            return json.loads(response.readline(128 * 1024))


def publish(credentials_path, method, **fields):
    for delay in (0.3, 0.6, 1.2):
        response = send(credentials_path, method, **fields)
        if response.get("ok"):
            return
        if response.get("error") != "busy":
            raise RuntimeError(f"岛仔返回 {response.get('error', '未知错误')}")
        time.sleep(delay)
    raise RuntimeError("岛仔队列忙，请稍后再试")


def run(credentials_path):
    run_id = uuid4().hex
    status_id = f"demo-status-{run_id}"
    session = f"demo-receipt-{run_id}"
    action = {"session": session, "action_label": "打印回执"}
    publish(credentials_path, "notify", id=f"demo-notice-{run_id}",
            title="接口示例已连接", markdown="进度卡将更新 20 秒。", **action)
    print("到岛上查看接口示例，点击「打印回执」。")
    try:
        for step in range(20):
            publish(credentials_path, "status_update", id=status_id,
                    title=f"模拟进度 · {(step + 1) * 5}%", ttl_seconds=4, **action)
            deadline = time.monotonic() + 1
            while time.monotonic() < deadline:
                pending = send(credentials_path, "poll_open")
                if not pending.get("ok"):
                    raise RuntimeError(f"岛仔返回 {pending.get('error', '未知错误')}")
                request = pending.get("open")
                if request is not None:
                    handled = request["session"] == session
                    if handled:
                        print("已收到岛上的点击。", flush=True)
                    send(credentials_path, "ack_open",
                         request_id=request["request_id"], opened=handled)
                time.sleep(0.25)
    finally:
        try:
            send(credentials_path, "status_clear", id=status_id)
        except (OSError, ValueError, KeyError):
            pass  # 失联时，岛仔会在状态有效期结束后自动撤卡。


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("credentials", type=Path, help="岛仔设置中复制的凭据文件路径")
    args = parser.parse_args()
    try:
        run(args.credentials)
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        parser.exit(1, f"{error}\n")
