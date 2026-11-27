import argparse
import hashlib
import io
import json
import os
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageOps


WALL_TEMPLATE = Path(__file__).resolve().parents[1] / "docs/images/readme/supporters.webp"
WALL_WIDTH, HEADER_HEIGHT, FOOTER_HEIGHT = 1400, 352, 126
COLUMNS, COLUMN_GAP, ROW_HEIGHT = 11, 108.73, 128
TIME_FORMAT = "%Y-%m-%d %H:%M:%S"


@dataclass
class Supporter:
    amount: int
    last_payment: datetime
    avatar: Path


def read_ledger(path: Path, output: Path) -> list[Supporter]:
    if path.is_relative_to(output):
        raise ValueError("台账必须放在发布目录之外")
    ledger = json.loads(path.read_text(encoding="utf-8-sig"))
    payments = ledger.get("payments") if isinstance(ledger, dict) else None
    if not isinstance(payments, list) or not payments:
        raise ValueError("台账的 payments 必须是非空列表")

    supporters: dict[str, Supporter] = {}
    seen = set()
    for index, row in enumerate(payments, 1):
        try:
            channel, time, payer, cents, avatar = (
                row[key] for key in ("channel", "time", "payer", "amount_cents", "avatar")
            )
            payment_time = datetime.strptime(time, TIME_FORMAT)
            if channel not in ("wechat", "alipay") or payment_time.strftime(TIME_FORMAT) != time:
                raise ValueError("支付渠道或时间格式无效")
            if not isinstance(payer, str) or not payer or payer.strip() != payer:
                raise ValueError("付款人标识无效")
            if type(cents) is not int or cents < 1:
                raise ValueError("金额必须为正整数分")
            relative = Path(avatar)
            if relative.anchor:
                raise ValueError("头像必须使用台账目录下的相对路径")
            avatar_path = (path.parent / relative).resolve()
            if not avatar_path.is_relative_to(path.parent) or avatar_path.is_relative_to(output):
                raise ValueError("头像必须位于台账目录内、发布目录外")
        except (KeyError, TypeError, ValueError, OSError) as error:
            raise ValueError(f"第 {index} 笔记录无效：{error}") from error

        transaction = (channel, payment_time, payer, cents)
        if transaction in seen:
            continue
        seen.add(transaction)
        supporter = supporters.get(payer)
        if supporter is None:
            supporters[payer] = Supporter(cents, payment_time, avatar_path)
        else:
            supporter.amount += cents
            if payment_time > supporter.last_payment:
                supporter.last_payment = payment_time
                supporter.avatar = avatar_path
    return [supporters[payer] for payer in sorted(supporters)]


def circle(size: int) -> Image.Image:
    large = size * 3
    mask = Image.new("L", (large, large))
    ImageDraw.Draw(mask).ellipse((0, 0, large - 1, large - 1), fill=255)
    return mask.resize((size, size), Image.Resampling.LANCZOS)


def portrait(person: Supporter, mask: Image.Image) -> Image.Image:
    with Image.open(person.avatar) as source:
        image = ImageOps.fit(ImageOps.exif_transpose(source).convert("RGBA"), mask.size, Image.Resampling.LANCZOS)
    image.putalpha(ImageChops.multiply(image.getchannel("A"), mask))
    return image


def render_recent(supporters: list[Supporter]) -> Image.Image:
    recent = sorted(supporters, key=lambda person: person.last_payment, reverse=True)[:6]
    size, gap = 84, 10
    strip = Image.new("RGBA", (len(recent) * (size + gap) - gap, size))
    mask = circle(size)
    for index, person in enumerate(recent):
        strip.alpha_composite(portrait(person, mask), (index * (size + gap), 0))
    return strip


def render_wall(supporters: list[Supporter]) -> Image.Image:
    ranked = sorted(supporters, key=lambda person: (person.amount, person.last_payment), reverse=True)
    rows = (len(ranked) + COLUMNS - 1) // COLUMNS
    body_height = rows * ROW_HEIGHT
    with Image.open(WALL_TEMPLATE) as template:
        width, height = template.size
        if width != WALL_WIDTH or height < HEADER_HEIGHT + FOOTER_HEIGHT + ROW_HEIGHT:
            raise ValueError("感谢墙模板尺寸已变化")
        wall_height = HEADER_HEIGHT + body_height + FOOTER_HEIGHT
        if width * wall_height > 8_000_000:
            raise ValueError("感谢墙超过客户端像素上限")
        wall = Image.new("RGB", (width, wall_height))
        wall.paste(template.crop((0, 0, width, HEADER_HEIGHT)), (0, 0))
        body = template.crop((0, HEADER_HEIGHT, width, HEADER_HEIGHT + 1))
        wall.paste(body.resize((width, body_height), Image.Resampling.NEAREST), (0, HEADER_HEIGHT))
        wall.paste(template.crop((0, height - FOOTER_HEIGHT, width, height)), (0, HEADER_HEIGHT + body_height))

    size, margin = 98, 11
    ring = circle(size)
    face = circle(size - 10)
    shadow_mask = Image.new("L", (size + 2 * margin, size + 2 * margin))
    shadow_mask.paste(ring.point(lambda value: value * 22 // 255), (margin, margin + 4))
    shadow_mask = shadow_mask.filter(ImageFilter.GaussianBlur(5))
    for index, person in enumerate(ranked):
        row, column = divmod(index, COLUMNS)
        row_count = min(COLUMNS, len(ranked) - row * COLUMNS)
        x = round(width / 2 + (column - (row_count - 1) / 2) * COLUMN_GAP - size / 2)
        y = HEADER_HEIGHT + row * ROW_HEIGHT + 62 - size // 2
        wall.paste((45, 28, 67), (x - margin, y - margin, x + size + margin, y + size + margin), shadow_mask)
        wall.paste((255, 255, 255), (x, y, x + size, y + size), ring)
        image = portrait(person, face)
        wall.paste(image, (x + 5, y + 5), image.getchannel("A"))
    return wall


def webp(image: Image.Image, limit: int) -> bytes:
    for quality in (94, 85, 72):
        buffer = io.BytesIO()
        image.save(buffer, "WEBP", quality=quality, method=6)
        if buffer.tell() <= limit:
            return buffer.getvalue()
    raise ValueError("生成的 WebP 超过客户端大小上限")


def atomic_write(path: Path, data: bytes) -> None:
    stream = tempfile.NamedTemporaryFile(dir=path.parent, delete=False)
    temp = Path(stream.name)
    try:
        with stream:
            stream.write(data)
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="从本地打赏台账生成岛仔感谢墙，只导出显示所需的清单和头像图。",
        epilog="台账为 JSON，payments 中每笔包含 channel（wechat 或 alipay）、time（年-月-日 时:分:秒）、"
        "payer（同一支持者使用同一标识）、amount_cents（金额分）、avatar（相对台账目录的头像路径）。",
    )
    parser.add_argument("ledger", type=Path, help="本地 JSON 台账")
    parser.add_argument("output", type=Path, help="存放公开清单和图片的目录")
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        supporters = read_ledger(args.ledger.resolve(), output)
        recent = webp(render_recent(supporters), 256 * 1024)
        wall = webp(render_wall(supporters), 2 * 1024 * 1024)
        count = len(supporters)
        version = hashlib.sha256(count.to_bytes(8, "little") + recent + wall).hexdigest()[:24]
        manifest = {"version": version, "count": count, "recent": f"recent-{version}.webp", "wall": f"wall-{version}.webp"}
        output.mkdir(parents=True, exist_ok=True)
        atomic_write(output / manifest["recent"], recent)
        atomic_write(output / manifest["wall"], wall)
        # 最后替换清单，客户端读取旧清单时仍能访问旧图片。
        atomic_write(output / "supporters.json", (json.dumps(manifest, indent=2) + "\n").encode("utf-8"))
    except (ValueError, OSError, Image.DecompressionBombError) as error:
        parser.exit(1, f"生成失败：{error}\n")
    print(f"已生成 {count} 位支持者的感谢墙，版本 {version}")


if __name__ == "__main__":
    main()
