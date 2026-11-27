function refresh({ settings, today }) {
    const days = Math.round((Date.parse(settings.target) - Date.parse(today)) / 86400000);
    const card = days > 0 ? `距离${settings.title}还有 ${days} 天`
        : days === 0 ? `今天是${settings.title}日` : `${settings.title}已过 ${-days} 天`;
    return { card, storage: { lastDays: days } };
}
