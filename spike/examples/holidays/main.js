function refresh({ today, tz, data }) {
    const { year, holidays, makeup } = data["holidays-2026.json"];
    const holiday = holidays.find(([, from, to]) => from <= today && today <= to);
    const next = holidays.find(([, from]) => from > today);
    const daysUntil = (date) => (Date.parse(date) - Date.parse(today)) / 86400000;
    const working = makeup.includes(today);
    const upcoming = next ? `距${next[0]}还有 ${daysUntil(next[1])} 天` : `${year} 年假期已结束`;
    const card = holiday ? `${holiday[0]}假期`
        : working ? `今天调休上班 · ${upcoming}`
        : today.slice(0, 4) === String(year) ? upcoming : `暂无 ${today.slice(0, 4)} 年节假日数据`;
    return {
        card,
        marker: holiday ? `${holiday[0]}放假` : working ? "调休上班" : "",
        next_refresh_at: Date.parse(today) / 1000 + 86400 - tz.offset_minutes * 60,
    };
}
