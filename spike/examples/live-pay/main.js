const DAY = 86400;

function parseTime(text) {
    if (!/^(?:[01][0-9]|2[0-3]):[0-5][0-9]$/.test(text)) return null;
    const [hour, minute] = text.split(":").map(Number);
    return (hour * 60 + minute) * 60;
}

function weekday(date) {
    return date.getUTCDay() || 7;
}

function refresh({settings, now, tz}) {
    const salary = Number(settings.salary);
    const start = parseTime(settings.start);
    const end = parseTime(settings.end);
    const workdays = settings.days ? settings.days.split(",").map(Number) : [];
    if (!Number.isFinite(salary) || salary < 0) {
        return {status:{state:"idle", label:"今日工资", summary:"请输入有效的月薪"}};
    }
    if (start === null || end === null || start >= end) {
        return {status:{state:"idle", label:"今日工资", summary:"下班时间须晚于当天上班时间"}};
    }
    if (!workdays.length || !workdays.every(day => Number.isInteger(day) && day >= 1 && day <= 7)) {
        return {status:{state:"idle", label:"今日工资", summary:"请先选择工作日"}};
    }

    // 用宿主的本机偏移换算墙钟时间，日期运算统一使用 UTC 方法。
    const seconds = Math.floor(now / 1000);
    const offset = tz.offset_minutes * 60;
    const local = new Date((seconds + offset) * 1000);
    const midnight = Math.floor((seconds + offset) / DAY) * DAY - offset;
    const startAt = midnight + start;
    const endAt = midnight + end;
    const day = weekday(local);
    const afterStart = workdays.includes(day) && seconds >= startAt;
    const working = afterStart && seconds < endAt;

    const date = new Date(local);
    date.setUTCDate(1);
    let daysInMonth = 0;
    while (date.getUTCMonth() === local.getUTCMonth()) {
        if (workdays.includes(weekday(date))) daysInMonth++;
        date.setUTCDate(date.getUTCDate() + 1);
    }

    let countdown;
    if (working) {
        countdown = {label:"距下班", target_at:endAt, format:"HH:mm:ss", done_label:"已下班"};
    } else {
        let daysUntil = 0;
        while (!workdays.includes((day + daysUntil - 1) % 7 + 1) ||
               startAt + daysUntil * DAY <= seconds) {
            daysUntil++;
        }
        countdown = {
            label:"距上班", target_at:startAt + daysUntil * DAY, format:"d天 HH:mm", done_label:"已上班"
        };
    }

    return {
        status:{
            state:working ? "running" : "idle",
            label:"今日工资",
            counter:{
                start_value:0,
                start_at:afterStart ? startAt : seconds,
                per_second:afterStart ? salary / daysInMonth / (endAt - startAt) : 0,
                until:afterStart ? endAt : undefined,
                decimals:2,
                prefix:"¥",
                sensitive:true
            },
            countdown
        },
        pinned:{text:countdown.label, value:{countdown}, icon:"clock"},
        next_refresh_at:working ? endAt : Math.min(countdown.target_at, midnight + DAY)
    };
}
