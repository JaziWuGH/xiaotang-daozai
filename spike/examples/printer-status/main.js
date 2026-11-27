function refresh({ storage }) {
    const step = ((storage?.step ?? -1) + 1) % 5;
    const progress = [18, 34, 53, 71, 90][step];
    const temperatures = [218, 219, 221, 220, 222, 220, 219, 221, 220];
    return {
        status: {
            state: "running",
            label: "正在打印 · 桌面理线夹",
            progress_pct: progress,
            metrics: [
                { label: "喷嘴", value: temperatures[step + 4] + " °C" },
                { label: "热床", value: "60 °C" }
            ],
            series: temperatures.slice(step, step + 5),
            items: [
                { label: "当前任务", detail: "桌面理线夹", state: "running" },
                { label: "下一任务", detail: "收纳盒", state: "paused" }
            ],
            note: "折线记录喷嘴温度。\n读数每分钟模拟刷新。",
            tags: ["PLA", "0.20 mm", "模拟数据"],
            alert: "耗材余量偏低"
        },
        storage: { step }
    };
}
