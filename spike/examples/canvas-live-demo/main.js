function page(input, paused) {
    const second = Math.floor(input.now / 1000);
    const data = input.storage || {};
    const start = data.start ?? second;
    const until = paused ? (data.until ?? second) : undefined;
    const active = !paused;
    const purple = "$accent";
    const tween = {ms: 280, ease: "spring"};
    const hand = (name, length, width) => ({
        group: [{line: [0, 0, 0, -length], width, stroke: purple, cap: "round"}],
        translate: [94, 160], rotate: {bind: name}
    });
    const rows = Array.from({length: 10}, (_, i) => ([
        {rect: [208, 69 + i * 25, 168, 21], r: 6, fill: "$card"},
        {text: `记录 ${i + 1}`, at: [221, 84 + i * 25], size: 11, fill: "$muted"}
    ])).flat();
    return {
        storage: {paused, start, until},
        canvas: {
            height: 220,
            binds: {
                amount: {counter: {start_value: 38.4, per_second: 0.37, start_at: start, decimals: 2, until}},
                time: {now: "HH:mm:ss", zone: "Asia/Shanghai"},
                progress: {progress: [start, start + 300]},
                hour: {clock_angle: {zone: "Asia/Shanghai", hand: "hour"}},
                minute: {clock_angle: {zone: "Asia/Shanghai", hand: "minute"}}
            },
            nodes: [
                {rect: [0, 0, 193, 219], r: 14, fill: "$card"},
                {text: "累计金额", at: [16, 30], size: 12, fill: "$muted"},
                {id: "amount", text: "¥{amount}", at: [16, 68], size: 31, weight: 800, roll: true},
                {text: "{time}", at: [96, 103], size: 11, align: "center", fill: "$muted"},
                {circle: [94, 160, 40], stroke: "$track", width: 4},
                {arc: {c: [94, 160], r: 40, start: -90, sweep: {bind: "progress", map: [0, 360]}}, stroke: purple, width: 4, cap: "round"},
                hand("hour", 15, 4), hand("minute", 25, 2),
                {circle: [94, 160, 3], fill: purple},
                {id: "switch", rect: [208, 7, 168, 34], r: 9, fill: active ? purple : "$card",
                    hit: {action: active ? "pause" : "resume"}, hover: {lift: 2, opacity: 0.85}, animate: tween},
                {text: active ? "暂停计数" : "继续计数", at: [292, 29], size: 12, weight: 700, align: "center",
                    fill: active ? "#160D29" : "$text"},
                {text: "活动记录", at: [208, 58], size: 11, fill: "$muted"},
                {id: "records", group: rows, clip: [206, 67, 173, 150], scroll: {axis: "y", content: 252}}
            ]
        }
    };
}

function refresh(input) {
    return page(input, !!(input.storage && input.storage.paused));
}

function action(input) {
    if (input.action.id === "resume" && input.storage && input.storage.until) {
        const now = Math.floor(input.now / 1000);
        input = {...input, storage: {start: input.storage.start + now - input.storage.until}};
    }
    return page(input, input.action.id === "pause");
}
