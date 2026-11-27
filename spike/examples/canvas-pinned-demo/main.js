function refresh(input) {
    const now = Math.floor(input.now / 1000);
    return {
        canvas: {
            height: 128,
            nodes: [
                {text: "岛上的一小格", at: [0, 26], size: 23, weight: 700, font: "站酷快乐体"},
                {rect: [0, 46, 387, 78], r: 12, fill: "$card"},
                {icon: "coin", rect: [14, 63, 22, 22], stroke: "$accent", width: 1.5},
                {text: "金额与进度", at: [42, 80], size: 13, fill: "$text"},
                {icon: "calendar", rect: [194, 63, 22, 22], stroke: "$accent", width: 1.5},
                {text: "时钟与便签", at: [222, 80], size: 13, fill: "$text"},
                {text: "常驻内容每 8 秒轮换", at: [15, 109], size: 12, fill: "$muted"}
            ]
        },
        pinned: {
            canvas: [
                {
                    width: 109,
                    binds: {
                        amount: {counter: {start_value: 30.01, per_second: 0.07, start_at: now, decimals: 2}},
                        progress: {progress: [now, now + 260]}
                    },
                    nodes: [
                        {circle: [10, 11, 7], stroke: "$track", width: 2},
                        {arc: {c: [10, 11], r: 7, start: -90, sweep: {bind: "progress", map: [40, 360]}}, stroke: "$accent", width: 2},
                        {id: "amount", text: "¥{amount}", at: [22, 15], size: 12, weight: 650, roll: true}
                    ]
                },
                {
                    width: 109,
                    binds: {time: {now: "HH:mm"}},
                    nodes: [
                        {rect: [1, 5, 3, 12], r: 1, fill: "#4C96FF"},
                        {group: [
                            {id: "time", text: "{time}  便签", at: [10, 15], size: 11, max_width: 96,
                                loop: {kind: "flow", period_ms: 4800}}
                        ], clip: [10, 0, 99, 22]}
                    ]
                }
            ]
        }
    };
}
