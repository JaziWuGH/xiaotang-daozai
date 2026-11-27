// 宿主将日期差作为合成的分钟输入，验证迁移后的区间和持久化。
function refresh(target, today) {
    const previous = lastDays();
    saveDays(Math.max(5, Math.min(180, target - today)));
    return previous;
}
